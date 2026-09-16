//! `deka self doctor` (deka#1102): diagnose install problems — shadowed
//! binaries, version skew, mismatched dsc — as a human-readable report.
//!
//! Doctor only *reports* state; it never changes how any other command
//! resolves the compiler or itself. The resolution logic below mirrors, in
//! order, the production lookup it describes:
//!
//! - [`compiler::dsc::find_cli_dsc`] (CLI): `DEKA_NO_DSC`, then `DEKA_DSC`,
//!   then [`compiler::dsc::find_dsc`], then `PATH`.
//! - [`compiler::dsc::find_dsc`] (runtime/pool): the `dsc` file beside the
//!   running executable, then the repository's `target/release/dsc` for dev
//!   builds. `crates/runtime/src/dsc_transpile.rs` additionally honors
//!   `DEKA_DSC` as a legacy fallback for direct runtime callers.
//!
//! Exit code: 0 when the install is tidy or merely untidy, 1 when something
//! will actually break a build (no dsc resolvable, deka/dsc version
//! mismatch, project pin skew against the running binary).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use deka_cli_core::Context;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Severity {
    Warn,
    Error,
}

impl Severity {
    fn label(self) -> &'static str {
        match self {
            Severity::Warn => "warning",
            Severity::Error => "ERROR",
        }
    }
}

#[derive(Debug)]
struct Finding {
    severity: Severity,
    message: String,
}

/// The environment doctor inspects. Captured from the process up front so
/// the report logic stays pure and unit-testable (no `std::env` reads below
/// this point).
struct DoctorEnv {
    cwd: PathBuf,
    running_exe: PathBuf,
    running_version: String,
    path_var: Option<OsString>,
    deka_dsc: Option<OsString>,
    deka_no_dsc: bool,
}

impl DoctorEnv {
    fn from_process(cwd: PathBuf) -> Self {
        Self {
            cwd,
            running_exe: std::env::current_exe().unwrap_or_else(|_| PathBuf::from("deka")),
            running_version: env!("CARGO_PKG_VERSION").to_string(),
            path_var: std::env::var_os("PATH"),
            deka_dsc: std::env::var_os("DEKA_DSC"),
            deka_no_dsc: std::env::var_os("DEKA_NO_DSC").is_some(),
        }
    }
}

/// One `deka` / `dsc` discovered on PATH, with its self-reported version.
#[derive(Debug)]
struct PathEntry {
    path: PathBuf,
    version: Option<String>,
}

/// How the resolved dsc was found. Labels mirror the production lookup.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum HowFound {
    /// `DEKA_NO_DSC` is set: no dsc will be used.
    Disabled,
    /// `DEKA_DSC` pointed at a file.
    EnvVar,
    /// `dsc` sitting beside the running deka binary.
    Sibling,
    /// Repository `target/release/dsc` (dev builds).
    RepoPinned,
    /// First `dsc` on PATH.
    Path,
    /// Nothing found anywhere.
    None,
}

impl HowFound {
    fn label(self) -> &'static str {
        match self {
            HowFound::Disabled => "disabled (DEKA_NO_DSC is set)",
            HowFound::EnvVar => "DEKA_DSC",
            HowFound::Sibling => "sibling of the running deka binary",
            HowFound::RepoPinned => "repository pinned build (target/release/dsc)",
            HowFound::Path => "PATH",
            HowFound::None => "not found",
        }
    }
}

#[derive(Debug)]
struct Pairing {
    how: HowFound,
    path: Option<PathBuf>,
    version: Option<String>,
    /// Set when an override names something unusable (mirrors the error
    /// branch of `find_cli_dsc`).
    problem: Option<String>,
}

/// A project-level version pin discovered on disk.
#[derive(Debug)]
struct ProjectPin {
    source: String,
    version: String,
}

pub fn cmd(context: &Context) {
    let env = DoctorEnv::from_process(context.env.cwd.clone());
    let (lines, error_count) = render_report(&env);
    for line in lines {
        println!("{line}");
    }
    if error_count > 0 {
        println!();
        println!(
            "doctor: {error_count} problem(s) that will break a build (exit 1)"
        );
        std::process::exit(1);
    }
}

