//! Shared by both binaries: the scenes, the measurement protocol and process
//! metrics. Nothing here depends on GPUI, winit or wgpu.
use deka_native_ui::scene::{Renderer, Scene};
use deka_native_ui::world::World;
use deka_native_ui::{Application, Node, Style};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Arguments

pub fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}
pub fn flag(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}
pub fn app_name() -> String {
    arg("--app").unwrap_or_else(|| "world".into())
}

// ---------------------------------------------------------------------------
// Process metrics (what Activity Monitor shows)

#[derive(Clone, Copy, Debug, Default)]
pub struct Usage {
    pub footprint: u64,
    pub resident: u64,
    pub cpu_ns: u64,
    pub sys_ns: u64,
    pub since_exec_ns: u64,
}

pub fn usage() -> Usage {
    #[allow(deprecated, reason = "libc points to mach2 for mach_timebase_info")]
    // SAFETY: proc_pid_rusage fills a zeroed rusage_info_v4 for this process;
    // mach_timebase_info fills a plain struct.
    unsafe {
        let mut info: libc::rusage_info_v4 = std::mem::zeroed();
        let rc = libc::proc_pid_rusage(
            libc::getpid(),
            libc::RUSAGE_INFO_V4,
            (&raw mut info).cast(),
        );
        if rc != 0 {
            return Usage::default();
        }
        let mut tb = libc::mach_timebase_info { numer: 0, denom: 0 };
        libc::mach_timebase_info(&mut tb);
        let to_ns = |t: u64| t * u64::from(tb.numer) / u64::from(tb.denom.max(1));
        let now = libc::mach_absolute_time();
        Usage {
            footprint: info.ri_phys_footprint,
            resident: info.ri_resident_size,
            cpu_ns: to_ns(info.ri_user_time + info.ri_system_time),
            sys_ns: to_ns(info.ri_system_time),
            since_exec_ns: to_ns(now.saturating_sub(info.ri_proc_start_abstime)),
        }
    }
}

/// Metal memory this process has allocated (covers either backend's renderer).
pub fn gpu_mb() -> f64 {
    use objc2_metal::MTLDevice;
    objc2_metal::MTLCreateSystemDefaultDevice()
        .map(|d| d.currentAllocatedSize() as f64 / 1048576.)
        .unwrap_or(-1.)
}

pub fn mb(bytes: u64) -> f64 {
    bytes as f64 / 1048576.
}

fn load_avg() -> String {
    let mut loads = [0f64; 3];
    // SAFETY: getloadavg writes up to 3 doubles into the array.
    let n = unsafe { libc::getloadavg(loads.as_mut_ptr(), 3) };
    if n == 3 {
        format!("{:.2} {:.2} {:.2}", loads[0], loads[1], loads[2])
    } else {
        "n/a".into()
    }
}

// ---------------------------------------------------------------------------
// Timeline: milliseconds since the kernel started the process

static TIMELINE: Mutex<Vec<(f64, String)>> = Mutex::new(Vec::new());
pub fn mark(label: &str) {
    let ms = usage().since_exec_ns as f64 / 1e6;
    if let Ok(mut t) = TIMELINE.lock() {
        t.push((ms, label.to_owned()));
    }
}
fn timeline_ms(label: &str) -> Option<f64> {
    TIMELINE
        .lock()
        .ok()?
        .iter()
        .find(|(_, l)| l == label)
        .map(|(ms, _)| *ms)
}

