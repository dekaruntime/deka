//! Small Rust menu API; callbacks borrow their originating scope/tree weakly.
use crate::reactive::WeakScope;
use std::{
    cell::RefCell,
    rc::{Rc, Weak},
};
#[derive(Clone)]
pub(crate) struct Captured {
    scope: WeakScope,
    tree: Option<Weak<crate::view::Context>>,
}
impl Captured {
    pub fn new(scope: WeakScope) -> Self {
        Self {
            scope,
            tree: crate::retained::active_context(),
        }
    }
    pub fn run<R>(&self, run: impl FnOnce() -> R) -> Option<R> {
        let scope = self.scope.upgrade()?;
        if let Some(tree) = &self.tree {
            let tree = tree.upgrade()?;
            Some(tree.native_event(&scope, run))
        } else {
            Some(crate::retained::with_active(None, || scope.batch(run)))
        }
    }
}
// The service owns the callback cell; the reactive allocation owns only its
// cleanup guard. Either service removal or scope/mount disposal frees captures.
// Clone the inner Rc before calling so teardown during a callback cannot borrow
// the same cell; an in-flight call releases its captures when it returns.
type CallbackCell<T> = RefCell<Option<Rc<RefCell<T>>>>;
pub(crate) struct ScopedCallback<T>(Rc<CallbackCell<T>>);
struct CallbackOwner<T>(Weak<CallbackCell<T>>);
impl<T> Drop for CallbackOwner<T> {
    fn drop(&mut self) {
        if let Some(callback) = self.0.upgrade() {
            callback.borrow_mut().take();
        }
    }
}
impl<T: 'static> ScopedCallback<T> {
    pub(crate) fn new(callback: T) -> Self {
        let cell = Rc::new(RefCell::new(Some(Rc::new(RefCell::new(callback)))));
        crate::signal(CallbackOwner(Rc::downgrade(&cell)));
        Self(cell)
    }
    pub(crate) fn get(&self) -> Option<Rc<RefCell<T>>> {
        self.0.borrow().clone()
    }
}
#[cfg(any(target_os = "macos", target_os = "windows"))]
mod menus {
    use super::*;
    use crate::Scope;
    use deka_native_ui::window::{Accelerator, MenuEntry, MenuId, MenuItemSpec, MenuSpec};
    /// Build within an app/window scope. Each item captures its origin weakly.
    #[derive(Clone, Default)]
    pub struct Menu(pub(crate) MenuSpec);
    impl Menu {
        pub fn new() -> Self {
            Self::default()
        }
        pub fn item(mut self, item: MenuItem) -> Self {
            self.0.entries.push(MenuEntry::Item(item.0));
            self
        }
        pub fn separator(mut self) -> Self {
            self.0.entries.push(MenuEntry::Separator);
            self
        }
        pub fn submenu(mut self, label: impl Into<String>, menu: Menu) -> Self {
            self.0
                .entries
                .push(MenuEntry::Submenu(label.into(), menu.0));
            self
        }
        pub fn standard_app(mut self, label: impl Into<String>) -> Self {
            self.0.entries.push(MenuEntry::StandardApp(label.into()));
            self
        }
        pub fn native_menu(&self) -> Result<deka_native_ui::window::NativeMenu, String> {
            self.0.native_menu()
        }
    }
    #[derive(Clone)]
    pub struct MenuItem(pub(crate) MenuItemSpec);
    impl MenuItem {
        pub fn new(label: impl Into<String>, handler: impl FnMut() + 'static) -> Self {
            let captured = Captured::new(
                Scope::current_weak().expect("create menu callbacks inside an app scope"),
            );
            let handler = ScopedCallback::new(handler);
            Self(MenuItemSpec::new(label, move || {
                let Some(handler) = handler.get() else {
                    return false;
                };
                captured.run(|| (handler.borrow_mut())()).is_some()
            }))
        }
        pub fn id(&self) -> &MenuId {
            &self.0.id
        }
        pub fn enabled(mut self, enabled: bool) -> Self {
            self.0.enabled = enabled;
            self
        }
        pub fn accelerator(mut self, shortcut: &str) -> Result<Self, String> {
            self.0.accelerator = Some(shortcut.parse::<Accelerator>().map_err(|e| e.to_string())?);
            Ok(self)
        }
    }
}
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub use menus::{Menu, MenuItem};

#[cfg(all(test, any(target_os = "macos", target_os = "windows")))]
mod tests {
    use crate::{Menu, MenuItem, Scope};
    use deka_native_ui::window::MenuEvent;
    use std::{cell::Cell, rc::Rc};

    struct CountedDrop(Rc<Cell<usize>>);
    impl Drop for CountedDrop {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    #[test]
    fn scope_drop_frees_menu_callback_while_menu_clones_survive() {
        let drops = Rc::new(Cell::new(0));
        let calls = Rc::new(Cell::new(0));
        let scope = Scope::new();
        let weak_scope = scope.downgrade();
        let capture = Rc::new(CountedDrop(drops.clone()));
        let weak_capture = Rc::downgrade(&capture);
        let output = calls.clone();
        let item = scope.run(|| {
            MenuItem::new("Scoped", move || {
                let _capture = &capture;
                output.set(output.get() + 1);
            })
        });
        let event = MenuEvent {
            id: item.id().clone(),
        };
        let menu = Menu::new().item(item.clone());
        assert!(menu.0.event(&event));
        assert_eq!(calls.get(), 1);
        assert!(weak_capture.upgrade().is_some());
        drop(scope);
        assert!(weak_scope.upgrade().is_none());
        assert_eq!(
            drops.get(),
            1,
            "surviving menus must not own scoped captures"
        );
        assert!(weak_capture.upgrade().is_none());
        assert!(!menu.0.event(&event));
        assert_eq!(calls.get(), 1);
        drop((item, menu));
        assert_eq!(drops.get(), 1);
    }
}
