//! Queued native services. Models/callback routing also run without OS windows.
use super::{NativeWindow, WindowToken};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
pub type DialogResult = Result<Option<PathBuf>, String>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogKind {
    Open,
    Save,
}
#[derive(Clone, Default, Debug)]
pub struct FileDialogOptions {
    pub title: Option<String>,
    pub directory: Option<PathBuf>,
    pub file_name: Option<String>,
    pub filters: Vec<(String, Vec<String>)>,
}
impl FileDialogOptions {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }
    pub fn directory(mut self, path: impl AsRef<Path>) -> Self {
        self.directory = Some(path.as_ref().into());
        self
    }
    pub fn file_name(mut self, name: impl Into<String>) -> Self {
        self.file_name = Some(name.into());
        self
    }
    pub fn filter(mut self, name: impl Into<String>, extensions: &[&str]) -> Self {
        self.filters.push((
            name.into(),
            extensions.iter().map(|s| (*s).into()).collect(),
        ));
        self
    }
}
/// Injection seam for the OS dialog, used by the headless event driver.
pub trait FileDialogs: 'static {
    fn choose(
        &mut self,
        kind: DialogKind,
        options: &FileDialogOptions,
        parent: Option<&NativeWindow>,
    ) -> DialogResult;
}
pub struct SystemFileDialogs;
impl FileDialogs for SystemFileDialogs {
    fn choose(
        &mut self,
        kind: DialogKind,
        options: &FileDialogOptions,
        parent: Option<&NativeWindow>,
    ) -> DialogResult {
        if options
            .filters
            .iter()
            .any(|(_, extensions)| extensions.is_empty() || extensions.iter().any(String::is_empty))
        {
            return Err("file filters require nonempty extensions".into());
        }
        let mut dialog = rfd::FileDialog::new();
        if let Some(title) = &options.title {
            dialog = dialog.set_title(title);
        }
        if let Some(directory) = &options.directory {
            dialog = dialog.set_directory(directory);
        }
        if let Some(name) = &options.file_name {
            dialog = dialog.set_file_name(name);
        }
        for (name, extensions) in &options.filters {
            dialog = dialog.add_filter(name, extensions);
        }
        if let Some(parent) = parent {
            dialog = dialog.set_parent(parent);
        }
        Ok(match kind {
            DialogKind::Open => dialog.pick_file(),
            DialogKind::Save => dialog.save_file(),
        })
    }
}
struct NoDialogs;
impl FileDialogs for NoDialogs {
    fn choose(
        &mut self,
        _: DialogKind,
        _: &FileDialogOptions,
        _: Option<&NativeWindow>,
    ) -> DialogResult {
        Err("headless session requires an injected FileDialogs provider".into())
    }
}
pub enum ServiceRequest {
    FileDialog {
        window: WindowToken,
        kind: DialogKind,
        options: FileDialogOptions,
        complete: Box<dyn FnOnce(DialogResult)>,
    },
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    AppMenu(MenuSpec),
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    ContextMenu {
        window: WindowToken,
        menu: MenuSpec,
        position: (f32, f32),
    },
}
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub use menus::{MenuEntry, MenuItemSpec, MenuSpec};
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub use muda::{
    ContextMenu as NativeContextMenu, Menu as NativeMenu, MenuEvent, MenuId,
    accelerator::Accelerator,
};
#[cfg(any(target_os = "macos", target_os = "windows"))]
mod menus {
    use muda::{MenuEvent, MenuId, accelerator::Accelerator};
    use std::{
        cell::RefCell,
        rc::Rc,
        sync::atomic::{AtomicU64, Ordering},
    };
    type Callback = Rc<RefCell<Box<dyn FnMut() -> bool>>>;
    #[derive(Clone)]
    pub struct MenuItemSpec {
        pub id: MenuId,
        pub label: String,
        pub enabled: bool,
        pub accelerator: Option<Accelerator>,
        callback: Callback,
    }
    impl MenuItemSpec {
        pub fn new(label: impl Into<String>, callback: impl FnMut() -> bool + 'static) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(1);
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            assert!(id != u64::MAX, "menu identity exhausted");
            Self {
                id: MenuId::new(format!("deka-menu-{id}")),
                label: label.into(),
                enabled: true,
                accelerator: None,
                callback: Rc::new(RefCell::new(Box::new(callback))),
            }
        }
    }
    #[derive(Clone)]
    pub enum MenuEntry {
        Item(MenuItemSpec),
        Separator,
        Submenu(String, MenuSpec),
        StandardApp(String),
    }
    #[derive(Clone, Default)]
    pub struct MenuSpec {
        pub entries: Vec<MenuEntry>,
    }
    impl MenuSpec {
        pub fn contains(&self, id: &MenuId) -> bool {
            self.entries.iter().any(|entry| match entry {
                MenuEntry::Item(item) => item.id == id,
                MenuEntry::Submenu(_, menu) => menu.contains(id),
                _ => false,
            })
        }
        pub fn event(&self, event: &MenuEvent) -> bool {
            for entry in &self.entries {
                match entry {
                    MenuEntry::Item(item) if item.id == event.id => {
                        return item.enabled && (item.callback.borrow_mut())();
                    }
                    MenuEntry::Submenu(_, menu) if menu.contains(&event.id) => {
                        return menu.event(event);
                    }
                    _ => {}
                }
            }
            false
        }
        /// Build actual OS menu objects on the UI thread. Used by the host and
        /// native smoke checks; no window or popup is created by this operation.
        pub fn native_menu(&self) -> Result<muda::Menu, String> {
            let menu = muda::Menu::new();
            for entry in &self.entries {
                match entry {
                    MenuEntry::Item(item) => menu
                        .append(&muda::MenuItem::with_id(
                            item.id.clone(),
                            &item.label,
                            item.enabled,
                            item.accelerator,
                        ))
                        .map_err(|e| e.to_string())?,
                    MenuEntry::Separator => menu
                        .append(&muda::PredefinedMenuItem::separator())
                        .map_err(|e| e.to_string())?,
                    MenuEntry::Submenu(label, children) => {
                        let submenu = muda::Submenu::new(label, true);
                        for child in children.native_menu()?.items() {
                            submenu
                                .append(match &child {
                                    muda::MenuItemKind::MenuItem(item) => item,
                                    muda::MenuItemKind::Submenu(item) => item,
                                    muda::MenuItemKind::Predefined(item) => item,
                                    muda::MenuItemKind::Check(item) => item,
                                    muda::MenuItemKind::Icon(item) => item,
                                })
                                .map_err(|e| e.to_string())?;
                        }
                        menu.append(&submenu).map_err(|e| e.to_string())?;
                    }
                    MenuEntry::StandardApp(label) => {
                        let app = muda::Submenu::new(label, true);
                        #[cfg(target_os = "macos")]
                        {
                            app.append(&muda::PredefinedMenuItem::about(None, None))
                                .map_err(|e| e.to_string())?;
                            app.append(&muda::PredefinedMenuItem::services(None))
                                .map_err(|e| e.to_string())?;
                            app.append(&muda::PredefinedMenuItem::hide(None))
                                .map_err(|e| e.to_string())?;
                            app.append(&muda::PredefinedMenuItem::hide_others(None))
                                .map_err(|e| e.to_string())?;
                            app.append(&muda::PredefinedMenuItem::show_all(None))
                                .map_err(|e| e.to_string())?;
                            app.append(&muda::PredefinedMenuItem::separator())
                                .map_err(|e| e.to_string())?;
                        }
                        app.append(&muda::PredefinedMenuItem::quit(None))
                            .map_err(|e| e.to_string())?;
                        menu.append(&app).map_err(|e| e.to_string())?;
                    }
                }
            }
            Ok(menu)
        }
    }
}
pub(super) struct Services {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    native: bool,
    dialogs: Box<dyn FileDialogs>,
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    app: Option<MenuSpec>,
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    popup: Vec<(WindowToken, MenuSpec)>,
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    native_menu: Option<muda::Menu>,
    #[cfg(target_os = "windows")]
    pub accelerator: std::rc::Rc<std::cell::Cell<isize>>,
}
impl Services {
    pub fn new(native: bool) -> Self {
        Self {
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            native,
            dialogs: if native {
                Box::new(SystemFileDialogs)
            } else {
                Box::new(NoDialogs)
            },
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            app: None,
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            popup: vec![],
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            native_menu: None,
            #[cfg(target_os = "windows")]
            accelerator: Default::default(),
        }
    }
    pub fn dialogs(&mut self, dialogs: impl FileDialogs) {
        self.dialogs = Box::new(dialogs);
    }
    pub fn has_menu(&self) -> bool {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            self.app.is_some()
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            false
        }
    }
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub fn native_menu(&self) -> Option<muda::Menu> {
        self.native_menu.clone()
    }
    pub fn opened(&self, window: &NativeWindow) -> Result<(), String> {
        #[cfg(target_os = "windows")]
        if let Some(menu) = &self.native_menu {
            Self::attach(menu, window)?;
        }
        #[cfg(not(target_os = "windows"))]
        let _ = window;
        Ok(())
    }
    pub fn closed(&mut self, id: WindowToken, window: Option<&NativeWindow>) {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        self.popup.retain(|(token, _)| *token != id);
        #[cfg(target_os = "windows")]
        if let (Some(menu), Some(window)) = (&self.native_menu, window) {
            use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(handle) = window.window_handle()
                && let RawWindowHandle::Win32(handle) = handle.as_raw()
            {
                // SAFETY: the live winit window owns this HWND until close ends.
                let _ = unsafe { menu.remove_for_hwnd(handle.hwnd.get()) };
            }
        }
        let _ = (id, window);
    }
    #[cfg(target_os = "windows")]
    fn attach(menu: &muda::Menu, window: &NativeWindow) -> Result<(), String> {
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let handle = window.window_handle().map_err(|e| e.to_string())?;
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return Err("not a Win32 window".into());
        };
        // SAFETY: called on winit's thread with the live window's HWND.
        unsafe { menu.init_for_hwnd(handle.hwnd.get()) }.map_err(|e| e.to_string())
    }
    pub fn apply(
        &mut self,
        request: ServiceRequest,
        valid: bool,
        parent: Option<Arc<NativeWindow>>,
        windows: &[Arc<NativeWindow>],
    ) -> Result<(), String> {
        match request {
            ServiceRequest::FileDialog {
                kind,
                options,
                complete,
                ..
            } => {
                complete(if valid {
                    self.dialogs.choose(kind, &options, parent.as_deref())
                } else {
                    Err("window is closed".into())
                });
            }
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            ServiceRequest::AppMenu(spec) => {
                if self.native {
                    let menu = spec.native_menu()?;
                    #[cfg(target_os = "macos")]
                    menu.init_for_nsapp();
                    #[cfg(target_os = "windows")]
                    {
                        // Detach the old menu before attaching its replacement:
                        // muda's Drop otherwise clears the newly installed bar.
                        if let Some(previous) = self.native_menu.take() {
                            use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
                            for window in windows {
                                let handle = window.window_handle().map_err(|e| e.to_string())?;
                                if let RawWindowHandle::Win32(handle) = handle.as_raw() {
                                    // SAFETY: these windows remain alive through replacement.
                                    unsafe { previous.remove_for_hwnd(handle.hwnd.get()) }
                                        .map_err(|e| e.to_string())?;
                                }
                            }
                            self.accelerator.set(0);
                        }
                        for window in windows {
                            Self::attach(&menu, window)?;
                        }
                        self.accelerator.set(menu.haccel());
                    }
                    self.native_menu = Some(menu);
                }
                self.app = Some(spec);
            }
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            ServiceRequest::ContextMenu {
                window,
                menu,
                position,
            } => {
                if !valid {
                    return Err("window is closed".into());
                }
                let selected = if self.native {
                    use muda::ContextMenu;
                    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
                    let parent = parent.ok_or("native context menu requires its window")?;
                    let handle = parent.window_handle().map_err(|e| e.to_string())?;
                    let native = menu.native_menu()?;
                    let position = Some(
                        muda::dpi::LogicalPosition::new(
                            f64::from(position.0),
                            f64::from(position.1),
                        )
                        .into(),
                    );
                    match handle.as_raw() {
                        #[cfg(target_os = "macos")]
                        // SAFETY: the Arc keeps the winit NSView alive while its modal popup tracks.
                        RawWindowHandle::AppKit(handle) => unsafe {
                            native.show_context_menu_for_nsview(handle.ns_view.as_ptr(), position)
                        },
                        #[cfg(target_os = "windows")]
                        // SAFETY: the Arc keeps the winit HWND alive while its modal popup tracks.
                        RawWindowHandle::Win32(handle) => unsafe {
                            native.show_context_menu_for_hwnd(handle.hwnd.get(), position)
                        },
                        _ => return Err("unsupported native menu window handle".into()),
                    }
                } else {
                    true
                };
                if selected {
                    self.popup.push((window, menu));
                }
            }
        }
        let _ = windows;
        Ok(())
    }
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub fn event(&mut self, event: &MenuEvent) -> bool {
        if let Some(index) = self
            .popup
            .iter()
            .position(|(_, menu)| menu.contains(&event.id))
        {
            let (_, menu) = self.popup.remove(index);
            return menu.event(event);
        }
        self.app.as_ref().is_some_and(|menu| menu.event(event))
    }
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub fn dismiss(&mut self, window: WindowToken) {
        self.popup.retain(|(id, _)| *id != window);
    }
}
impl ServiceRequest {
    pub(super) fn window(&self) -> Option<WindowToken> {
        match self {
            Self::FileDialog { window, .. } => Some(*window),
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            Self::ContextMenu { window, .. } => Some(*window),
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            Self::AppMenu(_) => None,
        }
    }
}
impl Drop for Services {
    fn drop(&mut self) {
        #[cfg(target_os = "windows")]
        self.accelerator.set(0);
        #[cfg(target_os = "macos")]
        if let Some(menu) = &self.native_menu {
            menu.remove_for_nsapp();
        }
    }
}