/// The backend's own start-up marks (the new window's `window::trace`).
pub static BACKEND_MARKS: Mutex<Option<fn() -> Vec<(&'static str, Instant)>>> = Mutex::new(None);

/// `TRACE` line: every mark in ms since exec, in time order.
fn trace_line() -> String {
    let now_ms = usage().since_exec_ns as f64 / 1e6;
    let now = Instant::now();
    let mut all: Vec<(f64, String)> = TIMELINE.lock().map(|t| t.clone()).unwrap_or_default();
    if let Some(f) = BACKEND_MARKS.lock().ok().and_then(|f| *f) {
        for (label, at) in f() {
            all.push((now_ms - now.duration_since(at).as_secs_f64() * 1000., label.to_owned()));
        }
    }
    all.sort_by(|a, b| a.0.total_cmp(&b.0));
    let parts: Vec<String> = all.iter().map(|(ms, l)| format!("{l}={ms:.1}")).collect();
    format!("TRACE {}", parts.join(" | "))
}

static ON_SCREEN: AtomicBool = AtomicBool::new(false);
/// Bounds of this process's window when the window server first listed it.
static FIRST_BOUNDS: Mutex<Option<display::Rect>> = Mutex::new(None);
/// Poll the window server for this process's first on-screen window.
pub fn start_visibility_probe() {
    let _ = std::thread::Builder::new()
        .name("probe".into())
        .spawn(|| {
            let t0 = Instant::now();
            let pid = std::process::id() as i32;
            while t0.elapsed() < Duration::from_secs(5) {
                if let Some(w) = display::on_screen_windows().into_iter().find(|w| w.pid == pid) {
                    mark("window on screen");
                    if let Ok(mut b) = FIRST_BOUNDS.lock() {
                        *b = Some(w.bounds);
                    }
                    ON_SCREEN.store(true, Ordering::Relaxed);
                    return;
                }
                std::thread::sleep(Duration::from_micros(500));
            }
        });
}

/// `window=[x y w h] placement=virtual|OFF:<why>` for the RESULT lines: where
/// the window server first showed this process's window.
fn placement() -> String {
    let Some(b) = FIRST_BOUNDS.lock().ok().and_then(|b| *b) else {
        return "window=none placement=none".into();
    };
    let verdict = match target_display() {
        Some(id) => match display::check_window(&b, id) {
            Ok(()) => "virtual".to_owned(),
            Err(e) => format!("OFF:{}", e.replace(' ', "_")),
        },
        None => "OFF:no_display".into(),
    };
    format!("window=[{:.0} {:.0} {:.0} {:.0}] placement={verdict}", b.x, b.y, b.w, b.h)
}

/// The virtual display this run must put its window on (`--display ID`).
pub fn target_display() -> Option<u32> {
    arg("--display").and_then(|v| v.parse().ok())
}

/// Refuse to open a window anywhere but on the harness's virtual display, so
/// a measurement never shows a window on a physical screen.
pub fn require_virtual_display() -> u32 {
    let Some(id) = target_display() else {
        eprintln!("refusing to open a window: no --display (run through cmp-batch, which creates a virtual display)");
        std::process::exit(2);
    };
    if !display::is_harness_display(id) {
        eprintln!("refusing to open a window: display {id} is not the harness's virtual display");
        std::process::exit(2);
    }
    id
}

/// Logical top-left for a `w` x `h` window centred on display `id`, in global
/// coordinates (origin at the top left of the main display).
pub fn window_origin(id: u32, w: f64, h: f64) -> (f64, f64) {
    let b = display::bounds(id);
    (b.x + ((b.w - w) / 2.).round(), b.y + ((b.h - h) / 2.).round())
}

/// CoreGraphics: displays, the window list and the virtual display shim
/// (src/vdisplay.m).
pub mod display {
    use std::ffi::c_void;

    #[repr(C)]
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct Rect {
        pub x: f64,
        pub y: f64,
        pub w: f64,
        pub h: f64,
    }
    impl Rect {
        pub fn contains(&self, o: &Rect) -> bool {
            o.x >= self.x && o.y >= self.y && o.x + o.w <= self.x + self.w && o.y + o.h <= self.y + self.h
        }
        pub fn intersects(&self, o: &Rect) -> bool {
            o.x < self.x + self.w && self.x < o.x + o.w && o.y < self.y + self.h && self.y < o.y + o.h
        }
    }
    impl std::fmt::Display for Rect {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "[{:.0} {:.0} {:.0} {:.0}]", self.x, self.y, self.w, self.h)
        }
    }

    /// Identifies the harness's display (CGDisplayVendorNumber/ModelNumber).
    pub const VENDOR: u32 = 0xdec4;
    pub const PRODUCT: u32 = 0x0b01;

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> *const c_void;
        fn CGRectMakeWithDictionaryRepresentation(dict: *const c_void, rect: *mut Rect) -> bool;
        fn CGDisplayBounds(id: u32) -> Rect;
        fn CGMainDisplayID() -> u32;
        fn CGGetActiveDisplayList(max: u32, ids: *mut u32, count: *mut u32) -> i32;
        fn CGDisplayVendorNumber(id: u32) -> u32;
        fn CGDisplayModelNumber(id: u32) -> u32;
        fn CGDisplayIsBuiltin(id: u32) -> u32;
        fn CGDisplayCopyDisplayMode(id: u32) -> *const c_void;
        fn CGDisplayModeGetWidth(mode: *const c_void) -> usize;
        fn CGDisplayModeGetHeight(mode: *const c_void) -> usize;
        fn CGDisplayModeGetPixelWidth(mode: *const c_void) -> usize;
        fn CGDisplayModeGetPixelHeight(mode: *const c_void) -> usize;
        fn CGDisplayModeGetRefreshRate(mode: *const c_void) -> f64;
        fn CGDisplayModeRelease(mode: *const c_void);
        static kCGWindowOwnerPID: *const c_void;
        static kCGWindowNumber: *const c_void;
        static kCGWindowLayer: *const c_void;
        static kCGWindowBounds: *const c_void;
        static kCGWindowOwnerName: *const c_void;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFArrayGetCount(array: *const c_void) -> isize;
        fn CFArrayGetValueAtIndex(array: *const c_void, index: isize) -> *const c_void;
        fn CFDictionaryGetValue(dict: *const c_void, key: *const c_void) -> *const c_void;
        fn CFNumberGetValue(number: *const c_void, kind: isize, out: *mut c_void) -> bool;
        fn CFStringGetCString(s: *const c_void, buf: *mut u8, len: isize, encoding: u32) -> bool;
        fn CFRelease(object: *const c_void);
    }
    unsafe extern "C" {
        fn cmp_vd_create(
            width_pt: u32,
            height_pt: u32,
            scale: u32,
            refresh: f64,
            ppi: f64,
            vendor: u32,
            product: u32,
            serial: u32,
            name: *const std::ffi::c_char,
        ) -> u32;
        fn cmp_vd_destroy() -> i32;
    }

    pub fn bounds(id: u32) -> Rect {
        // SAFETY: plain CoreGraphics query; an unknown id yields a zero rect.
        unsafe { CGDisplayBounds(id) }
    }
    pub fn main_id() -> u32 {
        // SAFETY: plain CoreGraphics query.
        unsafe { CGMainDisplayID() }
    }
    pub fn active() -> Vec<u32> {
        let mut ids = [0u32; 32];
        let mut n = 0u32;
        // SAFETY: CoreGraphics writes at most 32 ids and the count.
        let rc = unsafe { CGGetActiveDisplayList(32, ids.as_mut_ptr(), &mut n) };
        if rc != 0 {
            return vec![];
        }
        ids[..n as usize].to_vec()
    }
    pub fn is_harness_display(id: u32) -> bool {
        // SAFETY: plain CoreGraphics queries.
        unsafe { CGDisplayVendorNumber(id) == VENDOR && CGDisplayModelNumber(id) == PRODUCT && CGDisplayIsBuiltin(id) == 0 }
    }
    /// Every active display except the harness's: the ones a person can see.
    pub fn physical() -> Vec<u32> {
        active().into_iter().filter(|&id| !is_harness_display(id)).collect()
    }

    #[derive(Clone, Copy, Debug)]
    pub struct Mode {
        pub width: usize,
        pub height: usize,
        pub pixel_width: usize,
        pub pixel_height: usize,
        pub refresh: f64,
    }
    pub fn mode(id: u32) -> Option<Mode> {
        // SAFETY: the copied mode is released after reading it.
        unsafe {
            let m = CGDisplayCopyDisplayMode(id);
            if m.is_null() {
                return None;
            }
            let mode = Mode {
                width: CGDisplayModeGetWidth(m),
                height: CGDisplayModeGetHeight(m),
                pixel_width: CGDisplayModeGetPixelWidth(m),
                pixel_height: CGDisplayModeGetPixelHeight(m),
                refresh: CGDisplayModeGetRefreshRate(m),
            };
            CGDisplayModeRelease(m);
            Some(mode)
        }
    }

    #[derive(Clone, Debug)]
    pub struct WindowInfo {
        pub pid: i32,
        pub number: i32,
        pub layer: i32,
        pub bounds: Rect,
        pub owner: String,
    }
    /// What the window server lists as on screen (any display), front to back.
    pub fn on_screen_windows() -> Vec<WindowInfo> {
        const ON_SCREEN_ONLY: u32 = 1;
        const SINT32: isize = 3;
        let mut out = vec![];
        // SAFETY: CoreFoundation calls on the copied array (released below)
        // and values borrowed from it; null-checked.
        unsafe {
            let list = CGWindowListCopyWindowInfo(ON_SCREEN_ONLY, 0);
            if list.is_null() {
                return out;
            }
            let int = |dict: *const c_void, key: *const c_void| {
                let n = CFDictionaryGetValue(dict, key);
                let mut v: i32 = -1;
                if !n.is_null() {
                    CFNumberGetValue(n, SINT32, (&raw mut v).cast());
                }
                v
            };
            for i in 0..CFArrayGetCount(list) {
                let dict = CFArrayGetValueAtIndex(list, i);
                let mut bounds = Rect::default();
                let b = CFDictionaryGetValue(dict, kCGWindowBounds);
                if !b.is_null() {
                    CGRectMakeWithDictionaryRepresentation(b, &mut bounds);
                }
                let mut name = [0u8; 256];
                let s = CFDictionaryGetValue(dict, kCGWindowOwnerName);
                let owner = if !s.is_null() && CFStringGetCString(s, name.as_mut_ptr(), 256, 0x0800_0100) {
                    std::ffi::CStr::from_bytes_until_nul(&name)
                        .map(|c| c.to_string_lossy().into_owned())
                        .unwrap_or_default()
                } else {
                    String::new()
                };
                out.push(WindowInfo {
                    pid: int(dict, kCGWindowOwnerPID),
                    number: int(dict, kCGWindowNumber),
                    layer: int(dict, kCGWindowLayer),
                    bounds,
                    owner,
                });
            }
            CFRelease(list);
        }
        out
    }

    /// A window passes when it lies wholly on display `virt` and touches no
    /// display a person can see.
    pub fn check_window(w: &Rect, virt: u32) -> Result<(), String> {
        for id in physical() {
            let p = bounds(id);
            if p.intersects(w) {
                return Err(format!("window {w} overlaps physical display {id} {p}"));
            }
        }
        let v = bounds(virt);
        if !v.contains(w) {
            return Err(format!("window {w} is not inside virtual display {virt} {v}"));
        }
        Ok(())
    }

    /// A virtual display for the life of this value; dropping it removes the
    /// display (and so does the process exiting, however it exits).
    pub struct Virtual {
        pub id: u32,
    }
    impl Virtual {
        /// `width` x `height` points at `scale` pixels per point.
        pub fn create(width: u32, height: u32, scale: u32, refresh: f64) -> Result<Self, String> {
            let name = c"deka measurement display";
            // SAFETY: the shim copies the name; it keeps the display object.
            let id = unsafe {
                cmp_vd_create(width, height, scale, refresh, 218., VENDOR, PRODUCT, std::process::id(), name.as_ptr())
            };
            if id == 0 {
                return Err("CGVirtualDisplay could not be created (see stderr)".into());
            }
            Ok(Self { id })
        }
    }
    /// Remove the display now. Returns milliseconds until it went offline
    /// (-1: still online after 5 s). Safe to call more than once.
    pub fn destroy() -> i32 {
        // SAFETY: the shim releases its display object, if any, and polls.
        unsafe { cmp_vd_destroy() }
    }
    impl Drop for Virtual {
        fn drop(&mut self) {
            destroy();
        }
    }
}

