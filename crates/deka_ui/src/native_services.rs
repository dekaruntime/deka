//! Small Rust menu API; callbacks borrow their originating scope/tree weakly.
use crate::reactive::WeakScope;
use std::rc::Weak;
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
        pub fn new(label: impl Into<String>, mut handler: impl FnMut() + 'static) -> Self {
            let captured = Captured::new(
                Scope::current_weak().expect("create menu callbacks inside an app scope"),
            );
            Self(MenuItemSpec::new(label, move || {
                captured.run(&mut handler).is_some()
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