fn render_report(env: &DoctorEnv) -> (Vec<String>, usize) {
    let mut lines = Vec::new();
    let mut findings: Vec<Finding> = Vec::new();
    lines.push("deka self doctor".to_string());
    lines.push(String::new());

    // 1. Every deka and dsc on PATH, in resolution order, with versions.
    lines.push("binaries on PATH:".to_string());
    for (tool, entries) in [
        ("deka", path_entries(env, "deka")),
        ("dsc", path_entries(env, "dsc")),
    ] {
        lines.push(format!("  {tool}:"));
        if entries.is_empty() {
            lines.push("    (none on PATH)".to_string());
        }
        for (index, entry) in entries.iter().enumerate() {
            let version = entry.version.as_deref().unwrap_or("version unavailable");
            let marker = if index == 0 { "   <- first on PATH, wins" } else { "" };
            lines.push(format!(
                "    {}    {version}{marker}",
                entry.path.display()
            ));
        }
    }
    // Several agreeing installs are untidy, not fatal.
    let deka_count = path_entries(env, "deka").len();
    if deka_count > 1 {
        findings.push(Finding {
            severity: Severity::Warn,
            message: format!("{deka_count} deka binaries on PATH; the first one shadows the rest"),
        });
    }
    lines.push(String::new());

    // 2. deka/dsc pairing for the running binary.
    lines.push("dsc pairing:".to_string());
    let pairing = resolve_pairing(env);
    match &pairing.path {
        Some(path) => lines.push(format!(
            "  resolved: {} (via {})",
            path.display(),
            pairing.how.label()
        )),
        None => lines.push(format!("  resolved: none ({})", pairing.how.label())),
    }
    if let Some(problem) = &pairing.problem {
        findings.push(Finding {
            severity: Severity::Error,
            message: problem.clone(),
        });
    }
    match pairing.how {
        HowFound::Disabled => {
            findings.push(Finding {
                severity: Severity::Warn,
                message: "DEKA_NO_DSC is set: every command that needs dsc will fail"
                    .to_string(),
            });
        }
        HowFound::None => {
            findings.push(Finding {
                severity: Severity::Error,
                message: "no dsc resolvable: install dsc next to deka, set DEKA_DSC, or put it on PATH".to_string(),
            });
        }
        _ => {
            if let Some(version) = &pairing.version {
                lines.push(format!("  dsc --version: {version}"));
                lines.push(format!("  running deka:  {}", env.running_version));
                match compare_versions(version, &env.running_version) {
                    Some(std::cmp::Ordering::Equal) => {}
                    Some(order) => {
                        let relation = match order {
                            std::cmp::Ordering::Less => "older than",
                            std::cmp::Ordering::Greater => "NEWER than",
                            std::cmp::Ordering::Equal => unreachable!(),
                        };
                        findings.push(Finding {
                            severity: Severity::Error,
                            message: format!(
                                "dsc {version} is {relation} the running deka {} — a deka/dsc version mismatch breaks builds",
                                env.running_version
                            ),
                        });
                    }
                    None => findings.push(Finding {
                        severity: Severity::Warn,
                        message: format!(
                            "could not compare dsc version '{version}' with deka version '{}'",
                            env.running_version
                        ),
                    }),
                }
            } else {
                findings.push(Finding {
                    severity: Severity::Warn,
                    message: format!(
                        "resolved dsc at {} did not report a version",
                        pairing.path.as_ref().map(|p| p.display().to_string()).unwrap_or_default()
                    ),
                });
            }
        }
    }
    lines.push(String::new());

    // 3. Project pin vs the running binary.
    lines.push("project vs running:".to_string());
    lines.push(format!("  running deka: {}", env.running_version));
    let pins = project_pins(&env.cwd);
    if pins.is_empty() {
        lines.push("  no project pin found (no deka.json or node_modules/@dekaruntime/deka here)".to_string());
    }
    for pin in &pins {
        lines.push(format!("  {}: {}", pin.source, pin.version));
        match compare_versions(&pin.version, &env.running_version) {
            Some(std::cmp::Ordering::Equal) => {}
            Some(order) => {
                let (relation, which) = match order {
                    std::cmp::Ordering::Less => ("OLDER than", "project pin is older"),
                    std::cmp::Ordering::Greater => ("NEWER than", "running binary is older"),
                    std::cmp::Ordering::Equal => unreachable!(),
                };
                findings.push(Finding {
                    severity: Severity::Error,
                    message: format!(
                        "{}: {} is {relation} the running deka {} ({which}) — version skew against the project pin breaks builds",
                        pin.source, pin.version, env.running_version
                    ),
                });
            }
            None => findings.push(Finding {
                severity: Severity::Warn,
                message: format!(
                    "could not compare {} version '{}' with deka version '{}'",
                    pin.source, pin.version, env.running_version
                ),
            }),
        }
    }
    lines.push(String::new());

    // 4. Environment overrides in play.
    lines.push("environment overrides:".to_string());
    match &env.deka_dsc {
        Some(value) => {
            let valid = PathBuf::from(value).is_file();
            lines.push(format!(
                "  DEKA_DSC={} ({})",
                value.to_string_lossy(),
                if valid { "in play: file exists" } else { "set but not a file" }
            ));
        }
        None => lines.push("  DEKA_DSC: not set".to_string()),
    }
    lines.push(format!(
        "  DEKA_NO_DSC: {}",
        if env.deka_no_dsc { "set" } else { "not set" }
    ));
    lines.push(String::new());

    let error_count = findings
        .iter()
        .filter(|f| f.severity == Severity::Error)
        .count();
    let warn_count = findings
        .iter()
        .filter(|f| f.severity == Severity::Warn)
        .count();
    if findings.is_empty() {
        lines.push("summary: no problems found".to_string());
    } else {
        lines.push("findings:".to_string());
        for finding in &findings {
            lines.push(format!("  [{}] {}", finding.severity.label(), finding.message));
        }
        lines.push(format!(
            "summary: {error_count} error(s), {warn_count} warning(s)"
        ));
    }
    (lines, error_count)
}

