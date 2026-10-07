//! The desktop window: winit (window, input, event loop), wgpu, and deka's
//! vello_gpu fork drawing deka's scene. Layout, text, animation and hit testing
//! are deka's own (`scene`, `text`, `animation`); this module only shows them.
//!
//! Start-up: the GPU device and renderer are created on a thread started before
//! the event loop, the bundled font on another; the window appears in the
//! application's colour, frame one is drawn as soon as the window exists, and
//! the menu bar is built after it. Frames are drawn only when something
//! changed or a scene animates, and never while the window is hidden.
//!
//! Nothing here panics on a frame: errors are logged and the frame is skipped.
mod accessibility;
mod editor;
mod encode;
mod input;
#[cfg(target_os = "macos")]
mod mac;
mod render;
mod schedule;
pub mod trace;
mod ui;

pub use render::Snapshot;

use crate::{Application, Waker, scene::Scene};
pub(crate) use input::Input;
pub use input::{EventLayer, KeyInput};
use render::{Gpu, Pending};
use schedule::{Schedule, Wait};
use std::sync::Arc;
use std::time::{Duration, Instant};
pub use ui::{DesktopSession, TextClipboard};
/// Public adapter event payloads for platform integration and event tests.
pub mod accesskit_events {
    pub use accesskit::{Action, ActionData, ActionRequest, NodeId, Role, TreeId};
    pub use accesskit_winit::WindowEvent;
}
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};

use winit::window::{Window, WindowId};

/// What a window shows: a deka [`Application`] or the portfolio world.
pub(crate) trait Content: 'static {
    /// Build the scene for a `width` x `height` logical-pixel window at `scale`.
    fn frame(&mut self, width: f32, height: f32, scale: f32) -> &Scene;
    /// Handle one input; `true` when the window must redraw.
    fn input(&mut self, input: Input) -> bool;
    /// Background work between input events; `true` when the window must redraw.
    fn accessibility(&mut self, _scale: f32) -> accesskit::TreeUpdate {
        accessibility::empty_tree()
    }
    fn accessibility_event(&mut self, _event: &accesskit_winit::WindowEvent) -> bool {
        false
    }
    fn ime_area(&self) -> Option<crate::scene::Rect> {
        None
    }
    fn turn(&mut self) -> bool {
        false
    }
    /// How often [`Content::turn`] runs without being woken (`None`: only on wake).
    fn turn_interval(&self) -> Option<Duration> {
        None
    }
    /// Receives the window's [`Waker`] once the window exists.
    fn set_waker(&mut self, _waker: Waker) {}
    /// A frame reached the screen; `active` is whether the window has focus.
    fn presented(&mut self, _active: bool) {}
    #[cfg(test)]
    fn scene(&self) -> &Scene;
}

/// One presented frame, for callers that measure or limit frames.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    /// 1 for the first presented frame.
    pub index: u64,
    /// When `present` returned.
    pub presented_at: Instant,
}

/// How a window is opened.
pub struct Options {
    pub title: String,
    /// Inner size in logical pixels.
    pub width: f64,
    pub height: f64,
    /// Top-left of the window's frame in logical global screen coordinates
    /// (origin at the top left of the main display), on any display. `None`
    /// centres it on the main display.
    pub position: Option<(f64, f64)>,
    /// The window's colour before frame one is drawn.
    pub background: u32,
    /// Close the window after this many presented frames.
    pub frames: Option<u64>,
    /// Called after every presented frame.
    pub on_frame: Option<Box<dyn FnMut(Frame)>>,
}

impl Options {
    pub fn new(title: impl Into<String>, width: f64, height: f64) -> Self {
        Self {
            title: title.into(),
            width,
            height,
            position: None,
            background: 0xffffff,
            frames: None,
            on_frame: None,
        }
    }
}

/// Wakes the event loop from any thread; the loop then runs [`Content::turn`].
enum Wake {
    Work,
    Accessibility(accesskit_winit::Event),
}
impl From<accesskit_winit::Event> for Wake {
    fn from(event: accesskit_winit::Event) -> Self {
        Self::Accessibility(event)
    }
}

