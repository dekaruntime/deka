//! The GPUI side. One window, a fixed number of frames per scene size, prints
//! its numbers and quits on its own.
//!
//! Modes:
//! * `surface`  - zero-copy: vello -> RGBA texture -> NV12 pass into an IOSurface
//!   -> GPUI `paint_surface` (the stock GPUI 0.2.2 surface element path).
//! * `readback` - fallback: vello -> RGBA texture -> CPU readback -> BGRA
//!   `RenderImage` -> GPUI `paint_image` (GPUI re-uploads it to its atlas).
//! * `layer`    - zero-copy, no GPUI involvement: vello -> wgpu surface on a
//!   `CAMetalLayer` added as a sublayer of GPUI's view (Core Animation composites).

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    App, AppContext, Application, Bounds, Context, Corners, IntoElement, ParentElement, Render,
    RenderImage, Styled, Window, WindowBounds, WindowKind, WindowOptions, canvas, div, point, px,
    size,
};
use vello::Scene;
use vello::wgpu;

use crate::gpu::{Gpu, Nv12Pass, Readback, Target, write_png};
use crate::graph::{BACKGROUND, Graph, Layout};
use crate::mac::{SurfaceSlot, psnr};
use crate::stats::Series;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mode {
    Surface,
    Readback,
    Layer,
}

pub struct Config {
    pub mode: Mode,
    pub shapes: Vec<usize>,
    pub warmup: usize,
    pub frames: usize,
    pub ring: usize,
    pub pipelined: bool,
    pub shots: PathBuf,
}

#[derive(Default)]
struct Samples {
    interval: Series,
    sim: Series,
    build: Series,
    render: Series,
    paint: Series,
    total: Series,
}

struct LayerTarget {
    surface: wgpu::Surface<'static>,
    blitter: wgpu::util::TextureBlitter,
    _layer: objc2::rc::Retained<objc2_quartz_core::CAMetalLayer>,
}

/// Everything the canvas owns on the GPU. Dropping it is what removing a
/// canvas element from a live window would do in the product.
struct Canvas {
    gpu: Gpu,
    scene: Scene,
    target: Option<Target>,
    nv12: Option<Nv12Pass>,
    slots: Vec<SurfaceSlot>,
    slot: usize,
    readback: Option<Readback>,
    rgba: Vec<u8>,
    prev_image: Option<Arc<RenderImage>>,
    layer: Option<LayerTarget>,
    pending: Option<(usize, wgpu::SubmissionIndex)>,
}

impl Canvas {
    /// Orderly teardown: wait for our GPU work, give GPUI's atlas its image
    /// back, detach the layer, then drop wgpu objects before the device.
    fn teardown(mut self, window: &mut Window) {
        self.gpu.wait();
        if let Some(img) = self.prev_image.take() {
            let _ = window.drop_image(img);
        }
        if let Some(layer) = self.layer.take() {
            let LayerTarget { surface, blitter, _layer } = layer;
            drop(blitter);
            drop(surface);
            _layer.removeFromSuperlayer();
        }
        self.pending = None;
        // IOSurface-backed wgpu textures first, then the pixel buffers they
        // alias (GPUI may still hold its own retain on the last one shown).
        self.slots.clear();
        self.nv12 = None;
        self.readback = None;
        self.target = None;
        let Canvas { gpu, .. } = self;
        let Gpu { renderer, queue, device, adapter, instance } = gpu;
        drop(renderer);
        drop(queue);
        drop(device);
        drop(adapter);
        drop(instance);
    }
}

enum Phase {
    Running,
    /// Canvas resources dropped; GPUI keeps drawing plain frames.
    AfterTeardown(usize),
}

struct Bench {
    cfg: Config,
    canvas: Option<Canvas>,
    phase: Phase,
    in_use_hits: usize,
    run: usize,
    graph: Graph,
    frame: usize,
    clock: Instant,
    last: Option<Instant>,
    run_started: Option<Instant>,
    samples: Samples,
    paint_time: Rc<RefCell<Option<Duration>>>,
}

impl Bench {
    fn layout_for(shapes: usize) -> Layout {
        // 1k: a real force simulation per frame (CPU physics + render).
        // 10k: pure render benchmark (closed-form motion, no physics).
        if shapes <= 1_000 { Layout::Force } else { Layout::Laid }
    }

