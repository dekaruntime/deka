//! Runs a measurement batch with every window on a virtual display nobody
//! looks at. Creates the display (the physical main display's size, backing
//! scale and refresh), runs each measurement as a child process that opens its
//! window there, watches the window server for the whole run and fails the
//! batch the moment any child window touches a physical display, then removes
//! the display. Children are killed before the display goes, on success, on
//! Ctrl-C (SIGINT/SIGTERM/SIGHUP) and on panic.
//!
//! cmp-batch --bin DIR --out DIR [--rounds N] [--plan start,idle,animate,pixels]
//!           [--backends gpui,new] [--apps counter,settings,world]
use native_backend_compare::display::{self, Rect, Virtual};
use native_backend_compare::arg;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::time::{Duration, Instant};

static STOP: AtomicBool = AtomicBool::new(false);
static CHILD: AtomicI32 = AtomicI32::new(0);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

/// Kill the running child (if any) and remove the display, in that order.
fn cleanup() {
    kill_child();
    display::destroy();
}

fn kill_child() {
    let pid = CHILD.swap(0, Ordering::SeqCst);
    if pid > 0 {
        // SAFETY: signalling and reaping our own child process by pid.
        unsafe {
            libc::kill(pid, libc::SIGKILL);
            let mut status = 0;
            libc::waitpid(pid, &mut status, 0);
        }
    }
}

struct Run {
    output: String,
    windows: Vec<(i32, Rect)>,
    violation: Option<String>,
    status: Option<i32>,
}

/// Run one child to completion, polling its windows every 10 ms.
fn run(bin: &Path, args: &[&str], virt: u32) -> Run {
    let mut child = match Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return Run {
                output: format!("spawn {}: {e}\n", bin.display()),
                windows: vec![],
                violation: None,
                status: None,
            };
        }
    };
    let pid = child.id() as i32;
    CHILD.store(pid, Ordering::SeqCst);
    let pipe = |r: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut s = String::new();
            if let Some(mut r) = r {
                let _ = r.read_to_string(&mut s);
            }
            s
        })
    };
    let out = pipe(child.stdout.take().map(|r| Box::new(r) as Box<dyn Read + Send>));
    let err = pipe(child.stderr.take().map(|r| Box::new(r) as Box<dyn Read + Send>));
    let t0 = Instant::now();
    let mut windows: Vec<(i32, Rect)> = vec![];
    let mut violation = None;
    let status = loop {
        for w in display::on_screen_windows().into_iter().filter(|w| w.pid == pid) {
            if !windows.iter().any(|(n, b)| *n == w.number && *b == w.bounds) {
                windows.push((w.number, w.bounds));
            }
            if violation.is_none()
                && let Err(e) = display::check_window(&w.bounds, virt)
            {
                violation = Some(e);
            }
        }
        if violation.is_some() || STOP.load(Ordering::SeqCst) || t0.elapsed() > Duration::from_secs(90) {
            let _ = child.kill();
            break child.wait().ok().and_then(|s| s.code());
        }
        match child.try_wait() {
            Ok(Some(s)) => break s.code(),
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => break None,
        }
    };
    CHILD.store(0, Ordering::SeqCst);
    let mut output = out.join().unwrap_or_default();
    output.push_str(&err.join().unwrap_or_default());
    Run {
        output,
        windows,
        violation,
        status,
    }
}

/// Other applications' windows: (owner, number) -> bounds.
fn others() -> Vec<(String, i32, Rect)> {
    let me = std::process::id() as i32;
    display::on_screen_windows()
        .into_iter()
        .filter(|w| w.pid != me)
        .map(|w| (w.owner, w.number, w.bounds))
        .collect()
}

fn moved(before: &[(String, i32, Rect)], after: &[(String, i32, Rect)]) -> Vec<String> {
    before
        .iter()
        .filter_map(|(owner, n, b)| {
            let a = after.iter().find(|(_, m, _)| m == n)?;
            (a.2 != *b).then(|| format!("{owner} #{n} {b} -> {}", a.2))
        })
        .collect()
}