// ---------------------------------------------------------------------------
// The protocol both binaries follow, driven by one call per presented frame.

pub struct Protocol {
    backend: &'static str,
    app: String,
    warmup: u64,
    frames: u64,
    idle: Option<f64>,
    start: Mutex<Option<(Instant, Usage)>>,
}

pub static PRESENTED: AtomicU64 = AtomicU64::new(0);

impl Protocol {
    pub fn new(backend: &'static str) -> &'static Self {
        mark("main");
        require_virtual_display();
        start_visibility_probe();
        let p = Box::leak(Box::new(Self {
            backend,
            app: app_name(),
            warmup: arg("--warmup").and_then(|v| v.parse().ok()).unwrap_or(30),
            frames: arg("--frames").and_then(|v| v.parse().ok()).unwrap_or(240),
            idle: arg("--idle").and_then(|v| v.parse().ok()),
            start: Mutex::new(None),
        }));
        // Nothing hangs: every run ends by itself.
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_secs(60));
            eprintln!("watchdog: 60 s, exiting");
            std::process::exit(2);
        });
        p
    }

    /// Call after every presented (GPUI: painted) frame.
    pub fn frame(&'static self) {
        let n = PRESENTED.fetch_add(1, Ordering::Relaxed) + 1;
        if n == 1 {
            mark("first frame");
            if flag("--first-frame") {
                // Report once the window server shows the window too.
                std::thread::spawn(move || {
                    let t0 = Instant::now();
                    while !ON_SCREEN.load(Ordering::Relaxed) && t0.elapsed() < Duration::from_secs(3) {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    self.report_start();
                    std::process::exit(0);
                });
            }
            if let Some(secs) = self.idle {
                std::thread::spawn(move || self.idle(secs));
            }
        }
        if self.idle.is_some() || flag("--first-frame") {
            return;
        }
        if n == self.warmup {
            if let Ok(mut s) = self.start.lock() {
                *s = Some((Instant::now(), usage()));
            }
        }
        if n == self.warmup + self.frames {
            self.report_animation();
            std::process::exit(0);
        }
    }

    fn report_start(&self) {
        let first = timeline_ms("first frame").unwrap_or(f64::NAN);
        let screen = timeline_ms("window on screen").unwrap_or(f64::NAN);
        let main = timeline_ms("main").unwrap_or(f64::NAN);
        let u = usage();
        println!(
            "RESULT start backend={} app={} main_ms={main:.1} first_frame_ms={first:.1} window_on_screen_ms={screen:.1} visible_with_frame_ms={:.1} footprint_mb={:.1} gpu_mb={:.1} load=[{}] {}",
            self.backend,
            self.app,
            first.max(screen),
            mb(u.footprint),
            gpu_mb(),
            load_avg(),
            placement()
        );
        println!("{}", trace_line());
    }

    fn idle(&self, secs: f64) {
        // Let start-up settle, then measure a quiet window.
        std::thread::sleep(Duration::from_millis(500));
        let (u0, n0, t0) = (usage(), PRESENTED.load(Ordering::Relaxed), Instant::now());
        std::thread::sleep(Duration::from_secs_f64(secs));
        let u1 = usage();
        let wall = t0.elapsed().as_secs_f64();
        println!(
            "RESULT idle backend={} app={} secs={wall:.1} redraws={} cpu_pct={:.3} footprint_mb={:.1} resident_mb={:.1} gpu_mb={:.1} load=[{}] {}",
            self.backend,
            self.app,
            PRESENTED.load(Ordering::Relaxed) - n0,
            (u1.cpu_ns - u0.cpu_ns) as f64 / 1e9 / wall * 100.,
            mb(u1.footprint),
            mb(u1.resident),
            gpu_mb(),
            load_avg(),
            placement()
        );
        std::process::exit(0);
    }

    fn report_animation(&self) {
        let Some((t0, u0)) = self.start.lock().ok().and_then(|s| *s) else {
            return;
        };
        let wall = t0.elapsed().as_secs_f64();
        let u1 = usage();
        println!(
            "RESULT animate backend={} app={} frames={} fps={:.1} cpu_pct={:.1} kernel_pct={:.1} footprint_mb={:.1} resident_mb={:.1} gpu_mb={:.1} load=[{}] {}",
            self.backend,
            self.app,
            self.frames,
            self.frames as f64 / wall,
            (u1.cpu_ns - u0.cpu_ns) as f64 / 1e9 / wall * 100.,
            (u1.sys_ns - u0.sys_ns) as f64 / 1e9 / wall * 100.,
            mb(u1.footprint),
            mb(u1.resident),
            gpu_mb(),
            load_avg(),
            placement()
        );
    }
}