    fn ensure_targets(&mut self, window: &mut Window) -> (u32, u32, f64) {
        let scale = window.scale_factor() as f64;
        let vp = window.viewport_size();
        let w = ((f32::from(vp.width) as f64 * scale).round() as u32) & !1;
        let h = ((f32::from(vp.height) as f64 * scale).round() as u32) & !1;
        let mode = self.cfg.mode;
        let ring = self.cfg.ring;
        let c = self.canvas.as_mut().unwrap();
        let stale = c.target.as_ref().is_none_or(|t| t.width != w || t.height != h);
        if stale {
            let target = Target::new(&c.gpu.device, w, h);
            match mode {
                Mode::Surface => {
                    c.nv12 = Some(Nv12Pass::new(&c.gpu.device, &target));
                    c.slots = (0..ring).map(|_| SurfaceSlot::new(&c.gpu.device, w, h)).collect();
                }
                Mode::Readback => c.readback = Some(Readback::new(&c.gpu.device, w, h)),
                Mode::Layer => c.layer = Some(make_layer(&c.gpu, window, w, h, scale)),
            }
            c.target = Some(target);
        }
        (w, h, scale)
    }

    fn finish_run(&mut self, w: u32, h: u32) {
        let shapes = self.cfg.shapes[self.run];
        let wall = self.run_started.map(|s| s.elapsed()).unwrap_or_default();
        let fps = self.cfg.frames as f64 / wall.as_secs_f64();
        let mut r = format!(
            "== mode {:?}{}  shapes {} ({:?})  {}x{} device px  {} frames (after {} warmup)\n\
             fps {:.1} (wall {:.2}s)\n\
             frame interval  {}\n\
             physics/motion  {}\n\
             scene encode    {}\n\
             render{}  {}\n\
             gpui paint call {}\n\
             our main-thread {}\n",
            self.cfg.mode,
            if self.cfg.pipelined { " (pipelined)" } else { "" },
            shapes,
            Self::layout_for(shapes),
            w,
            h,
            self.cfg.frames,
            self.cfg.warmup,
            fps,
            wall.as_secs_f64(),
            self.samples.interval.summary(),
            self.samples.sim.summary(),
            self.samples.build.summary(),
            match self.cfg.mode {
                Mode::Surface => "+nv12+wait",
                Mode::Readback => "+readback ",
                Mode::Layer => "+blit+pres",
            },
            self.samples.render.summary(),
            self.samples.paint.summary(),
            self.samples.total.summary(),
        );
        let tag = if shapes >= 1000 { format!("{}k", shapes / 1000) } else { shapes.to_string() };
        let c = self.canvas.as_mut().unwrap();
        if self.cfg.mode == Mode::Surface {
            r += &format!(
                "IOSurfaceIsInUse true when a slot was about to be overwritten: {} of {} frames\n",
                self.in_use_hits, self.cfg.frames
            );
            // Screenshot: vello's RGBA output and what GPUI samples (IOSurface decoded).
            let target = c.target.as_ref().unwrap();
            let rb = Readback::new(&c.gpu.device, w, h);
            let mut rgba = Vec::new();
            rb.read(&c.gpu, target, &mut rgba, false);
            c.gpu.wait();
            // The slot written last holds the same frame as the target.
            let last = (c.slot + c.slots.len() - 1) % c.slots.len();
            let decoded = c.slots[last].decode_rgba();
            write_png(&self.cfg.shots.join(format!("graph-{tag}-vello.png")), w, h, &rgba);
            write_png(&self.cfg.shots.join(format!("graph-{tag}-gpui-surface.png")), w, h, &decoded);
            r += &format!(
                "PSNR vello RGBA vs IOSurface GPUI samples (NV12 4:2:0 round trip): {:.2} dB\n",
                psnr(&rgba, &decoded)
            );
        }
        println!("{r}");
    }

