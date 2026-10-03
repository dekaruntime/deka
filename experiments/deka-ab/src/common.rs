//! Shared by both backends: the apps, the measurement protocol and process
//! metrics. Nothing here depends on GPUI, winit, wgpu or vello.

use deka_native_ui::{Application, Edges, Node, Style};
use std::time::{Duration, Instant};

/// Logical window size; at 2x this is 3200x2000 device pixels.
pub const WINDOW_W: f32 = 1600.;
pub const WINDOW_H: f32 = 1000.;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum App {
    /// crates/deka_native_ui/examples/portfolio-world.rs (deka's World).
    World,
    /// A static, text-heavy deka UI through `deka_native_ui::Application`.
    Ui,
    /// Phase 1 canvas scenes (vello backend only).
    Canvas,
    /// Text quality sheet (offscreen PNG).
    Text,
    /// parley PlainEditor (vello backend only).
    Editor,
    /// vello buffer needs of each scene (vello backend, `diag` feature).
    Bump,
    /// Offscreen GPU memory of vello rendering the UI and world scenes.
    Mem,
}

#[derive(Clone, Debug)]
pub struct Args {
    pub app: App,
    pub warmup: usize,
    pub frames: usize,
    /// Quit as soon as the second frame starts (cold start measurement).
    pub first_frame: bool,
    /// Stay idle this many seconds after the first frame, then quit.
    pub idle_secs: Option<f64>,
    /// Programmatically resize the window while animating.
    pub resize: bool,
    pub interactive: bool,
    pub shapes: Vec<usize>,
}

impl Args {
    pub fn parse() -> Self {
        let args: Vec<String> = std::env::args().collect();
        let get = |name: &str| {
            args.iter()
                .position(|a| a == name)
                .and_then(|i| args.get(i + 1))
                .cloned()
        };
        let app = match get("--app").as_deref() {
            Some("ui") => App::Ui,
            Some("canvas") => App::Canvas,
            Some("text") => App::Text,
            Some("editor") => App::Editor,
            Some("bump") => App::Bump,
            Some("mem") => App::Mem,
            _ => App::World,
        };
        Self {
            app,
            warmup: get("--warmup").and_then(|v| v.parse().ok()).unwrap_or(30),
            frames: get("--frames").and_then(|v| v.parse().ok()).unwrap_or(240),
            first_frame: args.iter().any(|a| a == "--first-frame"),
            idle_secs: get("--idle").and_then(|v| v.parse().ok()),
            resize: args.iter().any(|a| a == "--resize"),
            interactive: args.iter().any(|a| a == "--interactive"),
            shapes: get("--shapes")
                .map(|v| v.split(',').filter_map(|n| n.parse().ok()).collect())
                .unwrap_or_else(|| vec![1_000, 10_000]),
        }
    }
}

/// Exit the process if a run ever hangs; interactive runs are exempt.
pub fn watchdog(args: &Args) {
    if args.interactive {
        return;
    }
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(90));
        eprintln!("watchdog: 90s elapsed, exiting");
        std::process::exit(2);
    });
}

/// Process metrics from `proc_pid_rusage` (what Activity Monitor reports).
#[derive(Clone, Copy, Debug, Default)]
pub struct Usage {
    /// Physical footprint, bytes (Activity Monitor's "Memory").
    pub footprint: u64,
    pub resident: u64,
    /// User + system CPU time, nanoseconds.
    pub cpu_ns: u64,
    pub sys_ns: u64,
    /// Nanoseconds since exec.
    pub since_exec_ns: u64,
}