// ---------------------------------------------------------------------------
// Applications

/// A static, text-heavy settings page (no animation: an idle window must not redraw).
pub struct Settings;
fn styled(classes: &str) -> Style {
    let mut style = Style::default();
    deka_native_ir::apply_classes(&mut style, classes).expect("classes");
    style
}
fn node(id: &str, classes: &str, text: Option<&str>, children: Vec<Node>) -> Node {
    Node {
        id: id.into(),
        style: styled(classes),
        text: text.map(Into::into),
        on_click: None,
        children,
    }
}
impl Application for Settings {
    fn initial_state(&self) -> Vec<f64> {
        vec![0.]
    }
    fn render(&self, state: &[f64]) -> Node {
        let rows = (0..14)
            .map(|i| {
                let mut toggle = node(
                    &format!("toggle{i}"),
                    "px-3 py-1 rounded bg-[#1A1611] text-[#F3EFE3]",
                    Some(if state[0] as usize == i { "On" } else { "Off" }),
                    vec![],
                );
                toggle.on_click = Some(i);
                node(
                    &format!("row{i}"),
                    if i % 2 == 0 {
                        "flex-row justify-between items-center px-3 py-2 rounded bg-[#F4F1EA]"
                    } else {
                        "flex-row justify-between items-center px-3 py-2 rounded bg-[#FFFFFF]"
                    },
                    None,
                    vec![
                        node(
                            &format!("label{i}"),
                            "text-sm text-[#222222]",
                            Some(&format!(
                                "Setting {i}: synchronise the workspace when the machine is idle"
                            )),
                            vec![],
                        ),
                        toggle,
                    ],
                )
            })
            .collect();
        node(
            "root",
            "p-6 gap-2 bg-[#FFFFFF]",
            None,
            vec![
                node("title", "text-2xl text-[#1A1611]", Some("Settings"), vec![]),
                node("rows", "gap-1", None, rows),
            ],
        )
    }
    fn event(&self, handler: usize, state: &mut [f64]) {
        state[0] = handler as f64;
    }
}

