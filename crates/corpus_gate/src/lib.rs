//! Corpus gate library (deka#1214): enumerate testsuite-corpus cases,
//! materialize fixtures exactly like the corpus runner, execute `deka run`,
//! and evaluate the outcome against each case's expectation.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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

pub fn run_case(deka: &Path, case: &Case, scratch: &Path) -> Result<RunResult, String> {
    // The pinned JSON corpus predates native typed JSON and still names its
    // old package. These fixtures now run against the built-in language path;
    // source compilation/execution remains mandatory. Other packages fail closed.
    let native_json = case.slug.starts_with("json-")
        && case.packages.iter().all(|package| package == "json")
        && std::iter::once(case.source.as_str())
            .chain(case.files.iter().map(|(_, source)| source.as_str()))
            .all(native_json_source);
    if !case.packages.is_empty() && !native_json {
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
        None if !case.packages.is_empty() && !native_json => PACKAGE_DEKA_JSON.to_string(),
        None => DEFAULT_DEKA_JSON.to_string(),
    };
    std::fs::write(directory.join("deka.json"), deka_json).unwrap();

    let mut child = Command::new(deka)
        .args(["run", &entry])
        .current_dir(&directory)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn {}: {e}", deka.display()))?;
    let deadline = std::time::Instant::now() + RUN_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{} timed out after {RUN_TIMEOUT:?}", case.slug));
            }
            Err(e) => return Err(format!("failed to wait for {}: {e}", case.slug)),
        }
    }
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let raw_stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let stderr: String = raw_stderr
        .lines()
        .filter(|line| !line.starts_with("[security]"))
        .collect::<Vec<_>>()
        .join("\n");
    let failed = !output.status.success();
    let ran_in_isolate = raw_stderr.contains("Run failed:") || !stdout.is_empty();
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
    Ok(RunResult {
        ok: !failed,
        stdout,
        stderr,
        error,
        transpile_failed: failed && !ran_in_isolate,
        diagnostics,
    })
}

fn stage_matches(expected: Stage, actual: Stage) -> bool {
    if expected == actual {
        return true;
    }
    // The native isolate does not distinguish typecheck from parse on
    // compile failure.
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