pub fn usage() -> Usage {
    #[cfg(target_os = "macos")]
    #[allow(deprecated, reason = "libc points to mach2 for mach_timebase_info; one call is not worth a dependency")]
    unsafe {
        let mut info: libc::rusage_info_v4 = std::mem::zeroed();
        let rc = libc::proc_pid_rusage(
            libc::getpid(),
            libc::RUSAGE_INFO_V4,
            &mut info as *mut _ as *mut libc::rusage_info_t,
        );
        if rc != 0 {
            return Usage::default();
        }
        let mut tb = libc::mach_timebase_info { numer: 0, denom: 0 };
        libc::mach_timebase_info(&mut tb);
        let to_ns = |t: u64| t * tb.numer as u64 / tb.denom.max(1) as u64;
        let now = libc::mach_absolute_time();
        Usage {
            footprint: info.ri_phys_footprint,
            resident: info.ri_resident_size,
            cpu_ns: to_ns(info.ri_user_time + info.ri_system_time),
            sys_ns: to_ns(info.ri_system_time),
            since_exec_ns: to_ns(now.saturating_sub(info.ri_proc_start_abstime)),
        }
    }
    #[cfg(not(target_os = "macos"))]
    Usage::default()
}

/// GPU memory this process has allocated on the system GPU, in MB. Metal
/// devices are per-process singletons, so this also covers GPUI's renderer.
pub fn gpu_mb() -> f64 {
    #[cfg(target_os = "macos")]
    {
        use objc2_metal::MTLDevice;
        objc2_metal::MTLCreateSystemDefaultDevice()
            .map(|d| d.currentAllocatedSize() as f64 / (1024. * 1024.))
            .unwrap_or(-1.)
    }
    #[cfg(not(target_os = "macos"))]
    -1.
}

pub fn mb(bytes: u64) -> f64 {
    bytes as f64 / (1024. * 1024.)
}

/// Frame timing series in milliseconds.
#[derive(Default, Clone)]
pub struct Series(pub Vec<f64>);

impl Series {
    pub fn push(&mut self, d: Duration) {
        self.0.push(d.as_secs_f64() * 1000.);
    }
    pub fn push_ms(&mut self, ms: f64) {
        self.0.push(ms);
    }
    pub fn mean(&self) -> f64 {
        self.0.iter().sum::<f64>() / self.0.len().max(1) as f64
    }
    pub fn summary(&self) -> String {
        if self.0.is_empty() {
            return "n/a".into();
        }
        let mut v = self.0.clone();
        v.sort_by(|a, b| a.total_cmp(b));
        let pct = |p: f64| v[((v.len() - 1) as f64 * p).round() as usize];
        format!(
            "mean {:6.2}  p50 {:6.2}  p95 {:6.2}  max {:6.2} ms",
            self.mean(),
            pct(0.5),
            pct(0.95),
            v[v.len() - 1]
        )
    }
}

/// The measurement protocol both backends follow, driven once per frame.
pub struct Protocol {
    pub args: Args,
    pub backend: &'static str,
    frame: usize,
    last: Option<Instant>,
    started: Option<Instant>,
    cpu_at_start: u64,
    sys_at_start: u64,
    pub interval: Series,
    pub work: Series,
    /// Backend B only: drawable acquire (vsync wait) + blit + present.
    pub present: Series,
    first_frame_logged: bool,
    idle_start: Option<(Instant, Usage, u64)>,
    pub redraws: u64,
    /// Sticky: once the run is over every later frame quits.
    done: bool,
}

pub enum Step {
    /// Keep going; request another frame if `animate`.
    Continue,
    /// The run is complete: tear down and quit.
    Quit,
}

impl Protocol {
    pub fn new(args: Args, backend: &'static str) -> Self {
        Self {
            args,
            backend,
            frame: 0,
            last: None,
            started: None,
            cpu_at_start: 0,
            sys_at_start: 0,
            interval: Series::default(),
            work: Series::default(),
            present: Series::default(),
            first_frame_logged: false,
            idle_start: None,
            redraws: 0,
            done: false,
        }
    }

    pub fn first_frame_done(&self) -> bool {
        self.first_frame_logged
    }

    pub fn frame_index(&self) -> usize {
        self.frame
    }