/// A DekaScript component compiled and run by deka's VM, as `dvm-ui` does.
pub fn vm_app(source: &str, entry: &str) -> deka_vm::ui::VmApp {
    let program = deka_vm::compiler::compile_entry(source, &deka_vm::Hosts::default(), entry)
        .expect("compile");
    deka_vm::ui::VmApp::new(program).expect("vm app")
}
pub const COUNTER: &str = include_str!("../../../crates/deka_vm/examples/counter.dsx");
/// What `deka init` writes as App.dsx.
pub const INIT_APP: &str = "export fn App() { let count = 0; return <view className=\"p-6 gap-4\"><p>Deka native app</p><p>Count: {count}</p><button onClick={fn() { count += 1; }}>Add one</button></view>; }\n";

// ---------------------------------------------------------------------------
// Scenes for the pixel comparison: each is one deka `Scene`, built once, that
// both backends paint. (logical width, height, scale) per scene.

pub const SCENES: [&str; 11] = [
    "counter",
    "counter-focused",
    "init-app",
    "settings",
    "world-title",
    "world-town",
    "world-room",
    "text-fallback",
    "text-sizes",
    "transforms",
    "paints",
];

fn render_app<A: Application>(app: A, w: f32, h: f32, clicks: &[usize], focus: Option<usize>) -> Scene {
    let mut host = deka_native_ui::Host::new(app);
    for &c in clicks {
        host.click(c);
    }
    let renderer = Renderer::new();
    let mut scene = renderer.render_at(&host.render(), w, h, 2., 0., true);
    if let Some(i) = focus {
        let id = scene.targets[i].id.clone();
        scene.focus_ring(&id);
    }
    scene
}

