//! deka#921: first-cut desktop packaging. A web project is built with the
//! same `deka build --bundle` pipeline as the browser, snapshotted through
//! `deka serve`, and launched in a wry/tao webview. Headless assertion reads
//! `document.body.innerText` via `DEKA_DESKTOP_DUMP=1`.
#![cfg(feature = "native")]

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

fn fixture_src() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/desktop-react")
}

fn copy_tree(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).expect("dst");
    for entry in fs::read_dir(src).expect("read src") {
        let entry = entry.expect("entry");
        let dest = dst.join(entry.file_name());
        if entry.file_type().expect("ty").is_dir() {
            copy_tree(&entry.path(), &dest);
        } else {
            fs::copy(entry.path(), dest).expect("copy file");
        }
    }
}

fn dsc_env(command: &mut Command) {
    if let Ok(dsc) = std::env::var("DEKA_DSC") {
        command.env("DEKA_DSC", dsc);
    } else if let Some(dsc) = compiler::dsc::find_cli_dsc().ok().flatten() {
        command.env("DEKA_DSC", dsc);
    }
}

#[test]
fn compile_desktop_rejects_a_source_file_and_preserves_existing_output() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("page.ds"), "export fn main() void { }\n").unwrap();
    let output = Command::new(cli_bin())
        .current_dir(dir.path())
        .args(["compile", "--desktop", "page.ds"])
        .output()
        .unwrap();
    assert!(!output.status.success(), "{output:?}");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        text.contains("web project directory") || text.contains("not a file"),
        "{text}"
    );
    fs::write(dir.path().join("deka-app"), "keep me").unwrap();
    let output = Command::new(cli_bin())
        .current_dir(dir.path())
        .args(["compile", "--desktop"])
        .output()
        .unwrap();
    assert!(!output.status.success(), "{output:?}");
    assert_eq!(
        fs::read_to_string(dir.path().join("deka-app")).unwrap(),
        "keep me"
    );
}

#[test]
fn compile_desktop_launches_webview_and_renders_react_output() {
    let source = tempfile::tempdir().unwrap();
    copy_tree(&fixture_src(), source.path());
    let destination = tempfile::tempdir().unwrap();
    let executable = destination.path().join("desktop-react");
    let mut compile = Command::new(cli_bin());
    compile
        .current_dir(source.path())
        .args(["compile", "--desktop", "--outfile"])
        .arg(&executable)
        .env("NO_COLOR", "1")
        .env("FORCE_COLOR", "0");
    dsc_env(&mut compile);
    let output = compile.output().unwrap();
    assert!(output.status.success(), "compile --desktop: {output:?}");
    assert!(executable.is_file(), "desktop executable missing");

    let (vfs_bytes, metadata) = compile::binary::extract_vfs(&executable).expect("desktop VFS");
    assert_eq!(metadata.entry_point, "index.html");
    let vfs = compile::vfs::VFS::from_bytes(&vfs_bytes).expect("parse VFS");
    assert_eq!(vfs.mode, compile::vfs::RuntimeMode::Desktop);
    let index = vfs.get_file("index.html").expect("index.html in snapshot");
    let html = String::from_utf8_lossy(&index.content);
    assert!(
        html.contains("Hello from Deka desktop"),
        "snapshot must contain the SSR React page:\n{html}"
    );
    assert!(
        html.contains("id=\"desktop-island\"") || html.contains("data-deka-island=\"HelloBadge\""),
        "snapshot must include the client island:\n{html}"
    );

    #[cfg(not(target_os = "macos"))]
    {
        let output = Command::new(&executable).output().unwrap();
        assert!(!output.status.success(), "non-macOS launch must fail");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            text.contains("macOS-only"),
            "expected macOS-only launch error, got: {text}"
        );
        return;
    }

    #[cfg(target_os = "macos")]
    {
        let mut child = Command::new(&executable)
            .env("DEKA_DESKTOP_DUMP", "1")
            .env("NO_COLOR", "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn desktop app");
        let mut stdout = child.stdout.take().expect("desktop stdout");
        let mut stderr = child.stderr.take().expect("desktop stderr");
        let deadline = Instant::now() + Duration::from_secs(45);
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(100));
                }
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("desktop dump timed out after 45s");
                }
                Err(err) => panic!("wait desktop app: {err}"),
            }
        };
        let mut out = Vec::new();
        let mut err = Vec::new();
        stdout.read_to_end(&mut out).expect("read desktop stdout");
        stderr.read_to_end(&mut err).expect("read desktop stderr");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out),
            String::from_utf8_lossy(&err)
        );
        assert!(status.success(), "desktop dump failed: {text}");
        assert!(
            text.contains("__DEKA_DESKTOP_DUMP__"),
            "dump marker missing: {text}"
        );
        assert!(
            text.contains("Hello from Deka desktop"),
            "webview did not render the React page text: {text}"
        );
        assert!(
            text.contains("React island"),
            "webview did not render the island: {text}"
        );
    }
}