/// Open `app` in a window and run until it closes. `--exercise N` runs the
/// application's handlers without a window and prints its text instead.
pub fn run<A: Application>(app: A) {
    let args: Vec<_> = std::env::args().collect();
    if let Some(index) = args.iter().position(|a| a == "--exercise") {
        let clicks = args
            .get(index + 1)
            .and_then(|s| s.parse().ok())
            .expect("--exercise requires a nonnegative click count");
        println!("{}", crate::exercise(app, clicks));
        return;
    }
    let reduced_motion = args.iter().any(|arg| arg == "--reduced-motion");
    let options = Options::new("Deka native experiment", 560., 300.);
    run_with(app, options, reduced_motion);
}

/// [`run`] with explicit window options (tests and measurements).
pub fn run_with<A: Application>(app: A, options: Options, reduced_motion: bool) {
    let gpu = Gpu::start();
    show(ui::UiContent::new(app, reduced_motion), options, gpu);
}

/// Start creating the GPU device and renderer on their own thread.
#[cfg(feature = "world-audio")]
pub(crate) fn start_gpu() -> Pending {
    Gpu::start()
}

/// Render `scene` offscreen at `scale` with the window's renderer.
pub fn snapshot(scene: &Scene, scale: f32) -> Result<Snapshot, String> {
    Gpu::new()?.snapshot(scene, f64::from(scale))
}

/// Run the event loop for `content`. Exits the process with status 1 when no
/// window can be opened (no display, no GPU).
pub(crate) fn show<C: Content>(content: C, options: Options, gpu: Pending) {
    trace::mark("show");
    crate::text::warm_on_thread();
    let mut builder = EventLoop::<Wake>::with_user_event();
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::EventLoopBuilderExtMacOS;
        // Built after frame one instead (mac::install_menu).
        builder.with_default_menu(false);
    }
    let event_loop = match builder.build() {
        Ok(event_loop) => event_loop,
        Err(e) => {
            eprintln!("deka: cannot open a window: {e}");
            std::process::exit(1);
        }
    };
    trace::mark("event loop built");
    let mut shell = Shell {
        proxy: event_loop.create_proxy(),
        content,
        options,
        gpu: GpuState::Starting(gpu),
        window: None,
        surface: None,
        schedule: Schedule::new(),
        events: EventLayer::default(),
        adapter: None,
        accessible_tree: Arc::new(std::sync::Mutex::new(accessibility::empty_tree())),
        focused: true,
        presented: 0,
        drawn_scale: None,
        shown_with_frame: false,
        menu_installed: false,
        failed: None,
    };
    if let Err(e) = event_loop.run_app(&mut shell) {
        eprintln!("deka: event loop: {e}");
        std::process::exit(1);
    }
    if let Some(error) = shell.failed {
        eprintln!("deka: {error}");
        std::process::exit(1);
    }
}

enum GpuState {
    Starting(Pending),
    Ready(Box<Gpu>),
    Failed,
}

struct Surface {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
}

struct Shell<C: Content> {
    events: EventLayer,
    adapter: Option<accesskit_winit::Adapter>,
    accessible_tree: Arc<std::sync::Mutex<accesskit::TreeUpdate>>,
    proxy: EventLoopProxy<Wake>,
    content: C,
    options: Options,
    gpu: GpuState,
    window: Option<Arc<Window>>,
    surface: Option<Surface>,
    schedule: Schedule,
    focused: bool,
    presented: u64,
    /// The scale factor the last frame was drawn at.
    drawn_scale: Option<f64>,
    /// Frame one went into the window before it was shown (macOS), so the
    /// window has its content when it first becomes visible.
    shown_with_frame: bool,
    menu_installed: bool,
    /// Why the window could not be shown (reported after the loop ends).
    failed: Option<String>,
}

enum Drawn {
    Presented {
        animating: bool,
    },
    /// The window is not visible; wait for it.
    Occluded,
    /// Nothing to draw into (zero size, surface being reconfigured): draw
    /// again when something changes.
    Skipped,
    /// The surface was out of date or timed out; it is reconfigured, try again.
    Retry,
}

