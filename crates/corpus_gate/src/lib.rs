//! Corpus gate library (deka#1214): enumerate testsuite-corpus cases,
//! materialize fixtures exactly like the corpus runner, check then execute them,
//! and evaluate the outcome against each case's expectation.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
mod process;

pub const DEFAULT_DEKA_LOCK: &str = "{\n  \"lockfileVersion\": 1,\n  \"packages\": {}\n}\n";
pub const DEFAULT_DEKA_JSON: &str = "{\n  \"name\": \"conformance-fixture\",\n  \"security\": {\n    \"allow\": {\n      \"read\": [\"./\"],\n      \"write\": [\".cache\"]\n    },\n    \"prompt\": false\n  }\n}\n";
pub const PACKAGE_DEKA_JSON: &str = "{\n  \"name\": \"conformance-fixture\",\n  \"security\": {\n    \"allow\": {\n      \"read\": [\"./\"],\n      \"write\": [\".cache\", \"php_modules\", \"ds_modules\"]\n    },\n    \"prompt\": false\n  }\n}\n";
const RUN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

// The pinned pre-VM corpus omitted text guards on these negative programs.
// Fill only absent guards; explicit corpus metadata remains authoritative.
fn native_diagnostic(slug: &str) -> Option<&'static str> {
    Some(match slug {
        "data-types-array-find-undefined-access-fail" => {
            "cannot access field `length` on type `Option<number>`"
        }
        "data-types-string-find-missing-fail" => "`string` has no field `find`",
        "data-types-array-reduce-empty-no-initial-fail" => {
            "cannot reduce an empty array without an initial value"
        }
        "flow-control-break-in-if-fail" | "flow-control-break-outside-loop-fail" => {
            "`break` outside of loop"
        }
        "flow-control-continue-in-if-fail" | "flow-control-continue-outside-loop-fail" => {
            "`continue` outside of loop"
        }
        "flow-control-for-of-const-reassign-fail" => "cannot assign to immutable variable `x`",
        _ => return None,
    })
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Pass,
    Fail,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage {
    Parse,
    Typecheck,
    Run,
}

#[derive(Debug)]
pub struct Case {
    pub slug: String,
    pub status: Status,
    pub stage: Stage,
    pub source: String,
    pub entry_path: String,
    pub files: Vec<(String, String)>,
    pub expected_stdout: Option<String>,
    pub expected_diagnostic_contains: Option<String>,
    pub deka_json: Option<serde_json::Value>,
    pub packages: Vec<String>,
}

fn collect_ds_files(dir: &Path, relative_to: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let full = entry.path();
        if full.is_dir() {
            collect_ds_files(&full, relative_to, out);
        } else if full
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                name.ends_with(".ds")
                    || name.ends_with(".dsx")
                    || name.ends_with(".css")
                    || name.ends_with(".mjs")
            })
        {
            if let Ok(rel) = full.strip_prefix(relative_to) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
}

