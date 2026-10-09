//! Cargo's native Rust UI development loop. Apps own markup patches; this
//! supervisor owns Cargo and the child process for compiled-code changes.
use deka_ui_hot_reload::{
    RESTART_PREFIX,
    files::{Change, Files, classify, snapshot, source_roots},
};
use std::io::BufRead;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};
fn main() {
    if let Err(error) = run() {
        eprintln!("deka dev: {error}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), String> {
    let mut arguments: Vec<_> = std::env::args().skip(1).collect();
    if arguments.first().is_some_and(|arg| arg == "deka") {
        arguments.remove(0);
    }
    if arguments.first().is_none_or(|arg| arg != "dev") {
        return Err("usage: cargo deka dev [cargo build options] [-- app arguments]".into());
    }
    arguments.remove(0);
    let split = arguments
        .iter()
        .position(|arg| arg == "--")
        .unwrap_or(arguments.len());
    let app_args = arguments.get(split + 1..).unwrap_or_default().to_vec();
    arguments.truncate(split);
    if arguments.iter().any(|arg| {
        arg == "--release"
            || arg == "--profile"
            || arg.starts_with("--profile=")
            || arg == "--target"
            || arg.starts_with("--target=")
    }) {
        return Err("hot reload requires the native debug profile".into());
    }
    let mut roots = source_roots(&arguments)?;
    arguments.extend(["--features".into(), "deka-ui/hot-reload".into()]);
    let stopped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let interrupt = stopped.clone();
    ctrlc::set_handler(move || interrupt.store(true, std::sync::atomic::Ordering::Relaxed))
        .map_err(|e| e.to_string())?;
    let mut previous = snapshot(&roots)?;
    let binary = build_current(&arguments, &mut roots, &mut previous)?;
    let mut baseline = previous.clone();
    let mut must_rebuild = false;
    let mut child = Running::new(&binary, &app_args)?;
    let mut pending: Option<(Files, std::time::Instant)> = None;
    eprintln!(
        "deka dev: watching native Rust UI; markup preserves state, compiled code rebuilds/restarts"
    );
    loop {
        if stopped.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
        if let Some(status) = child.child.try_wait().map_err(|e| e.to_string())? {
            return if status.success() {
                Ok(())
            } else {
                Err(format!("application exited: {status}"))
            };
        }
        let next = snapshot(&roots)?;
        let requested = child.rebuild.swap(false, Ordering::Relaxed);
        if next == previous && !requested {
            continue;
        }
        if !requested {
            match &pending {
                Some((files, observed))
                    if files == &next && observed.elapsed() >= Duration::from_millis(20) => {}
                _ => {
                    pending = Some((next, std::time::Instant::now()));
                    continue;
                }
            }
        }
        pending = None;
        let change = if must_rebuild || requested {
            Change::Restart(if requested {
                "application requested a rebuild".into()
            } else {
                "recovering a failed build".into()
            })
        } else {
            classify(&baseline, &next)
        };
        match change {
            Change::Markup => {
                baseline = next.clone();
                previous = next;
            }
            Change::Invalid(error) => {
                eprintln!("deka dev: {error}; keeping last good UI");
                previous = next;
            }
            Change::Restart(reason) => {
                eprintln!("deka dev: {reason}; rebuilding and restarting (signal state resets)");
                previous = next;
                must_rebuild = true;
                match build_current(&arguments, &mut roots, &mut previous) {
                    Ok(binary) => {
                        child.stop()?;
                        child = Running::new(&binary, &app_args)?;
                        baseline = previous.clone();
                        must_rebuild = false;
                        eprintln!("deka dev: restarted application");
                    }
                    Err(error) => eprintln!(
                        "deka dev: {error}; old application retained until the next successful build"
                    ),
                }
            }
        }
    }
}
// Refresh local dependency roots after manifest changes and rebuild saves made
// during compilation. An error returns to the supervisor with the old child alive.
fn build_current(
    arguments: &[String],
    roots: &mut Vec<PathBuf>,
    previous: &mut Files,
) -> Result<PathBuf, String> {
    loop {
        *roots = source_roots(arguments)?;
        *previous = snapshot(roots)?;
        let binary = build(arguments)?;
        if snapshot(roots)? == *previous {
            return Ok(binary);
        }
        eprintln!("deka dev: source changed during compilation; rebuilding before restart");
    }
}
struct Running {
    child: Child,
    rebuild: Arc<AtomicBool>,
}
impl Running {
    fn new(binary: &Path, args: &[String]) -> Result<Self, String> {
        let mut child = Command::new(binary)
            .args(args)
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        let stderr = child.stderr.take().ok_or("application stderr missing")?;
        let rebuild = Arc::new(AtomicBool::new(false));
        let requested = rebuild.clone();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(stderr).lines() {
                let Ok(line) = line else {
                    break;
                };
                if line.starts_with(RESTART_PREFIX) {
                    requested.store(true, Ordering::Relaxed);
                }
                eprintln!("{line}");
            }
        });
        Ok(Self { child, rebuild })
    }
    fn stop(&mut self) -> Result<(), String> {
        if self.child.try_wait().map_err(|e| e.to_string())?.is_none() {
            self.child.kill().map_err(|e| e.to_string())?;
        }
        self.child.wait().map_err(|e| e.to_string())?;
        Ok(())
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
fn build(arguments: &[String]) -> Result<PathBuf, String> {
    let output = Command::new("cargo")
        .args(["build", "--message-format=json-render-diagnostics"])
        .args(arguments)
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| e.to_string())?;
    let mut executables = vec![];
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if let Ok(message) = serde_json::from_str::<serde_json::Value>(line) {
            if message["reason"] == "compiler-message"
                && let Some(rendered) = message["message"]["rendered"].as_str()
            {
                eprint!("{rendered}");
            }
            if let Some(executable) = message["executable"].as_str() {
                executables.push(PathBuf::from(executable));
            }
        }
    }
    if !output.status.success() {
        return Err(format!("Cargo build failed: {}", output.status));
    }
    if executables.len() != 1 {
        return Err("select one executable with --bin NAME or --example NAME".into());
    }
    Ok(executables.remove(0))
}