    fn frame(&mut self, window: &mut Window) -> Option<FrameOut> {
        let (w, h, scale) = self.ensure_targets(window);
        let now = Instant::now();
        let measuring = self.frame >= self.cfg.warmup;
        if self.frame == self.cfg.warmup {
            self.run_started = Some(now);
        }
        if let (Some(last), true) = (self.last, measuring && self.frame > self.cfg.warmup) {
            self.samples.interval.push(now - last);
        }
        if let Some(p) = self.paint_time.borrow_mut().take()
            && measuring
            && self.frame > self.cfg.warmup
        {
            self.samples.paint.push(p);
        }
        self.last = Some(now);

        let t = self.clock.elapsed().as_secs_f64();
        let t0 = Instant::now();
        self.graph.update(t);
        let ts = Instant::now();
        let c = self.canvas.as_mut().unwrap();
        self.graph.encode(&mut c.scene, scale);
        let t1 = Instant::now();
        let target = c.target.as_ref().unwrap();
        c.gpu.render(&c.scene, target, BACKGROUND);
        let out = match self.cfg.mode {
            Mode::Surface => {
                let slot = &c.slots[c.slot];
                if slot.in_use() && measuring {
                    self.in_use_hits += 1;
                }
                let mut enc = c.gpu.device.create_command_encoder(&Default::default());
                c.nv12.as_ref().unwrap().encode(&mut enc, &slot.y_view, &slot.cbcr_view);
                let sub = c.gpu.queue.submit([enc.finish()]);
                let written = c.slot;
                c.slot = (c.slot + 1) % c.slots.len();
                // Fence: GPUI samples from another MTLCommandQueue (and MTLDevice
                // object); nothing orders it after our writes, so the CPU waits.
                let show = if self.cfg.pipelined {
                    // Show the previous frame's slot (one frame of latency) and
                    // wait only for *its* submission, which is normally done.
                    match c.pending.replace((written, sub)) {
                        Some((prev, prev_sub)) => {
                            c.gpu
                                .device
                                .poll(wgpu::PollType::Wait { submission_index: Some(prev_sub), timeout: None })
                                .expect("poll");
                            prev
                        }
                        None => {
                            c.gpu.wait();
                            written
                        }
                    }
                } else {
                    c.gpu.wait();
                    written
                };
                FrameOut::Surface(c.slots[show].pixel_buffer.clone())
            }
            Mode::Readback => {
                let rb = c.readback.as_ref().unwrap();
                let mut buf = std::mem::take(&mut c.rgba);
                rb.read(&c.gpu, target, &mut buf, true);
                let img = image::RgbaImage::from_raw(w, h, buf).expect("dims");
                let image = Arc::new(RenderImage::new(vec![image::Frame::new(img)]));
                if let Some(prev) = c.prev_image.replace(image.clone()) {
                    let _ = window.drop_image(prev);
                }
                FrameOut::Image(image)
            }
            Mode::Layer => {
                let lt = c.layer.as_ref().unwrap();
                let frame = match lt.surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(f)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
                    // GPUI renders the first frame inside applicationDidFinishLaunching,
                    // before the window is on screen: the layer reports Occluded.
                    // Skip (and do not count) that frame. Panicking here aborts the
                    // process (a panic cannot unwind out of that ObjC callback).
                    other => {
                        eprintln!("layer: skipped frame ({other:?})");
                        self.last = None;
                        return Some(FrameOut::Nothing);
                    }
                };
                let view = frame.texture.create_view(&Default::default());
                let mut enc = c.gpu.device.create_command_encoder(&Default::default());
                lt.blitter.copy(&c.gpu.device, &mut enc, &target.view, &view);
                c.gpu.queue.submit([enc.finish()]);
                c.gpu.queue.present(frame);
                FrameOut::Nothing
            }
        };
        let t2 = Instant::now();
        if measuring {
            self.samples.sim.push(ts - t0);
            self.samples.build.push(t1 - ts);
            self.samples.render.push(t2 - t1);
            self.samples.total.push(t2 - t0);
        }
        self.frame += 1;
        if self.frame == self.cfg.warmup + self.cfg.frames {
            self.finish_run(w, h);
            self.run += 1;
            if self.run == self.cfg.shapes.len() {
                return None;
            }
            let vp = window.viewport_size();
            let shapes = self.cfg.shapes[self.run];
            self.graph = Graph::new(
                shapes,
                f32::from(vp.width) as f64,
                f32::from(vp.height) as f64,
                Self::layout_for(shapes),
            );
            self.frame = 0;
            self.last = None;
            self.in_use_hits = 0;
            self.samples = Samples::default();
        }
        Some(out)
    }
}

