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
}
/// Owns app-wide state and supplies requests after each OS event batch.
pub trait WindowController: 'static {
    type App: Application;
    fn requests(&mut self) -> Vec<WindowRequest<Self::App>>;
    fn has_requests(&self) -> bool;
    fn set_waker(&mut self, _waker: Waker) {}
    fn opened(&mut self, _id: WindowToken, _window: Option<Arc<Window>>) {}
    fn closed(&mut self, _id: WindowToken) {}
    fn report_error(&self, operation: &str, error: String) {
        eprintln!("deka {operation}: {error}");
    }
    fn failed(&mut self, id: WindowToken, error: String) {
        self.report_error("window", format!("window {id:?}: {error}"));
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
}
/// Both native and headless hosts consume the same app commands and lifecycle.
fn apply<C: WindowController>(controller: &mut C, store: &mut impl Store<C::App>) {
    for request in controller.requests() {
        match request {
            WindowRequest::Open { id, app, options } => match store.open(id, app, options) {
                Ok(window) => controller.opened(id, window),
                Err(error) => controller.failed(id, error),
            },
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
    clipboard: super::ui::SharedClipboard,
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
        if !options.width.is_finite()
            || !options.height.is_finite()
            || options.width <= 0.
            || options.height <= 0.
        {
            return Err("window dimensions must be finite and positive".into());
        }
        let mut content = super::ui::UiContent::new(app, self.reduced);
        content.clipboard(self.clipboard.clone());
        let mut shell = Shell::new(content, options, self.proxy.clone(), Gpu::start());
        shell.standalone = false;
        shell.open(self.event_loop)?;
        let window = shell.window.as_ref().unwrap().clone();
        self.routes.register(window.id(), id);
        shell.first_frame(self.event_loop);
        self.windows.insert(id, shell);
        Ok(Some(window))
    }
    fn close(&mut self, id: WindowToken) -> bool {
        let Some(mut shell) = self.windows.remove(&id) else {
            return false;
        };
        shell.exiting(self.event_loop);
        self.routes.remove(id);
        true
    }
}
struct Multiple<C: WindowController> {
    controller: C,
    windows: BTreeMap<WindowToken, Shell<super::ui::UiContent<C::App>>>,
    routes: Routes,
    proxy: EventLoopProxy<Wake>,
    reduced: bool,
    clipboard: super::ui::SharedClipboard,
}
impl<C: WindowController> Multiple<C> {
    fn requests(&mut self, event_loop: &ActiveEventLoop) {
        let mut store = NativeStore {
            windows: &mut self.windows,
            routes: &mut self.routes,
            proxy: self.proxy.clone(),
            event_loop,
            reduced: self.reduced,
            clipboard: self.clipboard.clone(),
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
                for shell in self
                    .windows
                    .values_mut()
                    .filter(|s| s.content.has_pending_work())
                {
                    shell.user_event(event_loop, Wake::Work);
                }
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
                clipboard: self.clipboard.clone(),
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
            shell.exiting(event_loop);
            self.controller.closed(*id);
        }
        self.windows.clear();
        self.routes.0.clear();
    }
}
pub fn run_windows<C: WindowController>(mut controller: C) {
    crate::text::warm_on_thread();
    let mut builder = EventLoop::<Wake>::with_user_event();
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::EventLoopBuilderExtMacOS;
        builder.with_default_menu(false);
    }
    let event_loop = match builder.build() {
        Ok(event_loop) => event_loop,
        Err(error) => {
            controller.report_error("window", format!("cannot open event loop: {error}"));
            return;
        }
    };
    let proxy = event_loop.create_proxy();
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
        clipboard: Default::default(),
    };
    if let Err(error) = event_loop.run_app(&mut app) {
        app.controller
            .report_error("window", format!("event loop: {error}"));
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
        self.routes.remove(id);
        self.windows.remove(&id).is_some()
    }
}
/// Production command/lifecycle and winit routing without opening OS windows.
pub struct MultipleDesktopSession<C: WindowController> {
    controller: C,
    store: HeadlessStore<C::App>,
}
impl<C: WindowController> MultipleDesktopSession<C> {
    pub fn new(controller: C) -> Self {
        let mut app = Self {
            controller,
            store: HeadlessStore {
                windows: BTreeMap::new(),
                routes: Routes::default(),
                clipboard: Default::default(),
            },
        };
        app.requests();
        app
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

#[cfg(test)]
pub(super) mod native_tests;
