//! Public native compiler/VM CLI. The legacy V8 CLI remains a separate crate.
//! Command dispatch lives in [`cli`]; this module holds the implementations.
mod check_output;
pub mod cli;
mod packages;

mod desktop_runtime;

use deka_cli_core::registry::Output;
use deka_vm::{HostOp, HostReply, HostType, HostValue, Hosts, Program, Result, Vm, compiler};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

const TRAILER: &[u8; 16] = b"DEKA-NATIVE-APP1";

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Payload {
    version: u32,
    program: Program,
    desktop: bool,
}
#[derive(Deserialize)]
struct Project {
    entry: Option<String>,
    #[serde(default, rename = "entryFunction")]
    entry_function: Option<String>,
    desktop: Option<Desktop>,
}
#[derive(Deserialize)]
struct Desktop {
    entry: String,
    #[serde(rename = "entryFunction")]
    entry_function: String,
}
pub(crate) struct Source {
    pub(crate) path: PathBuf,
    entry: Option<String>,
    pub(crate) desktop: bool,
}
pub(crate) fn source(path: &Path, entry: Option<String>) -> Result<Source> {
    let path = if path.is_dir() {
        path.join("deka.json")
    } else {
        path.to_owned()
    };
    if path.file_name().is_some_and(|s| s == "deka.json") {
        let project: Project = serde_json::from_slice(
            &fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?,
        )
        .map_err(|e| e.to_string())?;
        let directory = path.parent().unwrap_or(Path::new("."));
        if let Some(desktop) = project.desktop {
            return Ok(Source {
                path: directory.join(desktop.entry),
                entry: entry.or(Some(desktop.entry_function)),
                desktop: true,
            });
        }
        return Ok(Source {
            path: directory.join(
                project
                    .entry
                    .ok_or("deka.json needs entry or desktop.entry")?,
            ),
            entry: entry.or(project.entry_function),
            desktop: false,
        });
    }
    if !matches!(
        path.extension().and_then(|s| s.to_str()),
        Some("ds" | "dsx")
    ) {
        return Err("source must be .ds, .dsx, or deka.json".into());
    }
    let desktop = path.extension().is_some_and(|s| s == "dsx");
    Ok(Source {
        path,
        entry: entry.or_else(|| desktop.then(|| "App".into())),
        desktop,
    })
}
pub(crate) fn hosts() -> Result<Hosts> {
    let mut hosts = Hosts::default();
    for name in ["echo", "print", compiler::CONSOLE_ERROR_OPERATION] {
        let diagnostic = name == compiler::CONSOLE_ERROR_OPERATION;
        hosts.register(HostOp::new(
            name,
            vec![HostType::String],
            HostType::Unit,
            false,
            move |args| {
                let HostValue::String(value) = &args[0] else {
                    unreachable!()
                };
                if diagnostic {
                    eprintln!("{value}");
                } else {
                    println!("{value}");
                }
                HostReply::Ready(Ok(HostValue::Unit))
            },
        ))?;
    }
    hosts.register(HostOp::new(
        "assert",
        vec![HostType::Bool],
        HostType::Unit,
        false,
        |args| {
            if args[0] == HostValue::Bool(true) {
                HostReply::Ready(Ok(HostValue::Unit))
            } else {
                HostReply::Ready(Err("assertion failed".into()))
            }
        },
    ))?;
    deka_vm::builtin_time::register(&mut hosts)?;
    deka_vm::builtin_math::register(&mut hosts)?;
    deka_vm::timers::register(&mut hosts)?;
    deka_vm::text_codec::register(&mut hosts)?;
    deka_vm::bytes::register(&mut hosts)?;
    deka_vm::url::register(&mut hosts)?;
    deka_vm::http_headers::register(&mut hosts)?;
    deka_vm::http_request::register(&mut hosts)?;
    deka_vm::http_response::register(&mut hosts)?;
    deka_vm::abort::register(&mut hosts)?;
    deka_vm::crypto::register(&mut hosts)?;
    deka_vm::builtin_crypto::register(&mut hosts)?;
    deka_vm::jwt::register(&mut hosts)?;
    deka_vm::fetch::register(&mut hosts)?;
    deka_vm::blob::register(&mut hosts)?;
    Ok(hosts)
}
pub(crate) fn compile(source: &Source) -> Result<Payload> {
    Ok(Payload {
        version: 1,
        program: compiler::compile_file(&source.path, &hosts()?, source.entry.as_deref())?,
        desktop: source.desktop,
    })
}
fn cli_runtime() -> Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())
}
pub(crate) fn execute(payload: Payload, exercise: Option<usize>) -> Result<()> {
    if payload.version != 1 {
        return Err("unsupported application format".into());
    }
    if payload.desktop {
        let app = desktop_runtime::Desktop::new(|| {
            deka_vm::ui::VmApp::with_hosts(payload.program, hosts()?)
        })?;
        if let Some(clicks) = exercise {
            println!("{}", deka_native_ui::exercise(app, clicks));
        } else {
            #[cfg(feature = "desktop")]
            deka_native_ui::run(app);
            #[cfg(not(feature = "desktop"))]
            return Err("this build has no desktop window host; use --exercise for CI".into());
        }
    } else {
        if exercise.is_some() {
            return Err("--exercise requires a desktop application".into());
        }
        let mut vm = Vm::new(payload.program, hosts()?)?;
        cli_runtime()?.block_on(vm.run())?;
    }
    Ok(())
}
pub(crate) fn embedded(path: &Path) -> Result<Option<Payload>> {
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    #[cfg(target_os = "macos")]
    let end = deka_executable::signed_data_end(&mut file)?.unwrap_or(size);
    #[cfg(not(target_os = "macos"))]
    let end = size;
    if end < 24 || end > size {
        return Ok(None);
    }
    // codesign may pad before the signature; scan only the signed data's tail.
    let start = end.saturating_sub(4096);
    file.seek(SeekFrom::Start(start))
        .map_err(|e| e.to_string())?;
    let mut tail = vec![0; (end - start) as usize];
    file.read_exact(&mut tail).map_err(|e| e.to_string())?;
    let Some(at) = tail.windows(16).rposition(|bytes| bytes == TRAILER) else {
        return Ok(None);
    };
    if at < 8 {
        return Err("invalid embedded application footer".into());
    }
    let length = u64::from_le_bytes(
        tail[at - 8..at]
            .try_into()
            .map_err(|_| "invalid application footer")?,
    );
    let payload_end = start + at as u64 - 8;
    if length > payload_end || length > 64 * 1024 * 1024 {
        return Err("invalid embedded application length".into());
    }
    file.seek(SeekFrom::Start(payload_end - length))
        .map_err(|e| e.to_string())?;
    let mut bytes = vec![0; length as usize];
    file.read_exact(&mut bytes).map_err(|e| e.to_string())?;
    let payload: Payload = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    payload.program.validate()?;
    Ok(Some(payload))
}
pub(crate) fn build(payload: Payload, destination: &Path, out: &Output) -> Result<()> {
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    // Compile first, then stage beside the destination so failures preserve an existing app.
    let staged = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    fs::copy(&executable, staged.path()).map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    sign(staged.path(), &["--remove-signature"])?;
    let mut bytes = fs::read(staged.path()).map_err(|e| e.to_string())?;
    let payload = serde_json::to_vec(&payload).map_err(|e| e.to_string())?;
    let length = payload.len() as u64;
    bytes.extend_from_slice(&payload);
    bytes.extend_from_slice(&length.to_le_bytes());
    bytes.extend_from_slice(TRAILER);
    #[cfg(target_os = "macos")]
    {
        let end = bytes.len() as u64;
        deka_executable::extend_linkedit(&mut bytes, end)?;
    }
    fs::write(staged.path(), &bytes).map_err(|e| e.to_string())?;
    fs::set_permissions(
        staged.path(),
        fs::metadata(executable)
            .map_err(|e| e.to_string())?
            .permissions(),
    )
    .map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    sign(staged.path(), &["--force", "--sign", "-"])?;
    staged.persist(destination).map_err(|e| e.to_string())?;
    out.print(format_args!("{}\n", destination.display()));
    Ok(())
}
pub(crate) fn tests(paths: &[String], out: &Output, err: &Output) -> Result<()> {
    fn discover(path: &Path, explicit: bool, out: &mut Vec<PathBuf>) -> Result<()> {
        if path.is_dir() {
            let mut entries = fs::read_dir(path)
                .map_err(|e| e.to_string())?
                .collect::<std::io::Result<Vec<_>>>()
                .map_err(|e| e.to_string())?;
            entries.sort_by_key(|e| e.path());
            for entry in entries {
                if matches!(
                    entry.file_name().to_str(),
                    Some("node_modules" | "ds_modules" | ".git" | ".target" | "target" | "dist")
                ) {
                    continue;
                }
                if entry.file_type().map_err(|e| e.to_string())?.is_symlink() {
                    continue;
                }
                discover(&entry.path(), false, out)?;
            }
        } else if explicit
            || path
                .file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.ends_with(".test.ds") || s.ends_with(".spec.ds"))
        {
            out.push(path.to_owned());
        }
        Ok(())
    }
    let mut files = vec![];
    for path in paths {
        discover(Path::new(path), true, &mut files)?;
    }
    files.sort();
    files.dedup();
    let mut passed = 0;
    let mut failed = 0;
    for file in files {
        let entries = compiler::test_entries(&file)?;
        for entry in entries {
            let result = compile(&Source {
                path: file.clone(),
                entry: Some(entry.clone()),
                desktop: false,
            })
            .and_then(|p| execute(p, None));
            match result {
                Ok(()) => {
                    passed += 1;
                    out.print(format_args!("PASS {}::{entry}\n", file.display()));
                }
                Err(error) => {
                    failed += 1;
                    err.print(format_args!("FAIL {}::{entry}: {error}\n", file.display()));
                }
            }
        }
    }
    out.print(format_args!("{passed} passed, {failed} failed\n"));
    if passed + failed == 0 {
        Err("no tests found (use *.test.ds and fn test_name())".into())
    } else if failed > 0 {
        Err(format!("{failed} test(s) failed"))
    } else {
        Ok(())
    }
}
pub(crate) fn init(directory: &Path) -> Result<()> {
    fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    for name in ["deka.json", "App.dsx", "app.test.ds"] {
        if directory.join(name).exists() {
            return Err(format!("init would overwrite {name}"));
        }
    }
    for (name, content) in [
        (
            "deka.json",
            "{\"name\":\"deka-app\",\"version\":\"0.1.0\",\"desktop\":{\"productName\":\"Deka App\",\"identifier\":\"gg.deka.app\",\"entry\":\"App.dsx\",\"entryFunction\":\"App\"}}\n",
        ),
        (
            "App.dsx",
            "export fn App() { let count = 0; return <view className=\"p-6 gap-4\"><p>Deka native app</p><p>Count: {count}</p><button onClick={fn() { count += 1; }}>Add one</button></view>; }\n",
        ),
        (
            "app.test.ds",
            "import { assert } from \"test\";\nfn test_addition() { assert(1 + 1 == 2); }\n",
        ),
    ] {
        fs::write(directory.join(name), content).map_err(|e| e.to_string())?;
    }
    Ok(())
}
pub(crate) fn dev(source: Source, payload: Payload) -> Result<()> {
    #[cfg(feature = "desktop")]
    {
        let files = compiler::source_files(&source.path)?;
        let watched = files
            .into_iter()
            .map(|path| {
                let bytes = fs::read(&path).ok();
                (path, bytes)
            })
            .collect();
        deka_native_ui::run(desktop_runtime::Desktop::new(|| {
            Ok(DevApp {
                source,
                app: deka_vm::ui::VmApp::with_hosts(payload.program, hosts()?)?,
                last: std::time::Instant::now(),
                watched,
            })
        })?);
        Ok(())
    }
    #[cfg(not(feature = "desktop"))]
    {
        let _ = source;
        execute(payload, None)
    }
}
#[cfg(feature = "desktop")]
struct DevApp {
    source: Source,
    app: deka_vm::ui::VmApp,
    last: std::time::Instant,
    watched: Vec<(PathBuf, Option<Vec<u8>>)>,
}
#[cfg(feature = "desktop")]
impl deka_native_ui::Application for DevApp {
    fn set_waker(&mut self, waker: deka_native_ui::Waker) {
        self.app.set_waker(waker);
    }
    fn has_ready_work(&self) -> bool {
        self.app.has_ready_work()
    }
    fn run_turn(&mut self, budget: usize) -> bool {
        self.app.run_turn(budget)
    }
    fn initial_state(&self) -> Vec<f64> {
        self.app.initial_state()
    }
    fn render(&self, state: &[f64]) -> deka_native_ui::Node {
        self.app.render(state)
    }
    fn event(&self, handler: usize, state: &mut [f64]) {
        self.app.event(handler, state);
    }
    fn live(&self) -> bool {
        true
    }
    fn poll_reload(&mut self) -> deka_native_ui::Reload {
        if self.last.elapsed() < std::time::Duration::from_millis(200) {
            return deka_native_ui::Reload::Unchanged;
        }
        self.last = std::time::Instant::now();
        let mut changed = false;
        for (path, previous) in &mut self.watched {
            let bytes = fs::read(path).ok();
            if *previous != bytes {
                changed = true;
                *previous = bytes;
            }
        }
        if !changed {
            return deka_native_ui::Reload::Unchanged;
        }
        if let Ok(files) = compiler::source_files(&self.source.path) {
            for path in files {
                if !self.watched.iter().any(|(old, _)| old == &path) {
                    let bytes = fs::read(&path).ok();
                    self.watched.push((path, bytes));
                }
            }
        }
        match compile(&self.source)
            .and_then(|p| deka_vm::ui::VmApp::with_hosts(p.program, hosts()?))
        {
            Ok(app) => {
                app.set_vm_waker(&self.app.waker());
                self.app = app;
                if let Ok(files) = compiler::source_files(&self.source.path) {
                    self.watched = files
                        .into_iter()
                        .map(|path| {
                            let bytes = fs::read(&path).ok();
                            (path, bytes)
                        })
                        .collect();
                }
                deka_native_ui::Reload::Reset
            }
            Err(error) => {
                eprintln!("deka dev: {error}");
                deka_native_ui::Reload::Unchanged
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn sign(path: &Path, args: &[&str]) -> Result<()> {
    let output = std::process::Command::new("/usr/bin/codesign")
        .args(args)
        .arg(path)
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "codesign: {}",
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

#[cfg(all(test, feature = "desktop"))]
mod dev_tests {
    use super::*;
    use deka_native_ui::{Application, Reload};
    fn text(node: &deka_native_ui::Node) -> String {
        let mut parts = vec![];
        if let Some(value) = &node.text {
            parts.push(value.clone());
        }
        for child in &node.children {
            parts.push(text(child));
        }
        parts.join(" ")
    }
    #[test]
    fn dependency_edit_updates_view_and_invalid_edit_preserves_it() {
        let directory = tempfile::tempdir().unwrap();
        let module = directory.path().join("title.ds");
        fs::write(&module, "export fn title() string { return \"before\"; }").unwrap();
        let entry = directory.path().join("App.dsx");
        fs::write(&entry, "import { title } from \"./title.ds\"; export fn App() { return <view><p>{title()}</p></view>; }").unwrap();
        let source = source(&entry, None).unwrap();
        let payload = compile(&source).unwrap();
        let watched = compiler::source_files(&source.path)
            .unwrap()
            .into_iter()
            .map(|path| {
                let bytes = fs::read(&path).ok();
                (path, bytes)
            })
            .collect();
        let mut app = DevApp {
            source,
            app: deka_vm::ui::VmApp::with_hosts(payload.program, hosts().unwrap()).unwrap(),
            last: std::time::Instant::now(),
            watched,
        };
        assert!(text(&app.render(&[])).contains("before"));
        fs::write(&module, "export fn title() string { return \"after\"; }").unwrap();
        app.last -= std::time::Duration::from_secs(1);
        assert!(matches!(app.poll_reload(), Reload::Reset));
        assert!(text(&app.render(&[])).contains("after"));
        fs::write(&module, "invalid edit").unwrap();
        app.last -= std::time::Duration::from_secs(1);
        assert!(matches!(app.poll_reload(), Reload::Unchanged));
        assert!(text(&app.render(&[])).contains("after"));
        app.last -= std::time::Duration::from_secs(1);
        assert!(matches!(app.poll_reload(), Reload::Unchanged));
        fs::write(&entry, "import { title } from \"./later.ds\"; export fn App() { return <view><p>{title()}</p></view>; }").unwrap();
        app.last -= std::time::Duration::from_secs(1);
        assert!(matches!(app.poll_reload(), Reload::Unchanged));
        fs::write(
            directory.path().join("later.ds"),
            "export fn title() string { return \"recovered\"; }",
        )
        .unwrap();
        app.last -= std::time::Duration::from_secs(1);
        assert!(matches!(app.poll_reload(), Reload::Reset));
        assert!(text(&app.render(&[])).contains("recovered"));
    }
}

#[cfg(test)]
mod async_runtime_tests {
    use super::cli_runtime;
    #[test]
    fn production_runtime_drives_network_io() {
        let runtime = cli_runtime().unwrap();
        let result = runtime.block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
                let address = listener.local_addr()?;
                let (client, (server, peer)) =
                    tokio::try_join!(tokio::net::TcpStream::connect(address), listener.accept())?;
                assert_eq!(client.peer_addr()?, address);
                assert_eq!(server.peer_addr()?, peer);
                Ok::<_, std::io::Error>(())
            })
            .await
        });
        result
            .expect("network operations timed out")
            .expect("network operation failed");
    }
}