fn main() {
    let bin = PathBuf::from(arg("--bin").expect("--bin DIR (the cmp-gpui/cmp-new build directory)"));
    let out = PathBuf::from(arg("--out").expect("--out DIR"));
    let rounds: usize = arg("--rounds").and_then(|v| v.parse().ok()).unwrap_or(5);
    let plan = arg("--plan").unwrap_or_else(|| "start,idle,animate,pixels".into());
    let backends = arg("--backends").unwrap_or_else(|| "gpui,new".into());
    let apps = arg("--apps").unwrap_or_else(|| "counter,settings,world".into());
    let backends: Vec<&str> = backends.split(',').collect();
    let apps: Vec<&str> = apps.split(',').collect();
    let _ = std::fs::create_dir_all(&out);
    let mut log = std::fs::File::create(out.join("batch.log")).expect("batch.log");
    macro_rules! say {
        ($($t:tt)*) => {{
            let line = format!($($t)*);
            println!("{line}");
            let _ = writeln!(log, "{line}");
        }};
    }

    // SAFETY: installing a handler that only stores to an atomic.
    unsafe {
        for sig in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
            libc::signal(sig, on_signal as *const () as libc::sighandler_t);
        }
    }
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        cleanup();
        default_hook(info);
    }));

    let main = display::main_id();
    let Some(m) = display::mode(main) else {
        eprintln!("no mode for the main display");
        std::process::exit(1);
    };
    let scale = (m.pixel_width / m.width.max(1)) as u32;
    // Built-in panels report 0 Hz; they run at 60.
    let refresh = if m.refresh > 0. { m.refresh } else { 60. };
    say!(
        "main display {main}: {}x{} pt, {}x{} px (scale {scale}), {refresh} Hz, bounds {}",
        m.width,
        m.height,
        m.pixel_width,
        m.pixel_height,
        display::bounds(main)
    );
    let physical_before: Vec<(u32, Rect)> = display::physical().into_iter().map(|id| (id, display::bounds(id))).collect();
    let windows_before = others();

    let t = Instant::now();
    let virt = match Virtual::create(m.width as u32, m.height as u32, scale, refresh) {
        Ok(v) => v,
        Err(e) => {
            say!("FAIL {e}");
            std::process::exit(1);
        }
    };
    let vm = display::mode(virt.id);
    say!(
        "virtual display {} online in {} ms: bounds {}, mode {:?}, main display still {}",
        virt.id,
        t.elapsed().as_millis(),
        display::bounds(virt.id),
        vm,
        display::main_id()
    );
    if vm.is_none_or(|v| v.pixel_width != m.pixel_width || v.width != m.width) {
        say!("FAIL the virtual display's mode does not match the main display's");
        drop(virt);
        std::process::exit(1);
    }
    std::thread::sleep(Duration::from_millis(1000));
    say!("main display mode with the virtual display: {:?}", display::mode(main));
    let physical_during: Vec<(u32, Rect)> = display::physical().into_iter().map(|id| (id, display::bounds(id))).collect();
    let windows_during = others();
    say!(
        "physical displays before {physical_before:?}, with the virtual display {physical_during:?}"
    );
    let shifted = moved(&windows_before, &windows_during);
    say!("other apps' windows moved by adding the display: {}", shifted.len());
    for s in &shifted {
        say!("  moved: {s}");
    }

    let id = virt.id.to_string();
    let mut failed = false;
    let mut results = vec![];
    let mut exec = |backend: &str, args: &[&str]| -> bool {
        if STOP.load(Ordering::SeqCst) {
            return false;
        }
        let mut full: Vec<&str> = args.to_vec();
        full.extend(["--display", &id]);
        let path = bin.join(format!("cmp-{backend}"));
        let r = run(&path, &full, virt.id);
        let _ = writeln!(log, "## {} {}\n{}exit {:?}", path.display(), full.join(" "), r.output, r.status);
        for (n, b) in &r.windows {
            let _ = writeln!(log, "PLACEMENT window #{n} {b} on virtual display {} {}", virt.id, display::bounds(virt.id));
        }
        let result: Vec<&str> = r.output.lines().filter(|l| l.starts_with("RESULT")).collect();
        results.extend(result.iter().map(|s| s.to_string()));
        if let Some(v) = &r.violation {
            say!("FAIL {backend} {}: {v}", args.join(" "));
            return false;
        }
        if result.iter().any(|l| !l.contains("placement=virtual")) {
            say!("FAIL {backend} {}: the child saw its window off the virtual display", args.join(" "));
            return false;
        }
        let windowed = args.first() != Some(&"snap") || backend == "gpui";
        let needs_result = args.first() != Some(&"snap") && args.first() != Some(&"compare");
        // A start-up run can exit within one poll of its window appearing;
        // then the child's own sighting (placement=virtual) is the evidence.
        let seen = !r.windows.is_empty() || !result.is_empty();
        if r.status != Some(0) || (needs_result && result.is_empty()) || (windowed && !seen) {
            say!(
                "FAIL {backend} {}: exit {:?}, {} RESULT lines, {} windows seen",
                args.join(" "),
                r.status,
                result.len(),
                r.windows.len()
            );
            return false;
        }
        for line in &result {
            println!("{line}");
        }
        std::thread::sleep(Duration::from_secs(1));
        true
    };

    let out_s = out.display().to_string();
    'batch: {
        if plan.contains("start") {
            for _ in 0..rounds {
                for backend in &backends {
                    for app in &apps {
                        if !exec(backend, &["--app", app, "--first-frame"]) {
                            failed = true;
                            break 'batch;
                        }
                    }
                }
            }
        }
        // Each order once, as run-batch.sh did.
        for order in [backends.clone(), backends.iter().rev().copied().collect()] {
            for backend in &order {
                if plan.contains("idle") && !exec(backend, &["--app", "settings", "--idle", "10"]) {
                    failed = true;
                    break 'batch;
                }
                if plan.contains("animate") && !exec(backend, &["--app", "world", "--warmup", "60", "--frames", "600"]) {
                    failed = true;
                    break 'batch;
                }
            }
        }
        if plan.contains("pixels") {
            for scene in native_backend_compare::SCENES {
                if !exec("gpui", &["snap", "--scene", scene, "--out", &out_s]) {
                    failed = true;
                    break 'batch;
                }
            }
            if !exec("new", &["snap", "--out", &out_s]) || !exec("new", &["compare", "--dir", &out_s]) {
                failed = true;
                break 'batch;
            }
        }
    }
    let interrupted = STOP.load(Ordering::SeqCst);
    kill_child();
    let gone_ms = display::destroy();
    drop(virt);
    std::thread::sleep(Duration::from_millis(1000));
    say!("main display mode after: {:?}", display::mode(main));
    let windows_after = others();
    let physical_after: Vec<(u32, Rect)> = display::physical().into_iter().map(|id| (id, display::bounds(id))).collect();
    say!("virtual display removed ({gone_ms} ms until offline; -1 = still online after 5 s); physical displays after {physical_after:?}");
    let shifted = moved(&windows_before, &windows_after);
    say!("other apps' windows moved between before and after the batch: {}", shifted.len());
    for s in &shifted {
        say!("  moved: {s}");
    }
    for r in &results {
        let _ = writeln!(log, "{r}");
    }
    if interrupted {
        say!("INTERRUPTED");
        std::process::exit(130);
    }
    if failed {
        std::process::exit(1);
    }
    say!("OK {} results; every window stayed on virtual display {id}", results.len());
}