fn make_layer(gpu: &Gpu, window: &Window, w: u32, h: u32, scale: f64) -> LayerTarget {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use objc2_quartz_core::{CALayer, CAMetalLayer};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let RawWindowHandle::AppKit(h_) = HasWindowHandle::window_handle(window).expect("handle").as_raw() else {
        panic!("not AppKit")
    };
    let view = h_.ns_view.as_ptr() as *mut AnyObject;
    let layer = CAMetalLayer::new();
    unsafe {
        // GPUI's NSView is layer-hosting; its backing layer is GPUI's own
        // CAMetalLayer. Our layer is a sublayer, so it draws ABOVE all GPUI content.
        let parent: *mut CALayer = msg_send![view, layer];
        let parent = &*parent;
        layer.setFrame(parent.bounds());
        layer.setContentsScale(scale);
        layer.setOpaque(true);
        parent.addSublayer(&layer);
    }
    let surface = unsafe {
        gpu.instance
            .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(
                objc2::rc::Retained::as_ptr(&layer) as *mut _,
            ))
            .expect("surface from CAMetalLayer")
    };
    let format = wgpu::TextureFormat::Bgra8Unorm;
    surface.configure(
        &gpu.device,
        &wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: w,
            height: h,
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::Auto,
        },
    );
    LayerTarget { surface, blitter: wgpu::util::TextureBlitter::new(&gpu.device, format), _layer: layer }
}

enum FrameOut {
    Surface(core_video::pixel_buffer::CVPixelBuffer),
    Image(Arc<RenderImage>),
    Nothing,
}

/// Frames GPUI keeps drawing after the canvas resources are gone, so the
/// "canvas removed from a live window" path is exercised before quitting.
const FRAMES_AFTER_TEARDOWN: usize = 30;

impl Render for Bench {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.request_animation_frame();
        match self.phase {
            Phase::AfterTeardown(0) => {
                println!("{FRAMES_AFTER_TEARDOWN} GPUI frames drawn after teardown; quitting via GPUI");
                cx.quit();
                return div().size_full().bg(gpui::rgb(0x141722)).into_any_element();
            }
            Phase::AfterTeardown(ref mut left) => {
                *left -= 1;
                return div().size_full().bg(gpui::rgb(0x141722)).into_any_element();
            }
            Phase::Running => {}
        }
        let Some(out) = self.frame(window) else {
            let t = Instant::now();
            if let Some(canvas) = self.canvas.take() {
                canvas.teardown(window);
            }
            println!(
                "all runs done; canvas teardown (wgpu device, vello, IOSurfaces/layer) took {:.2} ms",
                t.elapsed().as_secs_f64() * 1000.0
            );
            self.phase = Phase::AfterTeardown(FRAMES_AFTER_TEARDOWN);
            return div().size_full().bg(gpui::rgb(0x141722)).into_any_element();
        };
        let paint_time = self.paint_time.clone();
        div()
            .size_full()
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, (), window, _| {
                        let t = Instant::now();
                        match out {
                            FrameOut::Surface(pb) => window.paint_surface(bounds, pb),
                            FrameOut::Image(img) => {
                                let _ = window.paint_image(bounds, Corners::default(), img, 0, false);
                            }
                            FrameOut::Nothing => {}
                        }
                        *paint_time.borrow_mut() = Some(t.elapsed());
                    },
                )
                .size_full(),
            )
            .into_any_element()
    }
}

pub fn run(cfg: Config) {
    Application::new().run(move |cx: &mut App| {
        let logical = size(px(1600.), px(1000.));
        // Bottom-right of the main display, non-activating floating panel so it
        // neither steals focus nor gets occluded (occluded windows stop drawing).
        let display = cx.primary_display().expect("display");
        let db = display.bounds();
        let origin = point(
            db.origin.x + db.size.width - logical.width - px(8.),
            db.origin.y + db.size.height - logical.height - px(8.),
        );
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds { origin, size: logical })),
                titlebar: None,
                focus: false,
                show: true,
                kind: WindowKind::PopUp,
                is_movable: false,
                is_resizable: false,
                ..Default::default()
            },
            |window, cx| {
                let gpu = Gpu::new();
                println!("adapter: {}", gpu.adapter_line());
                let vp = window.viewport_size();
                let graph = Graph::new(
                    cfg.shapes[0],
                    f32::from(vp.width) as f64,
                    f32::from(vp.height) as f64,
                    Bench::layout_for(cfg.shapes[0]),
                );
                cx.new(|_| Bench {
                    cfg,
                    canvas: Some(Canvas {
                        gpu,
                        scene: Scene::new(),
                        target: None,
                        nv12: None,
                        slots: Vec::new(),
                        slot: 0,
                        readback: None,
                        rgba: Vec::new(),
                        prev_image: None,
                        layer: None,
                        pending: None,
                    }),
                    phase: Phase::Running,
                    in_use_hits: 0,
                    run: 0,
                    graph,
                    frame: 0,
                    clock: Instant::now(),
                    last: None,
                    run_started: None,
                    samples: Samples::default(),
                    paint_time: Rc::new(RefCell::new(None)),
                })
            },
        )
        .expect("open window");
    });
}