/// Every `tool` on PATH in resolution order, with the version each binary
/// self-reports. Reads nothing from the process environment.
fn path_entries(env: &DoctorEnv, tool: &str) -> Vec<PathEntry> {
    let mut entries = Vec::new();
    let mut seen: Vec<PathBuf> = Vec::new();
    let Some(path_var) = &env.path_var else {
        return entries;
    };
    for dir in std::env::split_paths(path_var) {
        let candidate = dir.join(tool);
        if !candidate.is_file() {
            continue;
        }
        // The same directory can appear twice on PATH; report it once.
        if seen.iter().any(|p| p == &candidate) {
            continue;
        }
        seen.push(candidate.clone());
        entries.push(PathEntry {
            version: probe_version(&candidate),
            path: candidate,
        });
    }
    entries
}

/// First line of `<bin> --version`, trimmed, when the binary reports one.
fn probe_version(bin: &Path) -> Option<String> {
    let output = Command::new(bin)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = if output.stdout.is_empty() {
        output.stderr
    } else {
        output.stdout
    };
    String::from_utf8_lossy(&text)
        .lines()
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
}

/// Which dsc the running deka would use, and how it got there. Mirrors
/// `compiler::dsc::find_cli_dsc` branch-for-branch so the report describes
/// production resolution exactly.
fn resolve_pairing(env: &DoctorEnv) -> Pairing {
    if env.deka_no_dsc {
        return Pairing {
            how: HowFound::Disabled,
            path: None,
            version: None,
            problem: None,
        };
    }
    if let Some(value) = &env.deka_dsc {
        let path = PathBuf::from(value);
        if path.is_file() {
            let version = probe_version(&path);
            return Pairing {
                how: HowFound::EnvVar,
                path: Some(path),
                version,
                problem: None,
            };
        }
        return Pairing {
            how: HowFound::None,
            path: None,
            version: None,
            problem: Some(format!(
                "DEKA_DSC is set to {} but that path is not a file",
                path.display()
            )),
        };
    }

    // compiler::dsc::find_dsc: sibling of the running executable, then the
    // repository's target/release/dsc (dev builds).
    if let Some(dir) = env.running_exe.parent() {
        let sibling = dir.join("dsc");
        if sibling.is_file() {
            let version = probe_version(&sibling);
            return Pairing {
                how: HowFound::Sibling,
                path: Some(sibling),
                version,
                problem: None,
            };
        }
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if let Some(root) = manifest.ancestors().nth(2) {
        let pinned = root.join("target/release/dsc");
        if pinned.is_file() {
            let version = probe_version(&pinned);
            return Pairing {
                how: HowFound::RepoPinned,
                path: Some(pinned),
                version,
                problem: None,
            };
        }
    }

    // Fall back to PATH, honoring the injected PATH only.
    if let Some(path_var) = &env.path_var {
        for dir in std::env::split_paths(path_var) {
            let candidate = dir.join("dsc");
            if candidate.is_file() {
                let version = probe_version(&candidate);
                return Pairing {
                    how: HowFound::Path,
                    path: Some(candidate),
                    version,
                    problem: None,
                };
            }
        }
    }
    Pairing {
        how: HowFound::None,
        path: None,
        version: None,
        problem: None,
    }
}

/// Project-level pins: `node_modules/@dekaruntime/deka/package.json`'s
/// version, and a `"deka"` string field in `deka.json` when one is present.
fn project_pins(cwd: &Path) -> Vec<ProjectPin> {
    let mut pins = Vec::new();
    let npm_manifest = cwd
        .join("node_modules")
        .join("@dekaruntime")
        .join("deka")
        .join("package.json");
    if let Ok(text) = std::fs::read_to_string(&npm_manifest) {
        if let Some(version) = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|value| value.get("version")?.as_str().map(str::to_string))
        {
            pins.push(ProjectPin {
                source: "node_modules/@dekaruntime/deka".to_string(),
                version,
            });
        }
    }
    let deka_json = cwd.join("deka.json");
    if let Ok(text) = std::fs::read_to_string(&deka_json) {
        if let Some(pin) = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|value| value.get("deka")?.as_str().map(str::to_string))
        {
            pins.push(ProjectPin {
                source: "deka.json \"deka\"".to_string(),
                version: pin,
            });
        }
    }
    pins
}

