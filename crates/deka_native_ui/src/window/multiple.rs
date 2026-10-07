//! One winit loop, independently owned window trees and surfaces.
use super::{DesktopSession, Gpu, Options, Shell, Wait, Wake};
use crate::{Application, Waker, scene::Scene};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
    window::{Window, WindowId},
};

/// A logical window capability, independent of OS window handles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct WindowToken(u64);
impl WindowToken {
    pub fn allocate() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        assert!(id != u64::MAX, "window identity exhausted");
        Self(id)
    }
}
pub enum WindowRequest<A> {
    Open {
        id: WindowToken,
        app: A,
        options: Options,
    },
    Close(WindowToken),
    Service(super::ServiceRequest),
}
/// Owns app-wide state and supplies requests after each OS event batch.
pub trait WindowController: 'static {
    type App: Application;
    fn requests(&mut self) -> Vec<WindowRequest<Self::App>>;
    fn has_requests(&self) -> bool;
    fn set_waker(&mut self, _waker: Waker) {}
    fn opened(&mut self, _id: WindowToken, _window: Option<Arc<Window>>) {}
    fn closed(&mut self, _id: WindowToken) {}
    fn failed(&mut self, id: WindowToken, error: String) {
        eprintln!("deka: window {id:?}: {error}");
        self.closed(id);
    }
}
#[derive(Default)]
struct Routes(BTreeMap<WindowId, WindowToken>);
impl Routes {
    fn register(&mut self, window: WindowId, token: WindowToken) {
        self.0.insert(window, token);
    }
    fn target(&self, window: WindowId) -> Option<WindowToken> {
        self.0.get(&window).copied()
    }
    fn remove(&mut self, token: WindowToken) {
        self.0.retain(|_, id| *id != token);
    }
}
trait Store<A> {
    fn open(
        &mut self,
        id: WindowToken,
        app: A,
        options: Options,
    ) -> Result<Option<Arc<Window>>, String>;
    fn close(&mut self, id: WindowToken) -> bool;
    fn service(&mut self, request: super::ServiceRequest) -> Result<(), String>;
}
/// Both native and headless hosts consume the same app commands and lifecycle.
fn apply<C: WindowController>(controller: &mut C, store: &mut impl Store<C::App>) {
    for request in controller.requests() {
        match request {
            WindowRequest::Open { id, app, options } => match store.open(id, app, options) {
                Ok(window) => controller.opened(id, window),
                Err(error) => controller.failed(id, error),
            },
            WindowRequest::Service(request) => {
                if let Err(error) = store.service(request) {
                    eprintln!("deka: native service: {error}");
                }
            }
            WindowRequest::Close(id) => {
                store.close(id);
                controller.closed(id);
            }
        }
    }
}
struct NativeStore<'a, A: Application> {
    windows: &'a mut BTreeMap<WindowToken, Shell<super::ui::UiContent<A>>>,
    routes: &'a mut Routes,
    proxy: EventLoopProxy<Wake>,
    event_loop: &'a ActiveEventLoop,
    reduced: bool,
    failure: &'a mut Option<String>,
    clipboard: super::ui::SharedClipboard,
    services: &'a mut super::services::Services,
}
impl<A: Application> Store<A> for NativeStore<'_, A> {
    fn open(
        &mut self,
        id: WindowToken,
        app: A,
        options: Options,
    ) -> Result<Option<Arc<Window>>, String> {
        if self.windows.contains_key(&id) {
            return Err("window token is already open".into());
        }
        let mut content = super::ui::UiContent::new(app, self.reduced);
        content.clipboard(self.clipboard.clone());
        let mut shell = Shell::new(content, options, self.proxy.clone(), Gpu::start());
        shell.standalone = false;
        shell.menu_installed = self.services.has_menu();
        if let Err(error) = shell.open(self.event_loop) {
            self.failure.replace(error.clone());
            return Err(error);
        }
        let window = shell.window.as_ref().unwrap().clone();
        self.services.opened(&window)?;
        self.routes.register(window.id(), id);
        shell.first_frame(self.event_loop);
        self.windows.insert(id, shell);
        Ok(Some(window))
    }
    fn close(&mut self, id: WindowToken) -> bool {
        let Some(mut shell) = self.windows.remove(&id) else {
            return false;
        };
        self.services.closed(id, shell.window.as_deref());
        shell.exiting(self.event_loop);
        self.routes.remove(id);
        true
    }
    fn service(&mut self, request: super::ServiceRequest) -> Result<(), String> {
        let token = request.window();
        let parent = token
            .and_then(|id| self.windows.get(&id))
            .and_then(|shell| shell.window.clone());
        let valid = token.is_none_or(|id| self.windows.contains_key(&id));
        let windows = self
            .windows
            .values()
            .filter_map(|shell| shell.window.clone())
            .collect::<Vec<_>>();
        self.services.apply(request, valid, parent, &windows)?;
        if self.services.has_menu() {
            for shell in self.windows.values_mut() {
                shell.menu_installed = true;
            }
        }
        Ok(())
    }
}
struct Multiple<C: WindowController> {
    controller: C,
    windows: BTreeMap<WindowToken, Shell<super::ui::UiContent<C::App>>>,
    routes: Routes,
    proxy: EventLoopProxy<Wake>,
    reduced: bool,
    failure: Option<String>,
    clipboard: super::ui::SharedClipboard,
    services: super::services::Services,
}
impl<C: WindowController> Multiple<C> {
    fn requests(&mut self, event_loop: &ActiveEventLoop) {
        let mut store = NativeStore {
            windows: &mut self.windows,
            routes: &mut self.routes,
            proxy: self.proxy.clone(),
            event_loop,
            reduced: self.reduced,
            failure: &mut self.failure,
            clipboard: self.clipboard.clone(),
            services: &mut self.services,
        };
        apply(&mut self.controller, &mut store);
        for id in store
            .windows
            .iter()
            .filter_map(|(id, s)| s.close_requested.then_some(*id))
            .collect::<Vec<_>>()
        {
            store.close(id);
            self.controller.closed(id);
        }
        if self.windows.is_empty() && !self.controller.has_requests() {
            event_loop.exit();
        }
    }
}
impl<C: WindowController> ApplicationHandler<Wake> for Multiple<C> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.requests(event_loop);
    }
    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: Wake) {
        match event {
            Wake::Work => {
                for shell in self.windows.values_mut() {
                    shell.user_event(event_loop, Wake::Work);
                }
            }
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            Wake::Menu(event) => {
                self.services.event(&event);
            }
            Wake::Accessibility(event) => {
                if let Some(shell) = self
                    .routes
                    .target(event.window_id)
                    .and_then(|id| self.windows.get_mut(&id))
                {
                    shell.user_event(event_loop, Wake::Accessibility(event));
                }
            }
        }
        self.requests(event_loop);
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, window: WindowId, event: WindowEvent) {
        let Some(id) = self.routes.target(window) else {
            return;
        };
        if matches!(event, WindowEvent::CloseRequested) {
            let mut store = NativeStore {
                windows: &mut self.windows,
                routes: &mut self.routes,
                proxy: self.proxy.clone(),
                event_loop,
                reduced: self.reduced,
                failure: &mut self.failure,
                clipboard: self.clipboard.clone(),
                services: &mut self.services,
            };
            if store.close(id) {
                self.controller.closed(id);
            }
        } else if let Some(shell) = self.windows.get_mut(&id) {
            shell.window_event(event_loop, window, event);
        }
        self.requests(event_loop);
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.requests(event_loop);
        for shell in self.windows.values_mut() {
            shell.about_to_wait(event_loop);
        }
        let deadline = self
            .windows
            .values()
            .filter_map(|s| match s.schedule.wait() {
                Wait::Until(t) => Some(t),
                Wait::Forever => None,
            })
            .min();
        event_loop.set_control_flow(deadline.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
    }
    fn exiting(&mut self, event_loop: &ActiveEventLoop) {
        for (id, shell) in &mut self.windows {
            self.services.closed(*id, shell.window.as_deref());
            shell.exiting(event_loop);
            self.controller.closed(*id);
        }
        self.windows.clear();
    }
}
pub fn run_windows<C: WindowController>(mut controller: C) {
    crate::text::warm_on_thread();
    let services = super::services::Services::new(true);
    let mut builder = EventLoop::<Wake>::with_user_event();
    #[cfg(target_os = "windows")]
    {
        use winit::platform::windows::EventLoopBuilderExtWindows;
        let table = services.accelerator.clone();
        builder.with_msg_hook(move |message| {
            let accelerator = table.get();
            if accelerator == 0 {
                return false;
            }
            let message = message.cast::<windows_sys::Win32::UI::WindowsAndMessaging::MSG>();
            // SAFETY: winit supplies a live MSG; the table belongs to the retained menu.
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::TranslateAcceleratorW(
                    (*message).hwnd,
                    accelerator as _,
                    message,
                ) != 0
            }
        });
    }
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::EventLoopBuilderExtMacOS;
        builder.with_default_menu(false);
    }
    let event_loop = builder.build().unwrap_or_else(|error| {
        eprintln!("deka: cannot open windows: {error}");
        std::process::exit(1)
    });
    let proxy = event_loop.create_proxy();
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        let menu_proxy = proxy.clone();
        super::MenuEvent::set_event_handler(Some(move |event| {
            let _ = menu_proxy.send_event(Wake::Menu(event));
        }));
    }
    let wake = proxy.clone();
    controller.set_waker(Waker::new(move || {
        let _ = wake.send_event(Wake::Work);
    }));
    let mut app = Multiple {
        controller,
        windows: BTreeMap::new(),
        routes: Routes::default(),
        proxy,
        reduced: std::env::args().any(|arg| arg == "--reduced-motion"),
        failure: None,
        clipboard: Default::default(),
        services,
    };
    if let Err(error) = event_loop.run_app(&mut app) {
        eprintln!("deka: event loop: {error}");
        std::process::exit(1);
    }
    if let Some(error) = app.failure {
        eprintln!("deka: {error}");
        std::process::exit(1);
    }
}
struct HeadlessWindow<A: Application> {
    session: DesktopSession<A>,
    options: Options,
}
struct HeadlessStore<A: Application> {
    windows: BTreeMap<WindowToken, HeadlessWindow<A>>,
    routes: Routes,
    clipboard: super::ui::SharedClipboard,
    services: super::services::Services,
    errors: Vec<String>,
}
impl<A: Application> Store<A> for HeadlessStore<A> {
    fn open(
        &mut self,
        id: WindowToken,
        app: A,
        options: Options,
    ) -> Result<Option<Arc<Window>>, String> {
        if self.windows.contains_key(&id) {
            return Err("window token is already open".into());
        }
        self.routes.register(WindowId::from(id.0), id);
        let mut session = DesktopSession::new(app);
        session.clipboard(self.clipboard.clone());
        self.windows.insert(id, HeadlessWindow { session, options });
        Ok(None)
    }
    fn close(&mut self, id: WindowToken) -> bool {
        self.services.closed(id, None);
        self.routes.remove(id);
        self.windows.remove(&id).is_some()
    }
    fn service(&mut self, request: super::ServiceRequest) -> Result<(), String> {
        let valid = request
            .window()
            .is_none_or(|id| self.windows.contains_key(&id));
        if let Err(error) = self.services.apply(request, valid, None, &[]) {
            self.errors.push(error.clone());
            return Err(error);
        }
        Ok(())
    }
}
/// Production command/lifecycle and winit routing without opening OS windows.
pub struct MultipleDesktopSession<C: WindowController> {
    controller: C,
    store: HeadlessStore<C::App>,
}
impl<C: WindowController> MultipleDesktopSession<C> {
    pub fn new(controller: C) -> Self {
        Self::with_services(controller, false)
    }
    /// CPU window driver with real OS services. Must run on the native UI
    /// thread in a GUI session; queued file dialogs can display actual panels.
    pub fn with_native_services(controller: C) -> Self {
        Self::with_services(controller, true)
    }
    fn with_services(controller: C, native: bool) -> Self {
        let mut app = Self {
            controller,
            store: HeadlessStore {
                windows: BTreeMap::new(),
                routes: Routes::default(),
                clipboard: Default::default(),
                services: super::services::Services::new(native),
                errors: vec![],
            },
        };
        app.requests();
        app
    }
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub fn native_menu(&self) -> Option<super::NativeMenu> {
        self.store.services.native_menu()
    }
    pub fn dialogs(&mut self, dialogs: impl super::FileDialogs) {
        self.store.services.dialogs(dialogs);
    }
    pub fn service_errors(&self) -> &[String] {
        &self.store.errors
    }
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub fn menu_event(&mut self, event: super::MenuEvent) -> bool {
        let changed = self.store.services.event(&event);
        self.requests();
        changed
    }
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub fn dismiss_menu(&mut self, window: WindowToken) {
        self.store.services.dismiss(window);
    }
    pub fn clipboard(&mut self, clipboard: impl super::TextClipboard + 'static) {
        self.store.clipboard.replace(clipboard);
    }
    pub fn requests(&mut self) {
        apply(&mut self.controller, &mut self.store);
    }
    pub fn windows(&self) -> Vec<(WindowToken, WindowId)> {
        self.store
            .routes
            .0
            .iter()
            .map(|(os, id)| (*id, *os))
            .collect()
    }
    pub fn app(&self, id: WindowToken) -> Option<&C::App> {
        self.store
            .windows
            .get(&id)
            .map(|window| window.session.app())
    }
    pub fn frame(&mut self, id: WindowToken, scale: f32) -> Option<&Scene> {
        let window = self.store.windows.get_mut(&id)?;
        Some(window.session.frame(
            window.options.width as f32,
            window.options.height as f32,
            scale,
        ))
    }
    /// Same normalized key payload used after winit's private OS key fields.
    pub fn keyboard(&mut self, window: WindowId, key: super::KeyInput) -> bool {
        let Some(id) = self.store.routes.target(window) else {
            return false;
        };
        let changed = self
            .store
            .windows
            .get_mut(&id)
            .is_some_and(|w| w.session.keyboard(key));
        self.requests();
        changed
    }
    pub fn turn(&mut self, id: WindowToken) -> bool {
        self.store
            .windows
            .get_mut(&id)
            .is_some_and(|window| window.session.turn())
    }
    pub fn event(&mut self, window: WindowId, event: &WindowEvent, scale: f64) -> bool {
        let Some(id) = self.store.routes.target(window) else {
            return false;
        };
        let changed = if matches!(event, WindowEvent::CloseRequested) {
            self.store.close(id);
            self.controller.closed(id);
            true
        } else {
            self.store
                .windows
                .get_mut(&id)
                .is_some_and(|window| window.session.event(event, scale))
        };
        self.requests();
        changed
    }
}