fn world_scene(steps: &[(&str, f64)]) -> Scene {
    let mut world = World::new();
    world.start();
    world.set_muted(true);
    let mut t = 0.;
    world.frame(960., 640., t, true);
    for (key, ms) in steps {
        if !key.is_empty() {
            world.key(key, true);
        }
        let end = t + ms;
        while t < end {
            t += 1000. / 60.;
            world.frame(960., 640., t, true);
        }
        if !key.is_empty() {
            world.key(key, false);
        }
    }
    world.frame(960., 640., t, true)
}

pub fn scene(name: &str) -> (Scene, f32) {
    let column = |children: Vec<Node>| node("root", "p-4 gap-3 bg-[#FFFFFF]", None, children);
    let scene = match name {
        "counter" => render_app(vm_app(COUNTER, "Counter"), 560., 300., &[0, 0], None),
        "counter-focused" => render_app(vm_app(COUNTER, "Counter"), 560., 300., &[0], Some(0)),
        "init-app" => render_app(vm_app(INIT_APP, "App"), 560., 300., &[], None),
        "settings" => render_app(Settings, 960., 640., &[3], None),
        "world-title" => {
            let mut world = World::new();
            world.set_muted(true);
            world.frame(960., 640., 0., true)
        }
        "world-town" => world_scene(&[("right", 900.), ("down", 600.), ("", 100.)]),
        "world-room" => world_scene(&[("up", 400.), ("enter", 100.), ("", 1500.)]),
        "text-fallback" => Renderer::new().render(
            &column(vec![
                node("ja", "text-2xl text-[#1A1611]", Some("日本語のテキスト"), vec![]),
                node("emoji", "text-2xl", Some("Emoji 🎉 👋 ✅"), vec![]),
                node("mixed", "text-sm text-[#333333]", Some("Grüße, Ελληνικά, Кириллица, עברית"), vec![]),
            ]),
            480.,
            200.,
            2.,
        ),
        "text-sizes" => Renderer::new().render(
            &column(
                [11., 12., 14., 16., 20., 28.]
                    .iter()
                    .map(|s| {
                        let mut n = node(
                            &format!("s{s}"),
                            "text-[#1A1611]",
                            Some(&format!("{s} px: The quick brown fox jumps over the lazy dog")),
                            vec![],
                        );
                        n.style.font_size = Some(*s);
                        n
                    })
                    .collect(),
            ),
            560.,
            260.,
            2.,
        ),
        "transforms" => {
            let card = |id: &str, color: u32, f: &dyn Fn(&mut Style)| {
                let mut n = node(id, "p-3 text-[#FFFFFF]", Some("Card"), vec![]);
                n.style.width = deka_native_ui::Length::Px(120.);
                n.style.height = deka_native_ui::Length::Px(80.);
                n.style.radius = 10.;
                n.style.background = Some(color);
                n.on_click = Some(0);
                f(&mut n.style);
                n
            };
            Renderer::new().render(
                &node(
                    "root",
                    "flex-row p-8 gap-10 bg-[#F3EFE3]",
                    None,
                    vec![
                        card("a", 0x2a9d8f, &|s| s.rotate = 12.),
                        card("b", 0xe76f51, &|s| {
                            s.scale = 1.2;
                            s.opacity = 0.7;
                        }),
                        card("c", 0x264653, &|s| s.rotate = -30.),
                    ],
                ),
                560.,
                220.,
                2.,
            )
        }
        _ => {
            // Every paint kind with exact numbers: opacity, radius, clip.
            let swatch = |id: &str, color: u32, f: &dyn Fn(&mut Style)| {
                let mut n = node(id, "", None, vec![]);
                n.style.width = deka_native_ui::Length::Px(60.);
                n.style.height = deka_native_ui::Length::Px(60.);
                n.style.background = Some(color);
                f(&mut n.style);
                n
            };
            let mut clip = node("clip", "overflow-hidden bg-[#EEEEEE]", None, vec![]);
            clip.style.width = deka_native_ui::Length::Px(200.);
            clip.style.height = deka_native_ui::Length::Px(40.);
            let mut inner = node(
                "inner",
                "px-2 text-[#1A1611] whitespace-nowrap",
                Some("Clipped text that runs far past the edge of its box"),
                vec![],
            );
            inner.style.width = deka_native_ui::Length::Px(400.);
            clip.children = vec![inner];
            let root = node(
                "root",
                "p-6 gap-4 bg-[#FFFFFF]",
                None,
                vec![
                    node(
                        "r1",
                        "flex-row gap-4",
                        None,
                        vec![
                            swatch("s1", 0xff0000, &|_| {}),
                            swatch("s2", 0x0000ff, &|s| s.opacity = 0.5),
                            swatch("s3", 0x000000, &|s| s.radius = 30.),
                            swatch("s4", 0x2a9d8f, &|s| s.radius = 12.),
                        ],
                    ),
                    clip,
                ],
            );
            Renderer::new().render(&root, 400., 220., 2.)
        }
    };
    (scene, 2.)
}

