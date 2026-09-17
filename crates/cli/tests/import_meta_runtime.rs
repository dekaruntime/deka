//! End-to-end proof for deka#1139 (rfd#12 amendment): deka's module loader
//! supplies real `import.meta` values for DekaScript modules, and `url` /
//! `dirname` / `filename` name the original source file — never a compiled
//! or staged copy.
//!
//! This needs a real `dsc` that parses `import.meta` (dsc#282, pinned in
//! `scripts/dsc-version` as of deka#1149). It is resolved exactly the way
//! every other real-dsc CLI integration test in this file's directory
//! resolves one (`dekascript_run.rs` et al.): the spawned `cli` process
//! inherits `DEKA_DSC` from this test process's own environment — set by CI
//! (`ci.yml` installs the pinned dsc to `.ci/dsc` and exports `DEKA_DSC` for
//! the `cargo test` step) or, locally, by whatever the developer has on
//! `DEKA_DSC`/`PATH` (`compiler::dsc::find_cli_dsc`'s own resolution order).
//! No test-only env var, and no early return: without a resolvable dsc,
//! `deka run` fails with `[cli] dsc is required for check, fmt, transpile,
//! and lsp. …`, which surfaces as an ordinary, clearly-labeled test failure
//! below — never a silent pass.

use std::fs;
use std::path::Path;
use std::process::Command;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

struct Env {
    home: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        Self {
            home: tempfile::tempdir().expect("create fake home"),
        }
    }

    fn command(&self, dir: &Path) -> Command {
        let mut command = Command::new(cli_bin());
        command
            .current_dir(dir)
            .env("HOME", self.home.path())
            .env_remove("XDG_CACHE_HOME");
        command
    }
}