/// Compare two version strings as (major, minor, patch). The strings may be
/// free-form version lines (`dsc [version 0.8.1]`); the first `N.N.N`
/// substring is the version. Returns None when either side has no triple.
fn compare_versions(a: &str, b: &str) -> Option<std::cmp::Ordering> {
    extract_triple(a)?.partial_cmp(&extract_triple(b)?)
}

fn extract_triple(text: &str) -> Option<(u64, u64, u64)> {
    let bytes = text.as_bytes();
    for start in 0..bytes.len() {
        if !bytes[start].is_ascii_digit() {
            continue;
        }
        let end = (start..bytes.len())
            .find(|&i| !(bytes[i].is_ascii_digit() || bytes[i] == b'.'))
            .unwrap_or(bytes.len());
        let candidate = &text[start..end];
        let mut parts = candidate.split('.');
        let parse = |part: Option<&str>| part?.parse().ok();
        if let (Some(major), Some(minor), Some(patch), None) = (
            parse(parts.next()),
            parse(parts.next()),
            parse(parts.next()),
            parts.next(),
        ) {
            return Some((major, minor, patch));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn executable_script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
        path
    }

    fn fixture_env(home: &tempfile::TempDir, path_dirs: &[&Path]) -> DoctorEnv {
        DoctorEnv {
            cwd: home.path().join("project"),
            running_exe: home.path().join("bin/deka"),
            running_version: "0.53.2".to_string(),
            path_var: Some(std::env::join_paths(path_dirs.iter().map(PathBuf::from)).unwrap()),
            deka_dsc: None,
            deka_no_dsc: false,
        }
    }

    #[test]
    fn path_entries_report_resolution_order_with_versions() {
        let home = tempfile::tempdir().unwrap();
        let first = home.path().join("first");
        let second = home.path().join("second");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        executable_script(&first, "deka", "#!/bin/sh\necho 'deka [version 0.53.4]'\n");
        executable_script(&second, "deka", "#!/bin/sh\necho 'deka [version 0.53.1]'\n");

        let env = fixture_env(&home, &[&first, &second]);
        let entries = path_entries(&env, "deka");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].path, first.join("deka"));
        assert_eq!(entries[0].version.as_deref(), Some("deka [version 0.53.4]"));
        assert_eq!(entries[1].path, second.join("deka"));
        assert_eq!(entries[1].version.as_deref(), Some("deka [version 0.53.1]"));

        // A tool absent from every dir reports no entries.
        assert!(path_entries(&env, "dsc").is_empty());
    }

    #[test]
    fn pairing_prefers_deka_dsc_then_sibling_then_path() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join("bin");
        let sibling_bin = home.path().join("siblingbin");
        let path_dir = home.path().join("pathbin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&sibling_bin).unwrap();
        std::fs::create_dir_all(&path_dir).unwrap();
        // fixture_env points running_exe at home/bin/deka; the sibling case
        // moves it into siblingbin, where this dsc sits.
        let sibling = executable_script(&sibling_bin, "dsc", "#!/bin/sh\necho 'dsc 0.53.2'\n");
        let on_path = executable_script(&path_dir, "dsc", "#!/bin/sh\necho 'dsc 0.53.0'\n");
        let via_env = executable_script(&bin, "other-dsc", "#!/bin/sh\necho 'dsc 0.53.9'\n");

        // PATH alone resolves via PATH.
        let env = fixture_env(&home, &[&path_dir]);
        let pairing = resolve_pairing(&env);
        assert_eq!(pairing.how, HowFound::Path);
        assert_eq!(pairing.path.as_deref(), Some(on_path.as_path()));

        // A sibling beats PATH.
        let env = fixture_env(&home, &[&path_dir]);
        let env = DoctorEnv {
            running_exe: sibling_bin.join("deka"),
            ..env
        };
        let pairing = resolve_pairing(&env);
        assert_eq!(pairing.how, HowFound::Sibling);
        assert_eq!(pairing.path.as_deref(), Some(sibling.as_path()));

        // DEKA_DSC beats the sibling; a bad path is an error problem.
        let env = DoctorEnv {
            running_exe: sibling_bin.join("deka"),
            deka_dsc: Some(via_env.as_os_str().to_os_string()),
            ..env
        };
        let pairing = resolve_pairing(&env);
        assert_eq!(pairing.how, HowFound::EnvVar);
        assert_eq!(pairing.path.as_deref(), Some(via_env.as_path()));

        let env = DoctorEnv {
            deka_dsc: Some(home.path().join("missing").as_os_str().to_os_string()),
            ..env
        };
        let pairing = resolve_pairing(&env);
        assert_eq!(pairing.how, HowFound::None);
        assert!(pairing.problem.is_some());

        // DEKA_NO_DSC disables resolution entirely.
        let env = DoctorEnv {
            deka_no_dsc: true,
            ..env
        };
        let pairing = resolve_pairing(&env);
        assert_eq!(pairing.how, HowFound::Disabled);
    }

    #[test]
    fn project_pins_read_npm_manifest_and_deka_json() {
        let home = tempfile::tempdir().unwrap();
        let project = home.path().join("project");
        let npm = project.join("node_modules/@dekaruntime/deka");
        std::fs::create_dir_all(&npm).unwrap();
        std::fs::write(
            npm.join("package.json"),
            r#"{"name":"@dekaruntime/deka","version":"0.53.4"}"#,
        )
        .unwrap();
        std::fs::write(project.join("deka.json"), r#"{"name":"app","deka":"0.53.0"}"#).unwrap();

        let pins = project_pins(&project);
        assert_eq!(pins.len(), 2);
        assert_eq!(pins[0].source, "node_modules/@dekaruntime/deka");
        assert_eq!(pins[0].version, "0.53.4");
        assert_eq!(pins[1].source, "deka.json \"deka\"");
        assert_eq!(pins[1].version, "0.53.0");

        // No project on disk: no pins, no panic.
        let empty = tempfile::tempdir().unwrap();
        assert!(project_pins(empty.path()).is_empty());
    }

    #[test]
    fn version_comparison_parses_triples() {
        assert_eq!(
            compare_versions("0.53.4", "0.53.2"),
            Some(std::cmp::Ordering::Greater)
        );
        assert_eq!(
            compare_versions("0.53.1", "0.53.2"),
            Some(std::cmp::Ordering::Less)
        );
        assert_eq!(
            compare_versions("0.53.2", "0.53.2"),
            Some(std::cmp::Ordering::Equal)
        );
        assert_eq!(compare_versions("dev", "0.53.2"), None);
        // Free-form version lines: the N.N.N substring is the version.
        assert_eq!(
            compare_versions("dsc [version 0.53.4]", "deka [version 0.53.2]"),
            Some(std::cmp::Ordering::Greater)
        );
        assert_eq!(compare_versions("no version here", "0.53.2"), None);
    }
}
