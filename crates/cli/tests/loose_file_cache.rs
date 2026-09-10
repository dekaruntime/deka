//! deka#765: running a loose `.ds` file (one with no `deka.json` anywhere
//! above it) must work without writing anything into the directory the user
//! ran from. The compile output materializes into a user-global cache and
//! the artifact is the executed subject.
//!
//! The load-bearing assertion in these tests is the directory snapshot: it
//! is the actual user complaint from #743, asserted rather than inferred.
//! Everything runs against a stub `dsc` (via the pre-existing DEKA_DSC test
//! hook) so no real toolchain is needed; the stub emits real JavaScript and
//! records its invocations, which makes cache hits and compiler-version
//! rejection observable end to end.

use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

const EMPTY_DEKA_LOCK: &str = r#"{"lockfileVersion":1,"packages":{}}"#;
const NOTE: &str = "[note] not a deka project";

/// Every test gets its own HOME so the user cache resolves under a tempdir
/// (`~/.deka/cache`) instead of the developer's real home directory. No
/// deka-specific env override exists for this (deka#801); HOME is the
/// platform home directory, which is what the resolution order keys on.
struct Env {
    home: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        Self {
            home: tempfile::tempdir().expect("create fake home"),
        }
    }

    fn command(&self, dir: &Path, dsc: &Path, version: &str) -> Command {
        let mut command = Command::new(cli_bin());
        command
            .current_dir(dir)
            .env("HOME", self.home.path())
            .env_remove("XDG_CACHE_HOME")
            .env("DEKA_DSC", dsc)
            .env("STUB_DSC_VERSION", version)
            .env("DEKA_RATE_LIMIT_DISABLED", "1");
        command
    }

    fn cache_root(&self) -> PathBuf {
        self.home.path().join(".deka").join("cache")
    }
}

/// Stub `dsc`: `--version` reports STUB_DSC_VERSION; `transpile --out DIR`
/// emits one real JS module per source (a top-level console.log plus a
/// WinterTC `fetch` export so the same payload serves), and logs every
/// transpile invocation so tests can observe cache hits and regeneration.
fn write_dsc_stub(root: &Path) -> PathBuf {
    let stub = root.join("dsc-stub.sh");
    let log = root.join("dsc-transpile.log");
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "dsc stub ${{STUB_DSC_VERSION:-1.0.0}}"
  exit 0
fi
echo "$$" >> "{log}"
out=""
prev=""
src=""
for arg in "$@"; do
  if [ "$prev" = "--out" ]; then out="$arg"; fi
  prev="$arg"
  case "$arg" in
    --*) ;;
    transpile) ;;
    *) [ -z "$src" ] && src="$arg" ;;
  esac
done
if [ -z "$out" ] || [ -z "$src" ]; then
  echo "stub-dsc: unparseable argv: $*" >&2
  exit 1
fi
mkdir -p "$out"
stem=$(basename "$src" | sed 's/\.[^.]*$//')
printf 'console.log("stub-%s")\nexport default {{ async fetch() {{ return new Response("stub-%s") }} }}\n' "${{STUB_DSC_VERSION:-1.0.0}}" "${{STUB_DSC_VERSION:-1.0.0}}" > "$out/$stem.js"
"#,
        log = log.display()
    );
    fs::write(&stub, script).expect("write dsc stub");
    make_executable(&stub);
    stub
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path).expect("stat stub").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).expect("chmod stub");
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) {}

fn transpile_count(tooling: &Path) -> usize {
    fs::read_to_string(tooling.join("dsc-transpile.log"))
        .map(|text| text.lines().filter(|line| !line.trim().is_empty()).count())
        .unwrap_or(0)
}

fn run_loose(env: &Env, dir: &Path, dsc: &Path, version: &str, args: &[&str]) -> (bool, String) {
    let output = env
        .command(dir, dsc, version)
        .args(args)
        .output()
        .expect("run deka");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), combined)
}

/// Full recursive snapshot of a directory: every path relative to the root
/// with its bytes. The before/after equality IS the user complaint.
fn snapshot_dir(root: &Path) -> Vec<(String, Vec<u8>)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let rel = path.strip_prefix(root).expect("rel").display().to_string();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                out.push((rel, fs::read(&path).expect("read file")));
            }
        }
    }
    let mut snapshot = Vec::new();
    walk(root, root, &mut snapshot);
    snapshot.sort();
    snapshot
}

fn loose_dir_with(name: &str, body: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("create loose dir");
    let source = dir.path().join(name);
    fs::write(&source, body).expect("write loose source");
    (dir, source)
}