    pub fn measuring(&self) -> bool {
        self.frame >= self.args.warmup
    }

    /// Call at the start of every frame callback. Returns Quit when the run is over.
    pub fn begin_frame(&mut self) -> Step {
        if self.done {
            return Step::Quit;
        }
        self.redraws += 1;
        let now = Instant::now();
        if self.frame == 1 && self.args.first_frame {
            // Second frame started: the first one has been submitted.
            return Step::Quit;
        }
        if self.frame == self.args.warmup {
            self.started = Some(now);
            let u = usage();
            self.cpu_at_start = u.cpu_ns;
            self.sys_at_start = u.sys_ns;
        }
        if let (Some(last), true) = (self.last, self.measuring() && self.frame > self.args.warmup) {
            self.interval.push(now - last);
        }
        self.last = Some(now);
        Step::Continue
    }

    /// Call after the frame's own work (scene, encode, submit).
    pub fn end_frame(&mut self, work: Duration) -> Step {
        if !self.first_frame_logged {
            self.first_frame_logged = true;
            let u = usage();
            println!(
                "[{}] first frame submitted at {:.1} ms since exec; footprint {:.1} MB, resident {:.1} MB, GPU {:.1} MB",
                self.backend,
                u.since_exec_ns as f64 / 1e6,
                mb(u.footprint),
                mb(u.resident),
                gpu_mb()
            );
            if self.args.idle_secs.is_some() {
                self.idle_start = Some((Instant::now(), u, self.redraws));
            }
        }
        if self.measuring() {
            self.work.push(work);
        }
        self.frame += 1;
        if self.args.idle_secs.is_none()
            && !self.args.first_frame
            && self.frame == self.args.warmup + self.args.frames
        {
            self.report_animation();
            self.done = true;
            return Step::Quit;
        }
        Step::Continue
    }

    /// For idle runs: has the idle period elapsed? Prints the idle report once.
    pub fn idle_done(&mut self) -> bool {
        let Some((t0, u0, r0)) = self.idle_start else { return false };
        let Some(secs) = self.args.idle_secs else { return false };
        if t0.elapsed().as_secs_f64() < secs {
            return false;
        }
        let u1 = usage();
        let wall = t0.elapsed().as_secs_f64();
        println!(
            "[{}] idle {:.1}s: redraws during idle {}; CPU {:.3}% ({:.1} ms CPU); footprint {:.1} -> {:.1} MB; resident {:.1} -> {:.1} MB; GPU {:.1} MB",
            self.backend,
            wall,
            self.redraws - r0,
            (u1.cpu_ns - u0.cpu_ns) as f64 / 1e9 / wall * 100.,
            (u1.cpu_ns - u0.cpu_ns) as f64 / 1e6,
            mb(u0.footprint),
            mb(u1.footprint),
            mb(u0.resident),
            mb(u1.resident),
            gpu_mb(),
        );
        self.idle_start = None;
        self.done = true;
        true
    }

    pub fn report_animation(&self) {
        let wall = self.started.map(|s| s.elapsed().as_secs_f64()).unwrap_or(1.);
        let u = usage();
        let cpu = (u.cpu_ns - self.cpu_at_start) as f64 / 1e9;
        let sys = (u.sys_ns - self.sys_at_start) as f64 / 1e9;
        println!(
            "[{}] {:?}{} {} frames: fps {:.1}; process CPU {:.1}% of one core ({:.1}% kernel)\n  frame interval {}\n  our frame work {}\n  acquire+present {}\n  footprint {:.1} MB, resident {:.1} MB, GPU {:.1} MB",
            self.backend,
            self.args.app,
            if self.args.resize { " (resizing)" } else { "" },
            self.args.frames,
            self.args.frames as f64 / wall,
            cpu / wall * 100.,
            sys / wall * 100.,
            self.interval.summary(),
            self.work.summary(),
            self.present.summary(),
            mb(u.footprint),
            mb(u.resident),
            gpu_mb(),
        );
    }
}

