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
mod encode;
mod input;
#[cfg(target_os = "macos")]
mod mac;
mod render;
mod schedule;
mod ui;

pub use render::Snapshot;

use crate::{Application, Waker, scene::Scene};
pub(crate) use input::Input;
use input::key_name;
use render::{Gpu, Pending};
use schedule::{Schedule, Wait};
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::ModifiersState;
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
use winit::window::{Window, WindowId};

/// What a window shows: a deka [`Application`] or the portfolio world.
pub(crate) trait Content: 'static {
    /// Build the scene for a `width` x `height` logical-pixel window at `scale`.
    fn frame(&mut self, width: f32, height: f32, scale: f32) -> &Scene;
    /// Handle one input; `true` when the window must redraw.
    fn input(&mut self, input: Input) -> bool;
    /// Background work between input events; `true` when the window must redraw.
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
            background: 0xffffff,
            frames: None,
            on_frame: None,
        }
    }
}

/// Wakes the event loop from any thread; the loop then runs [`Content::turn`].
struct Wake;

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
    let mut shell = Shell {
        proxy: event_loop.create_proxy(),
        content,
        options,
        gpu: GpuState::Starting(gpu),
        window: None,
        surface: None,
        schedule: Schedule::new(),
        cursor: PhysicalPosition::new(0., 0.),
        modifiers: ModifiersState::empty(),
        focused: true,
        presented: 0,
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
    proxy: EventLoopProxy<Wake>,
    content: C,
    options: Options,
    gpu: GpuState,
    window: Option<Arc<Window>>,
    surface: Option<Surface>,
    schedule: Schedule,
    cursor: PhysicalPosition<f64>,
    modifiers: ModifiersState,
    focused: bool,
    presented: u64,
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
        let attributes = Window::default_attributes()
            .with_title(self.options.title.clone())
            .with_inner_size(LogicalSize::new(self.options.width, self.options.height))
            // Shown once its background is the application's colour.
            .with_visible(!cfg!(target_os = "macos"));
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .map_err(|e| format!("cannot create a window: {e}"))?,
        );
        #[cfg(target_os = "macos")]
        {
            mac::set_background(&window, self.options.background);
            window.set_visible(true);
        }
        let mut gpu = match std::mem::replace(&mut self.gpu, GpuState::Failed) {
            GpuState::Starting(pending) => pending.join()?,
            GpuState::Ready(gpu) => *gpu,
            GpuState::Failed => return Err("no GPU".into()),
        };
        let surface = gpu
            .instance
            .create_surface(window.clone())
            .map_err(|e| format!("cannot draw into the window: {e}"))?;
        let format = gpu.adopt(&surface)?;
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
        self.surface = Some(Surface { surface, config });
        self.gpu = GpuState::Ready(Box::new(gpu));
        self.window = Some(window);
        let proxy = self.proxy.clone();
        self.content.set_waker(Waker::new(move || {
            let _ = proxy.send_event(Wake);
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
        if (size.width, size.height) != (surface.config.width, surface.config.height) {
            surface.config.width = size.width;
            surface.config.height = size.height;
            surface.surface.configure(&gpu.device, &surface.config);
        }
        self.schedule.invalidate();
    }

    fn input(&mut self, input: Input) {
        if self.content.input(input) {
            self.schedule.invalidate();
        }
    }

    /// Draw and present one frame. Never panics: a failed or panicking frame
    /// is logged and skipped.
    fn frame(&mut self, event_loop: &ActiveEventLoop) {
        if self.schedule.occluded() {
            return;
        }
        let drawn = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.draw()));
        match drawn {
            Ok(Ok(Drawn::Presented { animating })) => {
                self.schedule.presented(animating);
                self.presented += 1;
                let frame = Frame {
                    index: self.presented,
                    presented_at: Instant::now(),
                };
                self.content.presented(self.focused);
                if self.presented == 1 {
                    #[cfg(target_os = "macos")]
                    mac::install_menu();
                }
                if let Some(on_frame) = self.options.on_frame.as_mut() {
                    on_frame(frame);
                }
                if self.options.frames.is_some_and(|n| self.presented >= n) {
                    event_loop.exit();
                }
            }
            Ok(Ok(Drawn::Occluded)) => self.schedule.surface_occluded(Instant::now()),
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
        let scale = window.scale_factor();
        let scene = self.content.frame(
            (f64::from(size.width) / scale) as f32,
            (f64::from(size.height) / scale) as f32,
            scale as f32,
        );
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
        let view = texture.texture.create_view(&Default::default());
        gpu.draw(
            scene,
            scale,
            &view,
            surface.config.width,
            surface.config.height,
        )?;
        window.pre_present_notify();
        gpu.queue.present(texture);
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
        if let Err(error) = self.open(event_loop) {
            self.failed = Some(error);
            event_loop.exit();
            return;
        }
        // Frame one now, not on the first RedrawRequested.
        self.frame(event_loop);
    }

    fn user_event(&mut self, _: &ActiveEventLoop, _: Wake) {
        if self.content.turn() {
            self.schedule.invalidate();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => self.resize(size),
            WindowEvent::ScaleFactorChanged { .. } => self.schedule.invalidate(),
            WindowEvent::Occluded(occluded) => self.schedule.set_occluded(occluded),
            WindowEvent::RedrawRequested => self.frame(event_loop),
            WindowEvent::Focused(focused) => {
                self.focused = focused;
                if !focused {
                    self.modifiers = ModifiersState::empty();
                }
                self.input(Input::Focus(focused));
            }
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::CursorMoved { position, .. } => self.cursor = position,
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                let scale = self.scale();
                self.input(Input::Press {
                    x: (self.cursor.x / scale) as f32,
                    y: (self.cursor.y / scale) as f32,
                });
            }
            WindowEvent::KeyboardInput {
                event,
                is_synthetic: false,
                ..
            } => {
                if let Some(name) = key_name(&event.key_without_modifiers()) {
                    self.input(Input::Key {
                        name,
                        down: event.state == ElementState::Pressed,
                        repeat: event.repeat,
                        shift: self.modifiers.shift_key(),
                    });
                }
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
        self.window = None;
    }
}

#[cfg(test)]
mod tests;