#[test]
fn loose_run_executes_from_user_cache_and_leaves_cwd_byte_identical() {
    let env = Env::new();
    let tooling = tempfile::tempdir().expect("tooling dir");
    let dsc = write_dsc_stub(tooling.path());
    let (dir, _source) = loose_dir_with("app.ds", "export const ok = true\n");
    let before = snapshot_dir(dir.path());

    let (ok, text) = run_loose(&env, dir.path(), &dsc, "1.0.0", &["run", "app.ds"]);

    assert!(ok, "loose run failed: {text}");
    assert!(
        text.contains("stub-1.0.0"),
        "compiled artifact must be the executed subject: {text}"
    );
    assert!(
        text.contains(NOTE) && text.contains("deka init"),
        "missing the not-a-project advisory: {text}"
    );
    // The advisory comes after the program output (RFD 55: advisories last).
    assert!(
        text.find("stub-1.0.0").expect("program output") < text.find(NOTE).expect("advisory"),
        "advisory must follow the program output: {text}"
    );
    assert_eq!(
        snapshot_dir(dir.path()),
        before,
        "loose run wrote into the user's directory"
    );
    // Output materialized into the user-global cache.
    let loose = env.cache_root().join("loose");
    let entries: Vec<_> = fs::read_dir(&loose)
        .expect("cache root exists")
        .flatten()
        .collect();
    assert_eq!(entries.len(), 1, "one content-addressed entry: {entries:?}");
    let artifact = entries[0].path().join("out").join("app.js");
    assert!(
        artifact.is_file(),
        "materialized artifact missing at {}",
        artifact.display()
    );
    assert_eq!(transpile_count(tooling.path()), 1, "cold path compiles once");
}

#[test]
fn loose_run_warm_path_is_quiet_and_compiler_version_change_regenerates() {
    let env = Env::new();
    let tooling = tempfile::tempdir().expect("tooling dir");
    let dsc = write_dsc_stub(tooling.path());
    let (dir, _source) = loose_dir_with("app.ds", "export const ok = true\n");

    let (ok, text) = run_loose(&env, dir.path(), &dsc, "1.0.0", &["run", "app.ds"]);
    assert!(ok, "cold run failed: {text}");
    assert_eq!(transpile_count(tooling.path()), 1);

    // Warm path: same compiler, same content — no recompile (RFD 55 quiet
    // warm path), same output.
    let (ok, text) = run_loose(&env, dir.path(), &dsc, "1.0.0", &["run", "app.ds"]);
    assert!(ok, "warm run failed: {text}");
    assert!(text.contains("stub-1.0.0"), "{text}");
    assert_eq!(
        transpile_count(tooling.path()),
        1,
        "warm entry must not recompile"
    );

    // Same content hash, different reported compiler version: the cached
    // entry must be rejected and regenerated, not reused (a digest-only key
    // is what made #743's worst symptom possible).
    let (ok, text) = run_loose(&env, dir.path(), &dsc, "2.0.0", &["run", "app.ds"]);
    assert!(ok, "post-upgrade run failed: {text}");
    assert!(
        text.contains("stub-2.0.0"),
        "stale artifact was reused instead of regenerated: {text}"
    );
    assert_eq!(
        transpile_count(tooling.path()),
        2,
        "compiler change must recompile exactly once"
    );
}

#[test]
fn project_run_is_unchanged_and_gets_no_advisory() {
    let env = Env::new();
    let tooling = tempfile::tempdir().expect("tooling dir");
    let dsc = write_dsc_stub(tooling.path());
    let project = tempfile::tempdir().expect("project dir");
    fs::write(project.path().join("deka.json"), "{}\n").expect("write manifest");
    fs::write(project.path().join("deka.lock"), EMPTY_DEKA_LOCK).expect("write lockfile");
    fs::write(project.path().join("app.ds"), "export const ok = true\n").expect("write source");

    let (ok, text) = run_loose(&env, project.path(), &dsc, "1.0.0", &["run", "app.ds"]);

    assert!(ok, "project run failed: {text}");
    assert!(text.contains("stub-1.0.0"), "{text}");
    assert!(
        !text.contains(NOTE),
        "a project run must not print the not-a-project advisory: {text}"
    );
    // Project behavior: the loader compiles into the project's own cache.
    assert!(
        project.path().join(".cache").join("dsc-modules").join("app.js").is_file(),
        "project compile cache missing; project behavior changed"
    );
    // And nothing went into the user cache.
    assert!(
        !env.cache_root().join("loose").exists(),
        "project run must not touch the user cache"
    );
}