fn run(env: &Env, dir: &Path, args: &[&str]) -> (bool, String) {
    let output = env
        .command(dir)
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

/// Every assertion needs the printed line for one `console.log(JSON path)`
/// call: `<label>=<value>`.
fn field(output: &str, label: &str) -> String {
    output
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{label}=")))
        .unwrap_or_else(|| panic!("missing `{label}=` line in output:\n{output}"))
        .trim()
        .to_string()
}

// DekaScript has no bare `console`/`try` (rfd#13); output goes through
// `unsafe { … }` (the one escape hatch that splices raw JS verbatim,
// deka_test's own test_lib uses the identical `logLine` idiom) and a
// checked-exception `unsafe<T>` block stands in for `try/catch` to prove the
// frozen object actually throws on assignment (deka#1139 item 5).
const LOG_LINE_FN: &str = r#"
fn logLine(message: string) void {
  match (unsafe { console.log(message) }) {
    Ok(v) => v,
    Err(e) => e,
  }
}
"#;

const ASSIGN_THROWS_CHECK: &str = r#"
const assignResult = unsafe<string> {
  import.meta.url = "tampered";
  return "no-throw";
}
match (assignResult) {
  Ok(_) => logLine("assignThrew=false"),
  Err(_) => logLine("assignThrew=true"),
}
"#;

/// A single-file entry: no relative import, so it exercises loose-file
/// materialization's simplest shape (one source, one compiled artifact).
fn loose_entry_source() -> String {
    format!(
        "{LOG_LINE_FN}\n\
         logLine(\"entry.url=\" + import.meta.url)\n\
         logLine(\"entry.dirname=\" + import.meta.dirname)\n\
         logLine(\"entry.filename=\" + import.meta.filename)\n\
         logLine(\"entry.main=\" + string(import.meta.main))\n\
         logLine(\"entry.resolve=\" + import.meta.resolve(\"./helper.ds\"))\n\
         {ASSIGN_THROWS_CHECK}"
    )
}

const PROJECT_ENTRY_SOURCE: &str = r#"
import { helperUrl, helperMain } from "./helper.ds";

fn logLine(message: string) void {
  match (unsafe { console.log(message) }) {
    Ok(v) => v,
    Err(e) => e,
  }
}

logLine("entry.url=" + import.meta.url)
logLine("entry.dirname=" + import.meta.dirname)
logLine("entry.filename=" + import.meta.filename)
logLine("entry.main=" + string(import.meta.main))
logLine("entry.resolve=" + import.meta.resolve("./helper.ds"))
logLine("helper.url=" + helperUrl)
logLine("helper.main=" + string(helperMain))

const assignResult = unsafe<string> {
  import.meta.url = "tampered";
  return "no-throw";
}
match (assignResult) {
  Ok(_) => logLine("assignThrew=false"),
  Err(_) => logLine("assignThrew=true"),
}
"#;

// Untyped exports: deka's lightweight pre-dsc export scanner (a text
// heuristic, `crates/deka_host/src/validation/modules.rs`) does not parse a
// type annotation between the name and `=` — unrelated to dsc#282/deka#1139,
// so worked around here rather than fixed.
const HELPER_SOURCE: &str = r#"
export const helperUrl = import.meta.url
export const helperMain = import.meta.main
"#;

/// A loose file (no `deka.json` anywhere above it): it reports its real
/// `.ds` path under the *original* directory (deka#1139 item 1), not the
/// `~/.deka/cache/loose/<hash>/out/` copy `deka run` actually compiles and
/// executes, is its own `main` (item 3), `resolve()` matches (item 4), and
/// the frozen object rejects assignment (item 5). Item 2 ("an imported
/// module reports its own path, not the entry's") is covered by
/// `project_run_reports_its_source_path` below and by the direct
/// `PhpxEsmLoader` unit tests in `crates/pool/src/esm_loader.rs` — a loose
/// script importing a second loose `.ds` sibling hits an unrelated,
/// pre-existing gap in loose-file multi-module compilation (the sibling
/// is not carried into the self-contained `dsc transpile` output), so it is
/// not exercised end-to-end here.
#[test]
fn loose_run_reports_original_source_not_the_cache_copy() {
    let env = Env::new();
    let dir = tempfile::tempdir().expect("loose dir");
    let entry = dir.path().join("main.ds");
    fs::write(&entry, loose_entry_source()).expect("write entry");
    // Not imported — exists only so `import.meta.resolve` has a real sibling
    // to resolve against.
    fs::write(dir.path().join("helper.ds"), "export const x = 1\n").expect("write helper");

    let (ok, output) = run(&env, dir.path(), &["run", "main.ds"]);
    assert!(ok, "deka run failed:\n{output}");

    let original_dir = dir
        .path()
        .canonicalize()
        .unwrap_or_else(|_| dir.path().to_path_buf());
    let expected_entry_url = format!("file://{}", original_dir.join("main.ds").display());
    let expected_helper_url = format!("file://{}", original_dir.join("helper.ds").display());

    assert_eq!(field(&output, "entry.url"), expected_entry_url);
    assert_eq!(
        field(&output, "entry.dirname"),
        original_dir.display().to_string()
    );
    assert_eq!(
        field(&output, "entry.filename"),
        original_dir.join("main.ds").display().to_string()
    );
    assert_eq!(field(&output, "entry.main"), "true");
    assert_eq!(field(&output, "entry.resolve"), expected_helper_url);

    // The object is frozen: assigning to an existing property is a no-op in
    // sloppy mode and throws in strict mode; DekaScript modules are always
    // strict, so this must throw (deka#1139 item 5).
    assert_eq!(field(&output, "assignThrew"), "true");

    // Never the compiled cache copy.
    assert!(
        !output.contains(".deka") && !output.contains("cache/loose"),
        "a cache path leaked into import.meta:\n{output}"
    );
}

/// A project file (`deka.json` present): the entry executes directly from
/// the project tree, so this is the "ordinary" path — no cache/staging
/// indirection — and must report the same real source path
/// (deka#1139 item 1, "and a project file").
#[test]
fn project_run_reports_its_source_path() {
    let env = Env::new();
    let dir = tempfile::tempdir().expect("project dir");
    fs::write(dir.path().join("deka.json"), "{}\n").expect("write deka.json");
    fs::write(
        dir.path().join("deka.lock"),
        r#"{"lockfileVersion":1,"packages":{}}"#,
    )
    .expect("write deka.lock");
    let entry = dir.path().join("main.ds");
    fs::write(&entry, PROJECT_ENTRY_SOURCE).expect("write entry");
    fs::write(dir.path().join("helper.ds"), HELPER_SOURCE).expect("write helper");

    let (ok, output) = run(&env, dir.path(), &["run", "main.ds"]);
    assert!(ok, "deka run failed:\n{output}");

    let project_dir = dir
        .path()
        .canonicalize()
        .unwrap_or_else(|_| dir.path().to_path_buf());
    let expected_entry_url = format!("file://{}", project_dir.join("main.ds").display());
    assert_eq!(field(&output, "entry.url"), expected_entry_url);
    assert_eq!(field(&output, "entry.main"), "true");
    assert_eq!(field(&output, "helper.main"), "false");
    assert_eq!(field(&output, "assignThrew"), "true");
}
