//! Spike: vello inside a GPUI window. Findings: the draft PR "Spike: vello inside a GPUI window".
//!
//!   vello-gpui headless  [--frames N]          offscreen GPU costs + screenshots + coverage.png
//!   vello-gpui window --mode surface|readback|layer [--frames N] [--warmup N] [--ring N]
//!
//! Every window run measures a fixed number of frames per scene size, prints,
//! and quits itself; a watchdog kills the process if that ever fails.

mod coverage;
mod gpu;
mod graph;
/// What graph.rs draws with.
mod gfx {
    pub use vello::{Scene, kurbo, peniko};
}
#[cfg(feature = "sparse")]
mod sparse;
#[cfg(target_os = "macos")]
mod mac;
mod stats;
#[cfg(target_os = "macos")]
mod window;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpu::{Gpu, Readback, Target, write_png};
use graph::{BACKGROUND, Graph, Layout};
use stats::Series;
use vello::Scene;

fn arg<T: std::str::FromStr>(args: &[String], name: &str, default: T) -> T {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn shots_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shots").join("raw")
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // Watchdog: never leave a window (or a hung GPU wait) running.
    let limit = arg(&args, "--watchdog", 90u64);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(limit));
        eprintln!("watchdog: {limit}s elapsed, exiting");
        std::process::exit(2);
    });
    match args.get(1).map(String::as_str) {
        Some("headless") => headless(&args),
        #[cfg(feature = "sparse")]
        Some("sparse") => sparse::run(arg(&args, "--frames", 60), &[1_000, 10_000], &shots_dir()),
        #[cfg(target_os = "macos")]
        Some("window") => {
            let mode = match args
                .iter()
                .position(|a| a == "--mode")
                .and_then(|i| args.get(i + 1))
                .map(String::as_str)
            {
                Some("surface") => window::Mode::Surface,
                Some("readback") => window::Mode::Readback,
                Some("layer") => window::Mode::Layer,
                other => panic!("--mode surface|readback|layer, got {other:?}"),
            };
            window::run(window::Config {
                mode,
                shapes: vec![1_000, 10_000],
                warmup: arg(&args, "--warmup", 30),
                frames: arg(&args, "--frames", 240),
                // GPUI keeps up to 3 drawables in flight; one more slot than
                // that (two more when pipelined) is never still being sampled.
                ring: arg(&args, "--ring", if args.iter().any(|a| a == "--pipelined") { 5 } else { 4 }),
                pipelined: args.iter().any(|a| a == "--pipelined"),
                shots: shots_dir(),
            });
        }
        _ => eprintln!("usage: vello-gpui headless | window --mode surface|readback|layer"),
    }
}