#[cfg(unix)]
#[test]
fn loose_run_works_when_source_directory_is_read_only() {
    use std::os::unix::fs::PermissionsExt;

    let env = Env::new();
    let tooling = tempfile::tempdir().expect("tooling dir");
    let dsc = write_dsc_stub(tooling.path());
    let (dir, _source) = loose_dir_with("app.ds", "export const ok = true\n");
    let before = snapshot_dir(dir.path());

    fs::set_permissions(dir.path(), {
        let mut perms = fs::metadata(dir.path()).expect("stat dir").permissions();
        perms.set_mode(0o555);
        perms
    })
    .expect("make source dir read-only");

    let (ok, text) = run_loose(&env, dir.path(), &dsc, "1.0.0", &["run", "app.ds"]);

    fs::set_permissions(dir.path(), {
        let mut perms = fs::metadata(dir.path()).expect("stat dir").permissions();
        perms.set_mode(0o755);
        perms
    })
    .expect("restore source dir permissions");

    assert!(ok, "read-only loose run failed: {text}");
    assert!(text.contains("stub-1.0.0"), "{text}");
    assert!(text.contains(NOTE), "{text}");
    assert_eq!(
        snapshot_dir(dir.path()),
        before,
        "read-only run wrote into the source directory"
    );
    assert!(env.cache_root().join("loose").is_dir());
}

#[test]
fn xdg_cache_home_takes_precedence_over_home() {
    let env = Env::new();
    let xdg = tempfile::tempdir().expect("xdg dir");
    let tooling = tempfile::tempdir().expect("tooling dir");
    let dsc = write_dsc_stub(tooling.path());
    let (dir, _source) = loose_dir_with("app.ds", "export const ok = true\n");

    let output = env
        .command(dir.path(), &dsc, "1.0.0")
        .env("XDG_CACHE_HOME", xdg.path())
        .args(["run", "app.ds"])
        .output()
        .expect("run deka");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(output.status.success(), "xdg run failed: {combined}");
    assert!(combined.contains("stub-1.0.0"), "{combined}");
    assert!(
        xdg.path().join("deka").join("loose").is_dir(),
        "cache must land under XDG_CACHE_HOME/deka"
    );
    assert!(
        !env.cache_root().exists(),
        "XDG_CACHE_HOME wins; HOME cache must not be created"
    );
}

struct KillOnDrop(Option<std::process::Child>);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn free_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .expect("bind ephemeral")
        .local_addr()
        .expect("local addr")
        .port()
}

#[test]
fn loose_serve_materializes_into_cache_and_leaves_cwd_clean() {
    let env = Env::new();
    let tooling = tempfile::tempdir().expect("tooling dir");
    let dsc = write_dsc_stub(tooling.path());
    let (dir, _source) = loose_dir_with("app.ds", "export const ok = true\n");
    let scratch = tempfile::tempdir().expect("scratch dir (serve log lives outside the source dir)");
    let before = snapshot_dir(dir.path());
    let port = free_port();
    let log_path = scratch.path().join("serve.log");
    let log = fs::File::create(&log_path).expect("serve log");

    let child = env
        .command(dir.path(), &dsc, "1.0.0")
        .args(["serve", "app.ds", "--port", &port.to_string(), "--no-prompt"])
        .stdout(Stdio::from(log.try_clone().expect("clone log")))
        .stderr(Stdio::from(log))
        .spawn()
        .expect("spawn deka serve");
    let _guard = KillOnDrop(Some(child));

    // Poll until the server answers.
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .expect("reqwest client");
    let deadline = Instant::now() + Duration::from_secs(45);
    let mut body = String::new();
    while Instant::now() < deadline {
        if let Ok(response) = client.get(format!("http://127.0.0.1:{port}/")).send() {
            body = response.text().unwrap_or_default();
            if !body.is_empty() {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    let log_text = fs::read_to_string(&log_path).unwrap_or_default();

    assert!(
        body.contains("stub-1.0.0"),
        "served response must come from the materialized artifact; body={body:?} log={log_text}"
    );
    assert!(
        log_text.contains(NOTE),
        "loose serve must print the not-a-project advisory: {log_text}"
    );
    assert_eq!(
        snapshot_dir(dir.path()),
        before,
        "loose serve wrote into the source directory"
    );
    assert!(
        env.cache_root().join("loose").is_dir(),
        "serve must materialize into the user cache"
    );
}

#[test]
fn cache_clear_empties_the_loose_cache() {
    let env = Env::new();
    let tooling = tempfile::tempdir().expect("tooling dir");
    let dsc = write_dsc_stub(tooling.path());
    let (dir, _source) = loose_dir_with("app.ds", "export const ok = true\n");

    let (ok, text) = run_loose(&env, dir.path(), &dsc, "1.0.0", &["run", "app.ds"]);
    assert!(ok, "seed run failed: {text}");
    assert!(env.cache_root().join("loose").is_dir());

    let (ok, text) = run_loose(&env, dir.path(), &dsc, "1.0.0", &["cache", "clear"]);
    assert!(ok, "cache clear failed: {text}");

    assert!(
        !env.cache_root().join("loose").exists(),
        "cache clear must remove the loose entries"
    );
}