pub fn write_png(path: &std::path::Path, width: u32, height: u32, rgba: &[u8]) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let file = std::fs::File::create(path).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .and_then(|mut w| w.write_image_data(rgba))
        .expect("write png");
}

pub fn read_png(path: &std::path::Path) -> Option<(u32, u32, Vec<u8>)> {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).ok()?));
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    Some((info.width, info.height, buf))
}

// ---------------------------------------------------------------------------
// Pixel comparison

/// Compare `<scene>-gpui.png` with `<scene>-new.png` in `dir`; write
/// `<scene>-side-by-side.png` (GPUI | new | difference x8) and print one row
/// per scene.
pub fn compare(dir: &std::path::Path) {
    println!(
        "| scene | size | identical | differ > 8 | max channel delta | mean delta | PSNR dB |\n|---|---|---|---|---|---|---|"
    );
    for name in SCENES {
        let (Some(a), Some(b)) = (
            read_png(&dir.join(format!("{name}-gpui.png"))),
            read_png(&dir.join(format!("{name}-new.png"))),
        ) else {
            println!("| {name} | missing | | | | | |");
            continue;
        };
        if (a.0, a.1) != (b.0, b.1) {
            println!("| {name} | {}x{} vs {}x{} | size differs | | | | |", a.0, a.1, b.0, b.1);
            continue;
        }
        let (w, h) = (a.0, a.1);
        let mut identical = 0usize;
        let mut differ = 0usize;
        let mut max = 0u8;
        let mut sum = 0f64;
        let mut sq = 0f64;
        let mut diff = Vec::with_capacity(a.2.len());
        for (pa, pb) in a.2.chunks_exact(4).zip(b.2.chunks_exact(4)) {
            let d = pa[..3]
                .iter()
                .zip(&pb[..3])
                .map(|(x, y)| x.abs_diff(*y))
                .max()
                .unwrap_or(0);
            for (x, y) in pa[..3].iter().zip(&pb[..3]) {
                let e = f64::from(*x) - f64::from(*y);
                sum += e.abs();
                sq += e * e;
            }
            identical += usize::from(d == 0);
            differ += usize::from(d > 8);
            max = max.max(d);
            let v = 255u8.saturating_sub(d.saturating_mul(8));
            diff.extend_from_slice(&[255, v, v, 255]);
        }
        let n = (w * h) as f64;
        let mse = sq / (n * 3.);
        let psnr = if mse == 0. { f64::INFINITY } else { 10. * (255f64 * 255. / mse).log10() };
        println!(
            "| {name} | {w}x{h} | {:.2}% | {:.3}% | {max} | {:.3} | {psnr:.1} |",
            identical as f64 / n * 100.,
            differ as f64 / n * 100.,
            sum / (n * 3.),
        );
        // GPUI | new | difference, separated by 8 px gutters.
        let gap = 8;
        let sw = w * 3 + gap * 2;
        let mut side = vec![255u8; (sw * h * 4) as usize];
        for (i, img) in [&a.2, &b.2, &diff].into_iter().enumerate() {
            let x0 = i as u32 * (w + gap);
            for y in 0..h {
                let src = (y * w * 4) as usize;
                let dst = ((y * sw + x0) * 4) as usize;
                side[dst..dst + (w * 4) as usize].copy_from_slice(&img[src..src + (w * 4) as usize]);
            }
        }
        write_png(&dir.join(format!("{name}-side-by-side.png")), sw, h, &side);
    }
}

/// `(x, y, w, h)` of `dir/<scene>-{gpui,new}.png` scaled `zoom` times with
/// nearest-neighbour, stacked GPUI above new, as `<scene>-zoom.png`.
pub fn zoom(dir: &std::path::Path, name: &str, rect: (u32, u32, u32, u32), zoom: u32) {
    let (x, y, w, h) = rect;
    let mut out = vec![255u8; (w * zoom * (h * zoom * 2 + 8) * 4) as usize];
    for (i, backend) in ["gpui", "new"].into_iter().enumerate() {
        let Some((iw, _, px)) = read_png(&dir.join(format!("{name}-{backend}.png"))) else {
            return;
        };
        let y0 = i as u32 * (h * zoom + 8);
        for oy in 0..h * zoom {
            for ox in 0..w * zoom {
                let s = (((y + oy / zoom) * iw + x + ox / zoom) * 4) as usize;
                let d = (((y0 + oy) * w * zoom + ox) * 4) as usize;
                out[d..d + 4].copy_from_slice(&px[s..s + 4]);
            }
        }
    }
    write_png(&dir.join(format!("{name}-zoom.png")), w * zoom, h * zoom * 2 + 8, &out);
}