fn load_case(category: &str, name: &str, dir: &Path) -> Option<Case> {
    let mut ds_files = Vec::new();
    collect_ds_files(dir, dir, &mut ds_files);
    let entry = ds_files.iter().find(|f| {
        let top_level = !f.contains('/');
        let status = (f.ends_with(".pass.ds")
            || f.ends_with(".pass.dsx")
            || f.ends_with(".fail.ds")
            || f.ends_with(".fail.dsx"));
        top_level && status
    })?;
    let status = if entry.ends_with(".pass.ds") || entry.ends_with(".pass.dsx") {
        Status::Pass
    } else {
        Status::Fail
    };
    let base = entry
        .trim_end_matches(".pass.dsx")
        .trim_end_matches(".fail.dsx")
        .trim_end_matches(".pass.ds")
        .trim_end_matches(".fail.ds");
    let source = std::fs::read_to_string(dir.join(entry)).ok()?;
    let metadata: serde_json::Value = std::fs::read_to_string(dir.join(format!("{base}.json")))
        .ok()
        .and_then(|bytes| serde_json::from_str(&bytes).ok())
        .unwrap_or(serde_json::Value::Null);
    let hosts: Vec<&str> = metadata
        .get("hosts")
        .and_then(|h| h.as_array())
        .map(|items| {
            let hosts: Vec<&str> = items
                .iter()
                .filter_map(|item| item.as_str())
                .filter(|item| *item == "native" || *item == "browser")
                .collect();
            if hosts.is_empty() {
                vec!["native", "browser"]
            } else {
                hosts
            }
        })
        .unwrap_or_else(|| vec!["native", "browser"]);
    if !hosts.contains(&"native") {
        return None;
    }
    let stage = match metadata.get("stage").and_then(|s| s.as_str()) {
        Some("parse") => Stage::Parse,
        Some("typecheck") => Stage::Typecheck,
        _ => Stage::Run,
    };
    let expected_stdout = metadata
        .get("expectedStdoutNative")
        .and_then(|s| s.as_str())
        .map(str::to_owned)
        .or_else(|| std::fs::read_to_string(dir.join(format!("{base}.stdout"))).ok());
    let expected_diagnostic_contains = metadata
        .get("expectedDiagnosticContains")
        .and_then(|s| s.as_str())
        .map(str::to_owned);
    let deka_json = metadata.get("dekaJson").filter(|d| d.is_object()).cloned();
    let packages: Vec<String> = metadata
        .get("packages")
        .and_then(|p| p.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .filter(|item| !item.trim().is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let files: Vec<(String, String)> = ds_files
        .iter()
        .filter(|f| *f != entry)
        .filter_map(|f| Some((f.clone(), std::fs::read_to_string(dir.join(f)).ok()?)))
        .collect();
    let slug: String = format!("{category}-{name}")
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    // Note 09 deliberately restores crypto after the pinned corpus removed it.
    // Migrate only that obsolete absence assertion on its exact original source;
    // changed source or updated explicit metadata remains authoritative.
    let expected_diagnostic_contains = if status == Status::Fail
        && slug == "error-globals-crypto-method-missing-fail"
        && expected_diagnostic_contains.as_deref() == Some("unknown identifier `crypto`")
        && source.trim() == "import { echo } from \"io\"\necho(crypto.nonexistent())"
    {
        Some("cannot inspect opaque type `Crypto`; call a declared receiver method or summoned function".into())
    } else {
        expected_diagnostic_contains
    };
    let expected_diagnostic_contains = expected_diagnostic_contains.or_else(|| {
        if status == Status::Fail {
            native_diagnostic(&slug).map(str::to_owned)
        } else {
            None
        }
    });
    Some(Case {
        slug,
        status,
        stage,
        source,
        entry_path: entry.clone(),
        files,
        expected_stdout,
        expected_diagnostic_contains,
        deka_json,
        packages,
    })
}

/// Every native-runnable case in the corpus, in deterministic order.
pub fn load_cases(corpus: &Path) -> Vec<Case> {
    let mut cases = Vec::new();
    let Ok(categories) = std::fs::read_dir(corpus) else {
        return cases;
    };
    let mut categories: Vec<_> = categories
        .flatten()
        .filter(|entry| {
            entry.path().is_dir() && !entry.file_name().to_string_lossy().starts_with('.')
        })
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    categories.sort();
    for category in categories {
        let category_dir = corpus.join(&category);
        let Ok(tests) = std::fs::read_dir(&category_dir) else {
            continue;
        };
        let mut tests: Vec<_> = tests
            .flatten()
            .filter(|entry| entry.path().is_dir())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        tests.sort();
        for test in tests {
            if let Some(case) = load_case(&category, &test, &category_dir.join(&test)) {
                cases.push(case);
            }
        }
    }
    cases
}

fn write_project(case: &Case, directory: &Path) -> String {
    if case.files.is_empty() {
        let ext = if case.entry_path.ends_with(".dsx") {
            ".dsx"
        } else {
            ".ds"
        };
        let target = format!("test{ext}");
        std::fs::write(directory.join(&target), &case.source).unwrap();
        return format!("./{target}");
    }
    let entry = directory.join(&case.entry_path);
    std::fs::create_dir_all(entry.parent().unwrap()).unwrap();
    std::fs::write(&entry, &case.source).unwrap();
    for (path, content) in &case.files {
        let full = directory.join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, content).unwrap();
    }
    format!("./{}", case.entry_path)
}

#[derive(Debug)]
pub struct RunResult {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
    pub error: Option<String>,
    /// Compilation failed in the observed check phase, never inferred from stdout.
    pub transpile_failed: bool,
    pub diagnostics: Vec<String>,
}

fn parse_native_diagnostics(stderr: &str) -> Vec<String> {
    let mut diagnostics = Vec::new();
    for line in stderr.lines() {
        if let Some((_, message)) = line.split_once('^') {
            let message = message.trim();
            if !message.is_empty() {
                diagnostics.push(message.to_string());
            }
        }
    }
    if diagnostics.is_empty() {
        if let Some(first) = stderr.lines().map(str::trim).find(|line| {
            !line.is_empty()
                && !line.starts_with('[')
                && !line.starts_with("Validation")
                && !line.starts_with('❌')
        }) {
            diagnostics.push(first.to_string());
        }
    }
    diagnostics
}

fn native_json_source(source: &str) -> bool {
    use deka_syntax::ast::{ExportDecl, Stmt};
    let arena = bumpalo::Bump::new();
    let parsed = deka_syntax::parse(source, &arena);
    if !parsed.errors.is_empty() {
        return false;
    }
    let Some(program) = parsed.program else {
        return false;
    };
    program.statements.iter().all(|statement| {
        let dependency = match statement {
            Stmt::Import { source, .. }
            | Stmt::Export {
                decl:
                    ExportDecl::NamedGroup {
                        source: Some(source),
                        ..
                    },
                ..
            } => Some(*source),
            _ => None,
        };
        dependency.is_none_or(|source| {
            source == "io" || source.starts_with("./") || source.starts_with("../")
        })
    })
}

// These pinned fixtures only need the native clock. Exact original source
// and metadata are required; execution and stdout comparisons remain mandatory.
fn native_time_fixture(case: &Case) -> bool {
    let source = match case.slug.as_str() {
        "time-now-after-epoch" => {
            "import { echo } from \"io\"\nimport { now } from \"time\"\necho(string(now() > 1577836800000))\n"
        }
        "time-now-is-number" => {
            "import { echo } from \"io\"\nimport { now } from \"time\"\necho(string(now() > 0))\n"
        }
        "time-two-nows" => {
            "import { echo } from \"io\"\nimport { now } from \"time\"\nconst a = now()\nconst b = now()\necho(string(b >= a))\n"
        }
        _ => return false,
    };
    case.status == Status::Pass
        && case.stage == Stage::Run
        && case.source == source
        && case.packages == ["time"]
        && case.files.is_empty()
        && case.deka_json.is_none()
        && case.expected_stdout.as_deref() == Some("true\n")
        && case.expected_diagnostic_contains.is_none()
}

// This one pinned fixture's package metadata predates the native bytes module.
// Only obsolete metadata is bypassed: source and its exact output are still run.
const LEGACY_BYTES_SOURCE: &str = "import { echo } from \"io\"\nimport { from_string, len } from \"bytes\"\n\nconst encoded = from_string(\"hello\")\necho(string(len(encoded)))\n";
fn native_bytes_fixture(case: &Case) -> bool {
    case.slug == "packages-bytes-from-string-len"
        && case.status == Status::Pass
        && case.stage == Stage::Run
        && case.packages == ["bytes"]
        && case.source == LEGACY_BYTES_SOURCE
        && case.expected_stdout.as_deref() == Some("5\n")
        && case.expected_diagnostic_contains.is_none()
        && case.files.is_empty()
        && case.deka_json.is_none()
}

// Only these unchanged canonical client calls bypass obsolete package metadata.
// Every other legacy HTTP program remains gated; source and stdout still execute.
fn native_http_fixture(case: &Case) -> bool {
    case.status == Status::Pass
        && case.stage == Stage::Run
        && case.packages == ["http", "tcp", "tls"]
        && case.files.is_empty()
        && case.expected_stdout.as_deref() == Some("err\n")
        && case.expected_diagnostic_contains.is_none()
        && match case.slug.as_str() {
            "http-invalid-url" => {
                case.source
                    == "import { echo } from \"io\"\nimport { get } from \"http\"\necho(match (get(\"not a url\")) {\n  Ok(v) => \"ok\",\n  Err(e) => \"err\",\n})\n"
                    && case.deka_json.is_none()
            }
            "http-get-refused" => {
                case.source
                    == "import { echo } from \"io\"\nimport { get } from \"http\"\necho(match (get(\"http://127.0.0.1:1/\")) {\n  Ok(v) => \"ok\",\n  Err(e) => \"err\",\n})\n"
                    && case.deka_json.as_ref()
                        == Some(
                            &serde_json::json!({"name": "conformance-fixture", "security": {"allow": {"read": ["./"], "write": [".cache", "php_modules"], "net": ["127.0.0.1:1"]}, "prompt": false}}),
                        )
            }
            _ => false,
        }
}

pub fn run_case(deka: &Path, case: &Case, scratch: &Path) -> Result<RunResult, String> {
    // The pinned JSON corpus predates native typed JSON and still names its
    // old package. These fixtures now run against the built-in language path;
    // source compilation/execution remains mandatory. Other packages fail closed.
    let native_json = case.slug.starts_with("json-")
        && case.packages.iter().all(|package| package == "json")
        && std::iter::once(case.source.as_str())
            .chain(case.files.iter().map(|(_, source)| source.as_str()))
            .all(native_json_source);
    let native_fixture = native_json
        || native_bytes_fixture(case)
        || native_time_fixture(case)
        || native_http_fixture(case);
    if !case.packages.is_empty() && !native_fixture {
        return Err(format!(
            "{} declares packages {:?}; the gate is offline and cannot install them",
            case.slug, case.packages
        ));
    }
    let directory = scratch.join(&case.slug);
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let entry = write_project(case, &directory);
    std::fs::write(directory.join("deka.lock"), DEFAULT_DEKA_LOCK).unwrap();
    let deka_json = match &case.deka_json {
        Some(value) => format!("{}\n", serde_json::to_string_pretty(value).unwrap()),
        None if !case.packages.is_empty() && !native_fixture => PACKAGE_DEKA_JSON.to_string(),
        None => DEFAULT_DEKA_JSON.to_string(),
    };
    std::fs::write(directory.join("deka.json"), deka_json).unwrap();

    let checked = process::execute(deka, "check", &entry, &directory, &case.slug)?;
    if !checked.status.success() {
        return Ok(command_result(checked, true));
    }
    let output = process::execute(deka, "run", &entry, &directory, &case.slug)?;
    Ok(command_result(output, false))
}

fn command_result(output: std::process::Output, check_failed: bool) -> RunResult {
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let raw_stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let stderr: String = raw_stderr
        .lines()
        .filter(|line| !line.starts_with("[security]"))
        .collect::<Vec<_>>()
        .join("\n");
    let failed = !output.status.success();
    let diagnostics = if failed {
        parse_native_diagnostics(if stderr.is_empty() {
            &raw_stderr
        } else {
            &stderr
        })
    } else {
        Vec::new()
    };
    let error = if failed {
        Some(
            diagnostics
                .first()
                .cloned()
                .or_else(|| {
                    stderr
                        .lines()
                        .map(str::trim)
                        .find(|l| !l.is_empty())
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| "deka run failed".into()),
        )
    } else {
        None
    };
    RunResult {
        ok: !failed,
        stdout,
        stderr,
        error,
        transpile_failed: check_failed,
        diagnostics,
    }
}

fn stage_matches(expected: Stage, actual: Stage) -> bool {
    if expected == actual {
        return true;
    }
    // The combined source check reports parse and typecheck diagnostics in
    // one phase; preserve the corpus's existing compilation-stage grouping.
    matches!(
        (expected, actual),
        (Stage::Parse, Stage::Typecheck) | (Stage::Typecheck, Stage::Parse)
    )
}

/// Evaluate one run against the case's expectation, the corpus runner's rules.
pub fn evaluate(case: &Case, result: &RunResult) -> Vec<String> {
    let mut reasons = Vec::new();
    let actual_stage = if result.ok {
        Stage::Run
    } else if result.transpile_failed {
        Stage::Parse
    } else {
        Stage::Run
    };
    let actual_status = if result.ok {
        Status::Pass
    } else {
        Status::Fail
    };
    if actual_status != case.status {
        reasons.push(format!(
            "status: want {:?}, got {:?}{}",
            case.status,
            actual_status,
            result
                .error
                .as_ref()
                .map(|e| format!(" ({e})"))
                .unwrap_or_default()
        ));
    }
    if !stage_matches(case.stage, actual_stage) {
        reasons.push(format!(
            "stage: want {:?}, got {:?}",
            case.stage, actual_stage
        ));
    }
    if let Some(expected) = &case.expected_stdout
        && result.stdout != *expected
    {
        reasons.push(format!(
            "stdout: expected {expected:?}, got {:?}",
            result.stdout
        ));
    }
    if let Some(needle) = &case.expected_diagnostic_contains {
        let hay = format!(
            "{}\n{}\n{}",
            result.diagnostics.join("\n"),
            result.stderr,
            result.error.clone().unwrap_or_default()
        )
        .to_lowercase();
        if !hay.contains(&needle.to_lowercase()) {
            reasons.push(format!(
                "diagnostic: expected to contain {needle:?}, got {:?}",
                result.error.as_deref().unwrap_or(&result.stderr)
            ));
        }
    }
    reasons
}

pub fn read_passing_list(path: &Path) -> Result<BTreeSet<String>, String> {
    let contents = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn case(status: Status, stage: Stage) -> Case {
        Case {
            slug: "cat-case".into(),
            status,
            stage,
            source: String::new(),
            entry_path: "test.ds".into(),
            files: Vec::new(),
            expected_stdout: None,
            expected_diagnostic_contains: None,
            deka_json: None,
            packages: Vec::new(),
        }
    }

    fn run(ok: bool, stdout: &str, stderr: &str, transpile_failed: bool) -> RunResult {
        RunResult {
            ok,
            stdout: stdout.into(),
            stderr: stderr.into(),
            error: if ok { None } else { Some("boom".into()) },
            transpile_failed,
            diagnostics: if ok { Vec::new() } else { vec!["boom".into()] },
        }
    }

    #[test]
    fn http_metadata_migration_rejects_source_output_package_and_config_changes() {
        let mut fixture = case(Status::Pass, Stage::Run);
        fixture.slug = "http-invalid-url".into();
        fixture.packages = vec!["http".into(), "tcp".into(), "tls".into()];
        fixture.source="import { echo } from \"io\"\nimport { get } from \"http\"\necho(match (get(\"not a url\")) {\n  Ok(v) => \"ok\",\n  Err(e) => \"err\",\n})\n".into();
        fixture.expected_stdout = Some("err\n".into());
        assert!(native_http_fixture(&fixture));
        fixture.source.push(' ');
        assert!(!native_http_fixture(&fixture));
        fixture.source="import { echo } from \"io\"\nimport { get } from \"http\"\necho(match (get(\"not a url\")) {\n  Ok(v) => \"ok\",\n  Err(e) => \"err\",\n})\n".into();
        fixture.expected_stdout = Some("ok\n".into());
        assert!(!native_http_fixture(&fixture));
        fixture.expected_stdout = Some("err\n".into());
        fixture.packages.push("remote".into());
        assert!(!native_http_fixture(&fixture));
        fixture.packages.pop();
        fixture.deka_json = Some(serde_json::json!({"other":true}));
        assert!(!native_http_fixture(&fixture));
        fixture.deka_json = None;
        fixture.expected_diagnostic_contains = Some("oops".into());
        assert!(!native_http_fixture(&fixture));
    }

    fn legacy_bytes_case() -> Case {
        let mut fixture = case(Status::Pass, Stage::Run);
        fixture.slug = "packages-bytes-from-string-len".into();
        fixture.packages = vec!["bytes".into()];
        fixture.source = LEGACY_BYTES_SOURCE.into();
        fixture.expected_stdout = Some("5\n".into());
        fixture
    }
    #[test]
    fn bytes_metadata_migration_rejects_changed_source_output_and_real_packages() {
        assert!(native_bytes_fixture(&legacy_bytes_case()));
        let mut fixture = legacy_bytes_case();
        fixture.packages.push("unpublished-real-package".into());
        assert!(!native_bytes_fixture(&fixture));
        let scratch = tempfile::tempdir().unwrap();
        assert!(
            run_case(Path::new("missing-runtime"), &fixture, scratch.path())
                .unwrap_err()
                .contains("offline")
        );
        let mut fixture = legacy_bytes_case();
        fixture.source.push_str("echo(99)\n");
        assert!(!native_bytes_fixture(&fixture));
        let mut fixture = legacy_bytes_case();
        fixture.expected_stdout = Some("99\n".into());
        assert!(!native_bytes_fixture(&fixture));
        let mut fixture = legacy_bytes_case();
        fixture.slug = "packages-bytes-roundtrip".into();
        assert!(!native_bytes_fixture(&fixture));
        let mut fixture = legacy_bytes_case();
        fixture.status = Status::Fail;
        assert!(!native_bytes_fixture(&fixture));
        let mut fixture = legacy_bytes_case();
        fixture.deka_json = Some(serde_json::json!({"name":"new-metadata"}));
        assert!(!native_bytes_fixture(&fixture));
    }

    #[test]
    fn time_migration_is_exact_and_never_waives_execution_or_output() {
        let mut fixture = case(Status::Pass, Stage::Run);
        fixture.slug = "time-now-is-number".into();
        fixture.source =
            "import { echo } from \"io\"\nimport { now } from \"time\"\necho(string(now() > 0))\n"
                .into();
        fixture.expected_stdout = Some("true\n".into());
        fixture.packages = vec!["time".into()];
        assert!(native_time_fixture(&fixture));
        assert!(!evaluate(&fixture, &run(false, "", "unknown now", true)).is_empty());
        assert!(!evaluate(&fixture, &run(true, "false\n", "", false)).is_empty());
        fixture.source.push_str("// changed");
        assert!(!native_time_fixture(&fixture));
        fixture.source.truncate(fixture.source.len() - 10);
        fixture.packages.push("another-package".into());
        assert!(!native_time_fixture(&fixture));
        fixture.packages.pop();
        fixture.deka_json = Some(serde_json::json!({}));
        assert!(!native_time_fixture(&fixture));
        fixture.deka_json = None;
        fixture.files.push(("other.ds".into(), "".into()));
        assert!(!native_time_fixture(&fixture));
        fixture.files.clear();
        fixture.status = Status::Fail;
        assert!(!native_time_fixture(&fixture));
        fixture.status = Status::Pass;
        fixture.stage = Stage::Typecheck;
        assert!(!native_time_fixture(&fixture));
        fixture.stage = Stage::Run;
        fixture.expected_stdout = Some("different\n".into());
        assert!(!native_time_fixture(&fixture));
        fixture.expected_stdout = Some("true\n".into());
        fixture.expected_diagnostic_contains = Some("error".into());
        assert!(!native_time_fixture(&fixture));
        fixture.expected_diagnostic_contains = None;
        fixture.slug = "time-sleep-ms-one".into();
        assert!(!native_time_fixture(&fixture));
    }

    #[test]
    fn status_must_match() {
        assert!(evaluate(&case(Status::Pass, Stage::Run), &run(true, "", "", false)).is_empty());
        assert!(!evaluate(&case(Status::Pass, Stage::Run), &run(false, "", "", false)).is_empty());
    }

    #[test]
    fn parse_and_typecheck_stages_are_interchangeable() {
        let mut expected = case(Status::Fail, Stage::Typecheck);
        assert!(evaluate(&expected, &run(false, "", "x", true)).is_empty());
        expected.stage = Stage::Parse;
        assert!(evaluate(&expected, &run(false, "", "x", true)).is_empty());
        expected.stage = Stage::Run;
        assert!(!evaluate(&expected, &run(false, "", "x", true)).is_empty());
    }

    #[test]
    fn stdout_compares_exactly() {
        let mut expected = case(Status::Pass, Stage::Run);
        expected.expected_stdout = Some("42\n".into());
        assert!(evaluate(&expected, &run(true, "42\n", "", false)).is_empty());
        assert!(!evaluate(&expected, &run(true, "42", "", false)).is_empty());
    }

    #[test]
    fn diagnostic_contains_is_case_insensitive_and_reads_stderr() {
        let mut expected = case(Status::Fail, Stage::Run);
        expected.expected_diagnostic_contains = Some("Assignment Target".into());
        let result = run(false, "", "assignment target must be mutable", false);
        assert!(evaluate(&expected, &result).is_empty());
        expected.expected_diagnostic_contains = Some("something else".into());
        assert!(!evaluate(&expected, &result).is_empty());
    }

    #[test]
    fn native_diagnostics_fall_back_to_the_first_plain_line() {
        assert_eq!(
            parse_native_diagnostics("[security] ignored\ntype mismatch at 3:1\n"),
            vec!["type mismatch at 3:1".to_string()]
        );
    }

    #[test]
    fn loading_unguarded_empty_reduce_rejects_an_unimplemented_method_error() {
        let dir = tempfile::tempdir().unwrap();
        let fixture = dir
            .path()
            .join("data_types/array_reduce_empty_no_initial_fail");
        std::fs::create_dir_all(&fixture).unwrap();
        std::fs::write(fixture.join("test.fail.ds"), "const items = [];\n").unwrap();
        let cases = load_cases(dir.path());
        let [case] = cases.as_slice() else {
            panic!("fixture must load")
        };
        let wrong = run(
            false,
            "",
            "Run failed: method call requires a record",
            false,
        );
        assert!(
            evaluate(case, &wrong)
                .iter()
                .any(|reason| reason.starts_with("diagnostic:"))
        );
        let intended = run(
            false,
            "",
            "cannot reduce an empty array without an initial value",
            false,
        );
        assert!(evaluate(case, &intended).is_empty());
    }

    #[test]
    fn restored_crypto_requires_the_missing_method_error_not_an_absent_global() {
        let dir = tempfile::tempdir().unwrap();
        let original = "import { echo } from \"io\"\necho(crypto.nonexistent())\n";
        let obsolete = r#"{"expectedDiagnosticContains":"unknown identifier `crypto`"}"#;
        std::fs::write(dir.path().join("test.fail.ds"), original).unwrap();
        std::fs::write(dir.path().join("test.json"), obsolete).unwrap();
        let load = || load_case("error_globals", "crypto_method_missing_fail", dir.path()).unwrap();
        let case = load();
        assert!(!evaluate(&case, &run(false, "", "unknown identifier `crypto`", false)).is_empty());
        assert!(evaluate(&case,&run(false,"","cannot inspect opaque type `Crypto`; call a declared receiver method or summoned function",false)).is_empty());
        assert!(!evaluate(&case, &run(true, "", "", false)).is_empty());
        std::fs::write(
            dir.path().join("test.json"),
            r#"{"expectedDiagnosticContains":"new diagnostic"}"#,
        )
        .unwrap();
        assert_eq!(
            load().expected_diagnostic_contains.as_deref(),
            Some("new diagnostic")
        );
        std::fs::write(dir.path().join("test.json"), obsolete).unwrap();
        std::fs::write(dir.path().join("test.fail.ds"), "crypto.other()\n").unwrap();
        assert_eq!(
            load().expected_diagnostic_contains.as_deref(),
            Some("unknown identifier `crypto`")
        );
    }

    #[test]
    fn explicit_corpus_diagnostic_metadata_takes_precedence() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("test.fail.ds"), "const items = [];\n").unwrap();
        std::fs::write(
            dir.path().join("test.json"),
            r#"{"expectedDiagnosticContains":"new corpus expectation"}"#,
        )
        .unwrap();
        let case = load_case(
            "data_types",
            "array_reduce_empty_no_initial_fail",
            dir.path(),
        )
        .unwrap();
        assert!(evaluate(&case, &run(false, "", "new corpus expectation", false)).is_empty());
        assert!(
            !evaluate(
                &case,
                &run(
                    false,
                    "",
                    "cannot reduce an empty array without an initial value",
                    false
                )
            )
            .is_empty()
        );
    }
}