impl<C: Content> Shell<C> {
    fn open(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        let mut attributes = Window::default_attributes()
            .with_title(self.options.title.clone())
            .with_inner_size(LogicalSize::new(self.options.width, self.options.height))
            // Shown once its background is the application's colour.
            .with_visible(false);
        if let Some((x, y)) = self.options.position {
            attributes = attributes.with_position(LogicalPosition::new(x, y));
        }
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .map_err(|e| format!("cannot create a window: {e}"))?,
        );
        self.content.frame(
            self.options.width as f32,
            self.options.height as f32,
            window.scale_factor() as f32,
        );
        *self
            .accessible_tree
            .lock()
            .unwrap_or_else(|e| e.into_inner()) =
            self.content.accessibility(window.scale_factor() as f32);
        if let Some((_, root)) = self
            .accessible_tree
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .nodes
            .iter_mut()
            .find(|(id, _)| *id == accessibility::ROOT)
        {
            root.set_label(self.options.title.clone());
        }
        self.adapter = Some(accesskit_winit::Adapter::with_mixed_handlers(
            event_loop,
            &window,
            accessibility::Activation(self.accessible_tree.clone()),
            self.proxy.clone(),
        ));
        trace::mark("window created");
        // On macOS the window stays hidden until frame one is in it (`first_frame`).
        #[cfg(target_os = "macos")]
        {
            // AppKit clamps a new window's content rect to the main screen, so a
            // position on another display only holds once the window exists;
            // it is still hidden here.
            if let Some((x, y)) = self.options.position {
                window.set_outer_position(LogicalPosition::new(x, y));
            }
            mac::set_background(&window, self.options.background);
        }
        let mut gpu = match std::mem::replace(&mut self.gpu, GpuState::Failed) {
            GpuState::Starting(pending) => {
                let gpu = pending.join()?;
                trace::mark("gpu joined");
                gpu
            }
            GpuState::Ready(gpu) => *gpu,
            GpuState::Failed => return Err("no GPU".into()),
        };
        let surface = gpu
            .instance
            .create_surface(window.clone())
            .map_err(|e| format!("cannot draw into the window: {e}"))?;
        trace::mark("surface created");
        let format = gpu.adopt(&surface)?;
        trace::mark("surface adopted");
        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&gpu.device, &config);
        trace::mark("surface configured");
        self.surface = Some(Surface { surface, config });
        self.gpu = GpuState::Ready(Box::new(gpu));
        self.window = Some(window);
        let proxy = self.proxy.clone();
        self.content.set_waker(Waker::new(move || {
            let _ = proxy.send_event(Wake::Work);
        }));
        self.schedule
            .plan_turn(Instant::now(), self.content.turn_interval());
        Ok(())
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        let (Some(surface), GpuState::Ready(gpu)) = (&mut self.surface, &self.gpu) else {
            return;
        };
        if size.width == 0 || size.height == 0 {
            return;
        }
        // Showing a window reports its size again: nothing to redraw then.
        if (size.width, size.height) != (surface.config.width, surface.config.height) {
            surface.config.width = size.width;
            surface.config.height = size.height;
            surface.surface.configure(&gpu.device, &surface.config);
            self.schedule.invalidate();
        }
    }

    /// The window's scale factor is `scale` (winit also reports it when a
    /// window is first shown): redraw only if frames were drawn at another.
    fn rescale(&mut self, scale: f64) {
        if self.drawn_scale != Some(scale) {
            self.schedule.invalidate();
        }
    }

    fn input(&mut self, input: Input) {
        if self.content.input(input) {
            self.schedule.invalidate();
        }
        self.sync_ime();
    }
    fn sync_ime(&self) {
        if let Some(window) = &self.window {
            let area = self.content.ime_area();
            window.set_ime_allowed(area.is_some());
            if let Some(r) = area {
                window.set_ime_cursor_area(
                    LogicalPosition::new(r.x, r.y),
                    LogicalSize::new(r.width, r.height),
                );
            }
        }
    }

    /// Frame one, and the window shown with it.
    ///
    /// On macOS, wgpu hands out no surface texture until AppKit reports the
    /// window visible, tens of milliseconds after it is ordered front, so the
    /// window would appear in its background colour and get its content
    /// later. Instead frame one is drawn straight into the hidden window's
    /// layer and the window is shown with it (GPUI did the same). If that
    /// fails, the window is shown and frame one takes the usual path.
    fn first_frame(&mut self, event_loop: &ActiveEventLoop) {
        #[cfg(target_os = "macos")]
        {
            let drawn =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.draw_hidden()));
            if let Some(window) = &self.window {
                window.set_visible(true);
            }
            trace::mark("window shown");
            match drawn {
                Ok(Ok(animating)) => {
                    self.shown_with_frame = true;
                    self.presented_frame(event_loop, animating);
                    return;
                }
                Ok(Err(error)) => eprintln!("deka: frame one waits for the window: {error}"),
                Err(_) => eprintln!("deka: frame one waits for the window after a panic"),
            }
        }
        #[cfg(not(target_os = "macos"))]
        if let Some(window) = &self.window {
            window.set_visible(true);
        }
        self.frame(event_loop);
    }

    /// macOS: frame one into the hidden window's layer (see `first_frame`).
    /// Returns whether the scene animates.
    #[cfg(target_os = "macos")]
    fn draw_hidden(&mut self) -> Result<bool, String> {
        let (Some(window), Some(surface), GpuState::Ready(gpu)) =
            (&self.window, &self.surface, &mut self.gpu)
        else {
            return Err("no window surface".into());
        };
        let size = window.inner_size();
        if (size.width, size.height) != (surface.config.width, surface.config.height) {
            return Err("the window changed size".into());
        }
        trace::mark("frame: start");
        let scale = window.scale_factor();
        let scene = self.content.frame(
            (f64::from(size.width) / scale) as f32,
            (f64::from(size.height) / scale) as f32,
            scale as f32,
        );
        trace::mark("frame: scene built");
        gpu.draw_into_layer(&surface.surface, &surface.config, scene, scale)?;
        self.drawn_scale = Some(scale);
        trace::mark("frame: presented");
        Ok(scene.animating)
    }

    /// Bookkeeping after a frame reached the window.
    fn presented_frame(&mut self, event_loop: &ActiveEventLoop, animating: bool) {
        self.schedule.presented(animating);
        self.presented += 1;
        let frame = Frame {
            index: self.presented,
            presented_at: Instant::now(),
        };
        self.content.presented(self.focused);
        self.sync_ime();
        let mut update = self.content.accessibility(self.scale() as f32);
        if let Some((_, root)) = update
            .nodes
            .iter_mut()
            .find(|(id, _)| *id == accessibility::ROOT)
        {
            root.set_label(self.options.title.clone());
        }
        *self
            .accessible_tree
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = update.clone();
        if let Some(adapter) = &mut self.adapter {
            adapter.update_if_active(|| update);
        }
        if let Some(on_frame) = self.options.on_frame.as_mut() {
            on_frame(frame);
        }
        if self.options.frames.is_some_and(|n| self.presented >= n) {
            event_loop.exit();
        }
        // Normally the visibility event installs it first.
        if self.presented >= 2 {
            self.install_menu();
        }
    }

    /// The app menu, once the window is on screen: building it holds the
    /// main thread for tens of milliseconds, which before the window shows
    /// would hold the window back.
    fn install_menu(&mut self) {
        if self.menu_installed {
            return;
        }
        self.menu_installed = true;
        #[cfg(target_os = "macos")]
        mac::install_menu();
        trace::mark("menu installed");
    }

    /// Draw and present one frame. Never panics: a failed or panicking frame
    /// is logged and skipped.
    fn frame(&mut self, event_loop: &ActiveEventLoop) {
        if self.schedule.occluded() {
            return;
        }
        let drawn = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.draw()));
        match drawn {
            Ok(Ok(Drawn::Presented { animating })) => self.presented_frame(event_loop, animating),
            Ok(Ok(Drawn::Occluded)) => {
                if self.presented == 0 {
                    trace::mark("frame: refused while not visible");
                }
                self.schedule.surface_occluded(Instant::now())
            }
            Ok(Ok(Drawn::Skipped)) => self.schedule.skipped(),
            Ok(Ok(Drawn::Retry)) => self.schedule.invalidate(),
            Ok(Err(error)) => {
                eprintln!("deka: frame skipped: {error}");
                self.schedule.skipped();
            }
            Err(_) => {
                eprintln!("deka: frame skipped after a panic");
                self.schedule.skipped();
            }
        }
    }

    fn draw(&mut self) -> Result<Drawn, String> {
        let (Some(window), Some(surface), GpuState::Ready(gpu)) =
            (&self.window, &mut self.surface, &mut self.gpu)
        else {
            return Ok(Drawn::Skipped);
        };
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return Ok(Drawn::Skipped);
        }
        if (size.width, size.height) != (surface.config.width, surface.config.height) {
            surface.config.width = size.width;
            surface.config.height = size.height;
            surface.surface.configure(&gpu.device, &surface.config);
        }
        let first = self.presented == 0;
        if first {
            trace::mark("frame: start");
        }
        let scale = window.scale_factor();
        let scene = self.content.frame(
            (f64::from(size.width) / scale) as f32,
            (f64::from(size.height) / scale) as f32,
            scale as f32,
        );
        if first {
            trace::mark("frame: scene built");
        }
        let texture = match surface.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Occluded => return Ok(Drawn::Occluded),
            wgpu::CurrentSurfaceTexture::Timeout => return Ok(Drawn::Retry),
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                surface.surface.configure(&gpu.device, &surface.config);
                return Ok(Drawn::Retry);
            }
            other => return Err(format!("no surface texture: {other:?}")),
        };
        if first {
            trace::mark("frame: texture acquired");
        }
        let view = texture.texture.create_view(&Default::default());
        gpu.draw(
            scene,
            scale,
            &view,
            surface.config.width,
            surface.config.height,
        )?;
        if first {
            trace::mark("frame: drawn");
        }
        window.pre_present_notify();
        gpu.queue.present(texture);
        self.drawn_scale = Some(scale);
        if first {
            trace::mark("frame: presented");
        }
        Ok(Drawn::Presented {
            animating: scene.animating,
        })
    }

    fn scale(&self) -> f64 {
        self.window.as_ref().map_or(1., |w| w.scale_factor())
    }
}