/// Resize schedule for `--resize`: logical size for a given frame, sweeping
/// 1600x1000 -> 1100x700 -> 1600x1000 over 120 frames.
pub fn resize_size(frame: usize) -> (f32, f32) {
    let t = (frame % 120) as f32 / 120.;
    let k = 1. - (t * std::f32::consts::TAU).cos().mul_add(-0.5, 0.5);
    (WINDOW_W - 500. * (1. - k), WINDOW_H - 300. * (1. - k))
}

/// A static, text-heavy deka UI (settings-like page). No animations, so a
/// correct backend redraws it only when something changes.
pub struct UiApp;

fn text(id: &str, s: &str, size: f32, color: u32) -> Node {
    Node {
        id: id.into(),
        text: Some(s.into()),
        style: Style { font_size: Some(size), color: Some(color), ..Default::default() },
        on_click: None,
        children: vec![],
    }
}

impl Application for UiApp {
    fn initial_state(&self) -> Vec<f64> {
        vec![0.]
    }
    fn render(&self, state: &[f64]) -> Node {
        let rows = (0..18)
            .map(|i| Node {
                id: format!("row{i}"),
                text: None,
                on_click: None,
                style: Style {
                    row: true,
                    justify: deka_native_ui::Justify::Between,
                    align: deka_native_ui::Align::Center,
                    padding: Edges { top: 8., right: 12., bottom: 8., left: 12. },
                    background: Some(if i % 2 == 0 { 0xf4f1ea } else { 0xffffff }),
                    radius: 6.,
                    ..Default::default()
                },
                children: vec![
                    {
                        let mut label = text(&format!("label{i}"), &format!("Setting number {i}: synchronise the workspace when idle"), 14., 0x222222);
                        label.style.grow = 1.;
                        label.style.shrink = 1.;
                        label
                    },
                    Node {
                        id: format!("button{i}"),
                        text: Some(if state[0] as usize == i { "On".into() } else { "Off".into() }),
                        on_click: Some(0),
                        style: Style {
                            padding: Edges { top: 4., right: 14., bottom: 4., left: 14. },
                            background: Some(0x1d3557),
                            color: Some(0xffffff),
                            font_size: Some(12.),
                            radius: 12.,
                            shrink: 0.,
                            ..Default::default()
                        },
                        children: vec![],
                    },
                ],
            })
            .collect::<Vec<_>>();
        Node {
            id: "root".into(),
            text: None,
            on_click: None,
            style: Style {
                padding: Edges { top: 24., right: 32., bottom: 24., left: 32. },
                gap_y: 8.,
                background: Some(0xffffff),
                ..Default::default()
            },
            children: [
                vec![
                    text("title", "Deka settings", 24., 0x111111),
                    text(
                        "intro",
                        "A static page drawn by deka_native_ui: taffy layout, fontdue text, one Scene. Nothing animates, so an idle window should cost nothing.",
                        16.,
                        0x444444,
                    ),
                ],
                rows,
            ]
            .concat(),
        }
    }
    fn event(&self, _handler: usize, state: &mut [f64]) {
        state[0] = (state[0] + 1.) % 18.;
    }
}

/// The text sample for the quality sheet.
pub const TEXT_SAMPLE: &str = "Hamburgefonstiv 0123456789 — The quick brown fox jumps over the lazy dog.";
pub const TEXT_SIZES: [f32; 5] = [11., 12., 14., 16., 24.];
pub static FONT_BYTES: &[u8] =
    include_bytes!("../../../crates/deka_native_ui/assets/AtkinsonHyperlegible-Regular.ttf");

