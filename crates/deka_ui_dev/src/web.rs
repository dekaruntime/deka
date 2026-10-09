//! Loopback development transport for the same source classifier/template plans.
use super::*;
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::Mutex,
};
type Assets = BTreeMap<String, Vec<u8>>;
struct Update {
    sequence: u64,
    packet: String,
    initial: String,
}
struct Server {
    assets: Mutex<Assets>,
    update: Mutex<Update>,
    stopped: Arc<AtomicBool>,
    rebuild: AtomicBool,
}
impl Server {
    fn send(&self, packet: serde_json::Value) {
        let mut update = self.update.lock().unwrap();
        update.sequence += 1;
        update.packet = packet.to_string();
        if packet["type"] == "patch" {
            update.initial = update.packet.clone();
        }
        if packet["type"] == "reload" {
            update.initial = empty_patch();
        }
    }
}
fn empty_patch() -> String {
    serde_json::json!({"type":"patch","baseline":{},"current":{}}).to_string()
}
fn option(arguments: &mut Vec<String>, flag: &str) -> Result<Option<String>, String> {
    if let Some(index) = arguments.iter().position(|arg| arg == flag) {
        arguments.remove(index);
        return if index < arguments.len() {
            Ok(Some(arguments.remove(index)))
        } else {
            Err(format!("{flag} requires a value"))
        };
    }
    if let Some(index) = arguments
        .iter()
        .position(|arg| arg.starts_with(&format!("{flag}=")))
    {
        return Ok(Some(arguments.remove(index)[flag.len() + 1..].into()));
    }
    Ok(None)
}
pub(super) fn run(mut arguments: Vec<String>) -> Result<(), String> {
    let port: u16 = option(&mut arguments, "--port")?
        .unwrap_or_else(|| "3000".into())
        .parse()
        .map_err(|e| format!("invalid browser port: {e}"))?;
    let lesson = option(&mut arguments, "--lesson")?;
    if arguments.iter().any(|arg| {
        arg == "--"
            || arg == "--release"
            || arg.starts_with("--profile")
            || arg == "--target"
            || arg.starts_with("--target=")
    }) {
        return Err("browser hot reload requires the debug wasm32-unknown-unknown target; select a cdylib library/example with a start(canvas) export".into());
    }
    arguments.extend([
        "--target".into(),
        "wasm32-unknown-unknown".into(),
        "--features".into(),
        "deka-ui/hot-reload".into(),
    ]);
    let stopped = Arc::new(AtomicBool::new(false));
    let interrupt = stopped.clone();
    ctrlc::set_handler(move || interrupt.store(true, Ordering::Relaxed))
        .map_err(|e| e.to_string())?;
    let mut roots = source_roots(&arguments)?;
    let mut previous = snapshot(&roots)?;
    let wasm = build_current(&arguments, &mut roots, &mut previous)?;
    let mut baseline = previous.clone();
    let assets = bindgen(&wasm, lesson.as_deref())?;
    let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    let address = listener.local_addr().map_err(|e| e.to_string())?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let server = Arc::new(Server {
        assets: Mutex::new(assets),
        update: Mutex::new(Update {
            sequence: 0,
            packet: empty_patch(),
            initial: empty_patch(),
        }),
        stopped: stopped.clone(),
        rebuild: AtomicBool::new(false),
    });
    let serving = server.clone();
    std::thread::spawn(move || {
        while !serving.stopped.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let server = serving.clone();
                    std::thread::spawn(move || {
                        let _ = request(stream, &server);
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(20))
                }
                Err(_) => break,
            }
        }
    });
    println!(
        "deka dev: browser at http://{address}; markup preserves state, compiled code rebuilds/reloads"
    );
    let mut pending: Option<(Files, std::time::Instant)> = None;
    let mut must_rebuild = false;
    let mut changed_paths = std::collections::BTreeSet::new();
    while !stopped.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(50));
        let next = snapshot(&roots)?;
        let requested = server.rebuild.swap(false, Ordering::Relaxed);
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
            Change::Restart("browser requested a rebuild or a failed build needs retrying".into())
        } else {
            classify(&baseline, &next)
        };
        match change {
            Change::Markup => {
                changed_paths.extend(
                    next.iter()
                        .filter(|(path, source)| baseline.get(*path) != Some(*source))
                        .map(|(path, _)| path.clone()),
                );
                let current: Files = changed_paths
                    .iter()
                    .map(|path| (path.clone(), next[path].clone()))
                    .collect();
                let original: Files = current
                    .keys()
                    .map(|path| (path.clone(), baseline[path].clone()))
                    .collect();
                server.send(
                    serde_json::json!({"type":"patch","baseline":original,"current":current}),
                );
                previous = next;
            }
            Change::Invalid(reason) => {
                eprintln!("deka dev: {reason}; keeping last good UI");
                server.send(serde_json::json!({"type":"error","reason":reason}));
                previous = next;
            }
            Change::Restart(reason) => {
                eprintln!("deka dev: {reason}; rebuilding and restarting (signal state resets)");
                server.send(serde_json::json!({"type":"restart","reason":reason}));
                previous = next;
                must_rebuild = true;
                match build_current(&arguments, &mut roots, &mut previous)
                    .and_then(|wasm| bindgen(&wasm, lesson.as_deref()))
                {
                    Ok(assets) => {
                        *server.assets.lock().unwrap() = assets;
                        baseline = previous.clone();
                        must_rebuild = false;
                        changed_paths.clear();
                        server.rebuild.store(false, Ordering::Relaxed);
                        server.send(serde_json::json!({"type":"reload"}));
                    }
                    Err(reason) => {
                        server.send(serde_json::json!({"type":"error","reason":reason}));
                        eprintln!("deka dev: {reason}; keeping last good browser module");
                    }
                }
            }
        }
    }
    Ok(())
}
fn bindgen(wasm: &Path, lesson: Option<&str>) -> Result<Assets, String> {
    let out = wasm
        .parent()
        .ok_or("WASM artifact has no directory")?
        .join("deka-web-pkg");
    let status = Command::new("wasm-bindgen")
        .arg(wasm)
        .args(["--target", "web", "--out-name", "app", "--out-dir"])
        .arg(&out)
        .status()
        .map_err(|e| format!("wasm-bindgen-cli 0.2.128 is required: {e}"))?;
    if !status.success() {
        return Err("wasm-bindgen failed; keeping last good browser module".into());
    }
    fn read(path: &Path, root: &Path, assets: &mut Assets) -> Result<(), String> {
        for entry in std::fs::read_dir(path).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if path.is_dir() {
                read(&path, root, assets)?;
            } else {
                let key = format!(
                    "/pkg/{}",
                    path.strip_prefix(root)
                        .map_err(|e| e.to_string())?
                        .to_string_lossy()
                );
                assets.insert(key, std::fs::read(&path).map_err(|e| e.to_string())?);
            }
        }
        Ok(())
    }
    let mut assets = Assets::new();
    read(&out, &out, &mut assets)?;
    // Bindgen copies directly imported snippets, not their relative imports.
    let hosts: Vec<_> = assets
        .keys()
        .filter(|key| key.ends_with("/hot-host.js") || key.ends_with("/test-host.js"))
        .cloned()
        .collect();
    for key in hosts {
        let directory = key.rsplit_once('/').unwrap().0;
        assets.insert(
            format!("{directory}/host.js"),
            include_bytes!("../../deka_ui/web/host.js").to_vec(),
        );
    }
    let start = lesson.map_or_else(
        || "module.start(canvas)".into(),
        |lesson| {
            format!(
                "module.start({},canvas)",
                serde_json::to_string(lesson).unwrap()
            )
        },
    );
    assets.insert("/".into(), format!(r#"<!doctype html><meta charset="utf-8"><title>Deka Rust UI development</title><div style="position:relative"><canvas style="width:800px;height:600px"></canvas></div><script type="module">import init,* as module from '/pkg/app.js';await init({{module_or_path:'/pkg/app_bg.wasm'}});const canvas=document.querySelector('canvas');{start};</script>"#).into_bytes());
    Ok(assets)
}
fn request(mut stream: TcpStream, server: &Server) -> std::io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let mut header = Vec::new();
    let mut byte = [0];
    while header.len() < 16_384 && !header.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte)? == 0 {
            return Ok(());
        }
        header.push(byte[0]);
    }
    let header = String::from_utf8_lossy(&header);
    let mut request = header.lines().next().unwrap_or_default().split_whitespace();
    let method = request.next().unwrap_or_default();
    let path = request
        .next()
        .unwrap_or_default()
        .split('?')
        .next()
        .unwrap_or_default();
    if method == "GET" && path == "/favicon.ico" {
        return stream.write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n");
    }
    if method == "GET" && path == "/__deka/events" {
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n")?;
        let mut sequence = {
            let update = server.update.lock().unwrap();
            write!(stream, "data: {}\n\n", update.initial)?;
            Some(update.sequence)
        };
        let mut heartbeat = std::time::Instant::now();
        while !server.stopped.load(Ordering::Relaxed) {
            let packet = {
                let update = server.update.lock().unwrap();
                if sequence != Some(update.sequence) {
                    sequence = Some(update.sequence);
                    Some(update.packet.clone())
                } else {
                    None
                }
            };
            if let Some(packet) = packet {
                write!(stream, "data: {packet}\n\n")?;
            }
            if heartbeat.elapsed() >= Duration::from_secs(1) {
                stream.write_all(b": live\n\n")?;
                heartbeat = std::time::Instant::now();
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        return Ok(());
    }
    if method == "POST" && path == "/__deka/rebuild" {
        server.rebuild.store(true, Ordering::Relaxed);
        return stream
            .write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    }
    let assets = server.assets.lock().unwrap();
    if method != "GET" || !assets.contains_key(path) {
        return stream.write_all(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
    }
    let body = &assets[path];
    let mime = if path == "/" {
        "text/html; charset=utf-8"
    } else if path.ends_with(".wasm") {
        "application/wasm"
    } else {
        "application/javascript"
    };
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)
}