impl<C: Content> ApplicationHandler<Wake> for Shell<C> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        trace::mark("resumed");
        if let Err(error) = self.open(event_loop) {
            self.failed = Some(error);
            event_loop.exit();
            return;
        }
        // Frame one now, not on the first RedrawRequested.
        self.first_frame(event_loop);
    }

    fn user_event(&mut self, _: &ActiveEventLoop, event: Wake) {
        let changed = match event {
            Wake::Work => self.content.turn(),
            Wake::Accessibility(event) => {
                if self
                    .window
                    .as_ref()
                    .is_none_or(|w| w.id() != event.window_id)
                {
                    return;
                }
                self.content.accessibility_event(&event.window_event)
            }
        };
        if changed {
            self.schedule.invalidate();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        if let (Some(adapter), Some(window)) = (&mut self.adapter, &self.window) {
            adapter.process_event(window, &event);
        }
        if let Some(input) = self.events.translate(&event, self.scale()) {
            self.input(input);
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => self.resize(size),
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => self.rescale(scale_factor),
            WindowEvent::Occluded(true) => self.schedule.set_occluded(true),
            WindowEvent::Occluded(false) => {
                trace::mark("visible");
                if std::mem::take(&mut self.shown_with_frame) {
                    self.schedule.shown_with_frame();
                } else {
                    self.schedule.set_occluded(false);
                }
                if self.presented > 0 {
                    self.install_menu();
                }
            }
            WindowEvent::RedrawRequested => self.frame(event_loop),
            WindowEvent::Focused(focused) => {
                self.focused = focused;
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        self.schedule.take_retry(now);
        if self.schedule.turn_due(now) {
            if self.content.turn() {
                self.schedule.invalidate();
            }
            self.schedule.plan_turn(now, self.content.turn_interval());
        }
        if self.schedule.wants_frame()
            && let Some(window) = &self.window
        {
            window.request_redraw();
        }
        event_loop.set_control_flow(match self.schedule.wait() {
            Wait::Forever => ControlFlow::Wait,
            Wait::Until(deadline) => ControlFlow::WaitUntil(deadline),
        });
    }

    fn exiting(&mut self, _: &ActiveEventLoop) {
        // The surface refers to the window; release it first, then the GPU.
        self.surface = None;
        self.gpu = GpuState::Failed;
        self.adapter = None;
        self.window = None;
    }
}

#[cfg(test)]
mod tests;