/// Offscreen numbers (no GPUI, no vsync): what each stage costs on this GPU at
/// the window's device size (1600x1000 logical at 2x = 3200x2000).
fn headless(args: &[String]) {
    let frames = arg(args, "--frames", 120usize);
    let (w, h, scale) = (3200u32, 2000u32, 2.0f64);
    let t = Instant::now();
    let mut gpu = Gpu::new();
    println!("adapter: {}  (device + vello pipelines: {:.0} ms)", gpu.adapter_line(), t.elapsed().as_secs_f64() * 1000.0);
    let target = Target::new(&gpu.device, w, h);
    let readback = Readback::new(&gpu.device, w, h);
    #[cfg(target_os = "macos")]
    let nv12 = gpu::Nv12Pass::new(&gpu.device, &target);
    #[cfg(target_os = "macos")]
    let slot = mac::SurfaceSlot::new(&gpu.device, w, h);
    let mut scene = Scene::new();
    let mut rgba = Vec::new();
    let shapes_list: Vec<usize> = args
        .iter()
        .position(|a| a == "--shapes")
        .and_then(|i| args.get(i + 1))
        .map(|v| v.split(',').map(|n| n.parse().expect("--shapes a,b,c")).collect())
        .unwrap_or_else(|| vec![1_000, 10_000]);
    let layout = if args.iter().any(|a| a == "--hairball") { Layout::Hairball } else { Layout::Laid };
    for shapes in shapes_list {
        // As in the window: 1k runs the force simulation unless a layout is forced.
        let layout = if layout == Layout::Laid && shapes <= 1_000 && !args.iter().any(|a| a == "--no-physics") { Layout::Force } else { layout };
        let mut graph = Graph::new(shapes, w as f64 / scale, h as f64 / scale, layout);
        let (mut sim, mut build, mut render, mut nv, mut rb) =
            (Series::default(), Series::default(), Series::default(), Series::default(), Series::default());
        for f in 0..frames + 90 {
            let t0 = Instant::now();
            graph.update(f as f64 / 60.0);
            let ts = Instant::now();
            graph.encode(&mut scene, scale);
            let t1 = Instant::now();
            gpu.render(&scene, &target, BACKGROUND);
            gpu.wait();
            let t2 = Instant::now();
            #[cfg(target_os = "macos")]
            {
                let mut enc = gpu.device.create_command_encoder(&Default::default());
                nv12.encode(&mut enc, &slot.y_view, &slot.cbcr_view);
                gpu.queue.submit([enc.finish()]);
                gpu.wait();
            }
            let t3 = Instant::now();
            readback.read(&gpu, &target, &mut rgba, true);
            let t4 = Instant::now();
            if f >= 90 {
                sim.push(ts - t0);
                build.push(t1 - ts);
                render.push(t2 - t1);
                nv.push(t3 - t2);
                rb.push(t4 - t3);
            }
        }
        // Did vello really draw? Paint the target magenta, render the scene
        // once more, and count magenta pixels left behind. vello 0.11 drops a
        // frame silently when its fixed-size GPU buffers overflow.
        let dropped = frame_dropped(&mut gpu, &target, &readback, &scene, &mut rgba);
        write_png(&shots_dir().join(format!("headless-{shapes}-{layout:?}.png").to_lowercase()), w, h, &rgba);
        println!("== headless shapes {shapes} layout {layout:?}  {w}x{h}  {frames} frames");
        println!("frame actually rendered: {}", if dropped { "NO - vello dropped it (buffer overflow)" } else { "yes" });
        #[cfg(feature = "diag")]
        diag(&mut gpu, &target, &scene);
        println!("physics/motion (CPU)            {}", sim.summary());
        println!("scene encode (CPU)              {}", build.summary());
        println!("vello render + wait             {}", render.summary());
        println!("NV12 pass into IOSurface + wait {}", nv.summary());
        println!("readback copy+map+BGRA swizzle  {}", rb.summary());
        println!(
            "=> zero-copy GPU path {:.2} ms/frame; readback path {:.2} ms/frame (+ GPUI atlas upload)",
            render.mean() + nv.mean(),
            render.mean() + rb.mean()
        );
    }
    // Coverage sheet.
    let (cw, ch) = coverage::SIZE;
    let ctarget = Target::new(&gpu.device, cw, ch);
    let creadback = Readback::new(&gpu.device, cw, ch);
    let mut cscene = Scene::new();
    coverage::build(&mut cscene);
    gpu.render(&cscene, &ctarget, coverage::BACKGROUND);
    creadback.read(&gpu, &ctarget, &mut rgba, false);
    write_png(&shots_dir().join("coverage.png"), cw, ch, &rgba);
    println!("wrote {}", shots_dir().join("coverage.png").display());
}

pub const MAGENTA: vello::peniko::Color = vello::peniko::Color::from_rgba8(0xff, 0x00, 0xff, 0xff);

/// True when vello left the target untouched (magenta) for the given scene.
pub fn frame_dropped(gpu: &mut Gpu, target: &Target, readback: &Readback, scene: &Scene, rgba: &mut Vec<u8>) -> bool {
    gpu.render(&Scene::new(), target, MAGENTA);
    gpu.render(scene, target, BACKGROUND);
    readback.read(gpu, target, rgba, false);
    let magenta = rgba.chunks_exact(4).filter(|p| p[0] == 0xff && p[1] == 0 && p[2] == 0xff).count();
    magenta * 200 > rgba.len() / 4
}

/// vello's bump counters (needs vello's `debug_layers` feature): how much of
/// each fixed-size GPU buffer this scene needed, and which stage failed.
#[cfg(feature = "diag")]
fn diag(gpu: &mut Gpu, target: &Target, scene: &Scene) {
    #[allow(deprecated, reason = "the only API that returns vello's bump counters")]
    let device = gpu.device.clone();
    let bump = vello::util::block_on_wgpu(&device, gpu.renderer.render_to_texture_async(
        &gpu.device,
        &gpu.queue,
        scene,
        &target.view,
        &vello::RenderParams {
            base_color: BACKGROUND,
            width: target.width,
            height: target.height,
            antialiasing_method: vello::AaConfig::Area,
        },
        vello::low_level::DebugLayers::none(),
    ))
    .expect("render");
    if let Some(b) = bump {
        println!(
            "vello bump: failed=0x{:x} lines={} (cap {}) segments={} (cap {}) seg_counts={} (cap {}) tiles={} (cap {}) ptcl={} (cap {}) binning={} (cap {}) blend={}",
            b.failed, b.lines, 1u32 << 21, b.segments, 1u32 << 21, b.seg_counts, 1u32 << 21, b.tile, 1u32 << 21, b.ptcl, 1u32 << 23, b.binning, 1u32 << 18, b.blend
        );
    }
}
