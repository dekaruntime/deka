//! An app scope with independently owned window mounts (APS 74).
use crate::{Scope, UiApp, View};
use deka_native_ui::{
    Waker,
    window::{NativeWindow, Options, WindowController, WindowRequest, WindowToken},
};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::{Rc, Weak},
    sync::Arc,
};

type Factory = Box<dyn FnOnce(WindowHandle) -> View>;
enum Request {
    Open {
        handle: WindowHandle,
        options: Options,
        factory: Factory,
    },
    Close(WindowToken),
    Service(deka_native_ui::window::ServiceRequest),
}
#[derive(Clone, Copy, PartialEq)]
enum State {
    Pending,
    Open,
    Closing,
}
struct Manager {
    alive: Cell<bool>,
    scope: crate::reactive::WeakScope,
    requests: RefCell<Vec<Request>>,
    states: RefCell<BTreeMap<WindowToken, State>>,
    windows: RefCell<BTreeMap<WindowToken, std::sync::Weak<NativeWindow>>>,
    waker: RefCell<Option<Waker>>,
}
impl Manager {
    fn wake(&self) {
        if let Some(waker) = self.waker.borrow().as_ref() {
            waker.wake();
        }
    }
}
/// A capability for one window. Closing is queued until the event batch ends.
#[derive(Clone)]
pub struct WindowHandle {
    id: WindowToken,
    manager: Weak<Manager>,
}
impl WindowHandle {
    fn service(
        &self,
        make: impl FnOnce(crate::reactive::WeakScope) -> deka_native_ui::window::ServiceRequest,
    ) -> Result<(), AppClosed> {
        let manager = self
            .manager
            .upgrade()
            .filter(|manager| manager.alive.get())
            .ok_or(AppClosed)?;
        if !manager
            .states
            .borrow()
            .get(&self.id)
            .is_some_and(|state| matches!(state, State::Pending | State::Open))
        {
            return Err(AppClosed);
        }
        manager
            .requests
            .borrow_mut()
            .push(Request::Service(make(manager.scope.clone())));
        manager.wake();
        Ok(())
    }
    pub fn open_file(
        &self,
        options: deka_native_ui::window::FileDialogOptions,
        complete: impl FnOnce(deka_native_ui::window::DialogResult) + 'static,
    ) -> Result<(), AppClosed> {
        self.file_dialog(deka_native_ui::window::DialogKind::Open, options, complete)
    }
    pub fn save_file(
        &self,
        options: deka_native_ui::window::FileDialogOptions,
        complete: impl FnOnce(deka_native_ui::window::DialogResult) + 'static,
    ) -> Result<(), AppClosed> {
        self.file_dialog(deka_native_ui::window::DialogKind::Save, options, complete)
    }
    fn file_dialog(
        &self,
        kind: deka_native_ui::window::DialogKind,
        options: deka_native_ui::window::FileDialogOptions,
        complete: impl FnOnce(deka_native_ui::window::DialogResult) + 'static,
    ) -> Result<(), AppClosed> {
        self.service(|scope| {
            let captured = crate::native_services::Captured::new(scope);
            deka_native_ui::window::ServiceRequest::FileDialog {
                window: self.id,
                kind,
                options,
                complete: Box::new(move |result| {
                    captured.run(|| complete(result));
                }),
            }
        })
    }
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub fn context_menu(&self, menu: crate::Menu, x: f32, y: f32) -> Result<(), AppClosed> {
        self.service(|_| deka_native_ui::window::ServiceRequest::ContextMenu {
            window: self.id,
            menu: menu.0,
            position: (x, y),
        })
    }
    pub fn id(&self) -> WindowToken {
        self.id
    }
    /// Request close. Returns false after close/shutdown or a prior request.
    pub fn close(&self) -> bool {
        let Some(manager) = self.manager.upgrade().filter(|m| m.alive.get()) else {
            return false;
        };
        let mut states = manager.states.borrow_mut();
        let Some(state) = states.get_mut(&self.id) else {
            return false;
        };
        if matches!(state, State::Closing) {
            return false;
        }
        *state = State::Closing;
        drop(states);
        manager.requests.borrow_mut().push(Request::Close(self.id));
        manager.wake();
        true
    }
}
/// Clone into Rust handlers to open additional windows.
#[derive(Clone)]
pub struct WindowManager(Rc<Manager>);
impl WindowManager {
    /// Queue a window factory. The factory receives its own close capability.
    /// Signals allocated before window factories are shared app state; signals
    /// allocated inside a factory or handler belong to that window mount.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub fn app_menu(&self, menu: crate::Menu) -> Result<(), AppClosed> {
        if !self.0.alive.get() {
            return Err(AppClosed);
        }
        self.0.requests.borrow_mut().push(Request::Service(
            deka_native_ui::window::ServiceRequest::AppMenu(menu.0),
        ));
        self.0.wake();
        Ok(())
    }
    pub fn open(
        &self,
        options: Options,
        factory: impl FnOnce(WindowHandle) -> View + 'static,
    ) -> Result<WindowHandle, AppClosed> {
        if !self.0.alive.get() {
            return Err(AppClosed);
        }
        let handle = WindowHandle {
            id: WindowToken::allocate(),
            manager: Rc::downgrade(&self.0),
        };
        self.0.states.borrow_mut().insert(handle.id, State::Pending);
        self.0.requests.borrow_mut().push(Request::Open {
            handle: handle.clone(),
            options,
            factory: Box::new(factory),
        });
        self.0.wake();
        Ok(handle)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppClosed;
impl std::fmt::Display for AppClosed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("desktop app has closed")
    }
}
impl std::error::Error for AppClosed {}
/// Owns shared state and any number of window trees. Pass to `launch`.
pub struct DesktopApp {
    scope: Scope,
    windows: WindowManager,
}
impl DesktopApp {
    pub fn new(build: impl FnOnce(WindowManager)) -> Self {
        let scope = Scope::new();
        let windows = WindowManager(Rc::new(Manager {
            alive: Cell::new(true),
            scope: scope.downgrade(),
            requests: RefCell::new(vec![]),
            states: RefCell::new(BTreeMap::new()),
            windows: RefCell::new(BTreeMap::new()),
            waker: RefCell::new(None),
        }));
        scope.run(|| build(windows.clone()));
        Self { scope, windows }
    }
}
impl WindowController for DesktopApp {
    type App = UiApp;
    fn requests(&mut self) -> Vec<WindowRequest<UiApp>> {
        let requests = std::mem::take(&mut *self.windows.0.requests.borrow_mut());
        requests
            .into_iter()
            .map(|request| match request {
                Request::Open {
                    handle,
                    options,
                    factory,
                } => {
                    let id = handle.id;
                    let app = UiApp::new_in_scope(&self.scope, move || factory(handle));
                    if self.windows.0.states.borrow().get(&id) == Some(&State::Closing) {
                        drop(app);
                        WindowRequest::Close(id)
                    } else {
                        WindowRequest::Open { id, app, options }
                    }
                }
                Request::Close(id) => WindowRequest::Close(id),
                Request::Service(request) => WindowRequest::Service(request),
            })
            .collect()
    }
    fn has_requests(&self) -> bool {
        !self.windows.0.requests.borrow().is_empty()
    }
    fn set_waker(&mut self, waker: Waker) {
        self.windows.0.waker.replace(Some(waker));
    }
    fn opened(&mut self, id: WindowToken, window: Option<Arc<NativeWindow>>) {
        if let Some(window) = window {
            self.windows
                .0
                .windows
                .borrow_mut()
                .insert(id, Arc::downgrade(&window));
        }
        if self.windows.0.states.borrow().get(&id) == Some(&State::Pending) {
            self.windows.0.states.borrow_mut().insert(id, State::Open);
        }
    }
    fn closed(&mut self, id: WindowToken) {
        self.windows.0.states.borrow_mut().remove(&id);
        self.windows.0.windows.borrow_mut().remove(&id);
    }
}
impl Drop for DesktopApp {
    fn drop(&mut self) {
        self.windows.0.alive.set(false);
        self.windows.0.waker.borrow_mut().take();
        self.windows.0.requests.borrow_mut().clear();
    }
}