/// Write RGBA8 to PNG; errors are reported, never panicked on.
pub fn write_png(path: &std::path::Path, width: u32, height: u32, rgba: &[u8]) {
    if let Some(dir) = path.parent()
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        eprintln!("mkdir {}: {e}", dir.display());
        return;
    }
    let file = match std::fs::File::create(path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("create {}: {e}", path.display());
            return;
        }
    };
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    match enc.write_header() {
        Ok(mut w) => {
            if let Err(e) = w.write_image_data(rgba) {
                eprintln!("png {}: {e}", path.display());
            }
        }
        Err(e) => eprintln!("png {}: {e}", path.display()),
    }
}

pub fn shots_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shots").join("raw")
}

/// The deka Scene for the text sheet: one text node per size, through
/// deka_native_ui's own renderer (taffy + fontdue), exactly as the app does.
pub fn text_sheet_root() -> Node {
    Node {
        id: "sheet".into(),
        text: None,
        on_click: None,
        style: Style {
            padding: Edges { top: 12., right: 12., bottom: 12., left: 12. },
            gap_y: 10.,
            background: Some(0xffffff),
            ..Default::default()
        },
        children: TEXT_SIZES
            .iter()
            .map(|s| text(&format!("t{s}"), &format!("{s}px  {TEXT_SAMPLE}"), *s, 0x111111))
            .collect(),
    }
}
pub const SHEET_W: f32 = 980.;
pub const SHEET_H: f32 = 190.;

/// Read an RGBA8 PNG (as written by `write_png`).
pub fn read_png(path: &std::path::Path) -> Option<(u32, u32, Vec<u8>)> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader = png::Decoder::new(std::io::BufReader::new(file)).read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    buf.truncate(info.buffer_size());
    Some((info.width, info.height, buf))
}

/// Stack PNGs vertically (same width) with a grey separator, for side-by-side review.
pub fn stack_pngs(inputs: &[std::path::PathBuf], output: &std::path::Path) -> bool {
    let images: Vec<_> = inputs.iter().filter_map(|p| read_png(p)).collect();
    if images.len() != inputs.len() || images.is_empty() {
        eprintln!("stack: missing inputs");
        return false;
    }
    let width = images.iter().map(|i| i.0).max().unwrap_or(1);
    let gap = 6u32;
    let height = images.iter().map(|i| i.1 + gap).sum::<u32>();
    let mut out = vec![0x99u8; (width * height * 4) as usize];
    let mut y0 = 0u32;
    for (w, h, px) in &images {
        for y in 0..*h {
            let src = &px[(y * w * 4) as usize..((y + 1) * w * 4) as usize];
            let dst = ((y0 + y) * width * 4) as usize;
            out[dst..dst + src.len()].copy_from_slice(src);
        }
        y0 += h + gap;
    }
    write_png(output, width, height, &out);
    true
}

/// Crop (x, y, w, h) and upscale by an integer factor with nearest-neighbour
/// sampling, so antialiasing can be judged pixel by pixel.
pub fn crop_zoom(path: &std::path::Path, rect: (u32, u32, u32, u32), factor: u32, out: &std::path::Path) -> bool {
    let Some((w, h, px)) = read_png(path) else { return false };
    let (x0, y0, cw, ch) = rect;
    let (cw, ch) = (cw.min(w.saturating_sub(x0)), ch.min(h.saturating_sub(y0)));
    let (ow, oh) = (cw * factor, ch * factor);
    let mut o = vec![0u8; (ow * oh * 4) as usize];
    for y in 0..oh {
        for x in 0..ow {
            let s = (((y0 + y / factor) * w + x0 + x / factor) * 4) as usize;
            let d = ((y * ow + x) * 4) as usize;
            o[d..d + 4].copy_from_slice(&px[s..s + 4]);
        }
    }
    write_png(out, ow, oh, &o);
    true
}

/// Print a startup trace point (ms since exec) when run with --trace.
pub fn trace(label: &str) {
    if std::env::args().any(|a| a == "--trace") {
        println!("[trace] {:8.1} ms  {label}", usage().since_exec_ns as f64 / 1e6);
    }
}
