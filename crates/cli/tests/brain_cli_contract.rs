use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn brain_bin() -> PathBuf {
    if let Ok(path) = env::var("BRAIN_BIN") {
        return PathBuf::from(path);
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .ancestors()
        .nth(2)
        .expect("cli crate should live under runtime/crates/cli")
        .join("target")
        .join("debug")
        .join("brain")
}

fn run_brain(args: &[&str]) -> std::process::Output {
    let bin = brain_bin();
    assert!(
        bin.exists(),
        "blocked: source scaffold absent, expected brain binary at {} or BRAIN_BIN",
        bin.display()
    );

    Command::new(bin)
        .args(args)
        .output()
        .expect("brain command should execute")
}

fn output_text(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
#[ignore = "blocked: source scaffold absent, see deka#125"]
fn help_exposes_brain_retrieval_connector_plugin_and_auth_surface() {
    let output = run_brain(&["--help"]);
    assert!(output.status.success(), "{}", output_text(&output));

    let text = output_text(&output);
    for command in [
        "ask", "search", "get", "connect", "plugin", "self", "login", "logout", "whoami",
    ] {
        assert!(
            text.contains(command),
            "brain --help should list `{command}`; output:\n{text}"
        );
    }
}

#[test]
#[ignore = "blocked: source scaffold absent, see deka#125"]
fn self_help_exposes_shared_auth_trio() {
    let output = run_brain(&["self", "--help"]);
    assert!(output.status.success(), "{}", output_text(&output));

    let text = output_text(&output);
    for command in ["login", "logout", "whoami"] {
        assert!(
            text.contains(command),
            "brain self --help should list `{command}`; output:\n{text}"
        );
    }
}

#[test]
#[ignore = "blocked: source scaffold absent, see deka#125"]
fn command_surface_fails_closed_without_config_instead_of_stub_success() {
    for args in [
        &["ask", "what changed?"][..],
        &["search", "checkout"][..],
        &["get", "products/tana.md"][..],
        &["connect", "list"][..],
        &["plugin", "list"][..],
        &["whoami"][..],
        &["self", "whoami"][..],
    ] {
        let output = run_brain(args);
        assert!(
            !output.status.success(),
            "brain {args:?} should fail closed without auth/API config, not return stub success:\n{}",
            output_text(&output)
        );
    }
}

#[test]
#[ignore = "blocked: source scaffold absent, see deka#125"]
fn cli_source_stays_modular_and_below_elephant_file_threshold() {
    let root = env::var("BRAIN_CLI_SRC")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(2)
                .expect("cli crate should live under runtime/crates/cli")
                .join("crates")
                .join("brain-cli")
                .join("src")
        });

    assert!(
        root.join("main.rs").exists(),
        "blocked: source scaffold absent, expected {}",
        root.join("main.rs").display()
    );
    assert!(
        root.join("cli").is_dir(),
        "brain CLI should use the deka modular cli/ directory pattern"
    );

    for command in ["ask", "search", "get", "connect", "plugin", "self_cmd"] {
        assert!(
            root.join("cli").join(format!("{command}.rs")).exists()
                || root.join("cli").join(command).join("mod.rs").exists(),
            "brain CLI should keep `{command}` in its own command module"
        );
    }

    assert_file_under_threshold(&root.join("main.rs"));
    assert_rs_files_under_threshold(&root);
}

fn assert_rs_files_under_threshold(root: &Path) {
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        for entry in std::fs::read_dir(&path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()))
        {
            let entry = entry.expect("directory entry should be readable");
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                assert_file_under_threshold(&path);
            }
        }
    }
}

fn assert_file_under_threshold(path: &Path) {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
    let line_count = text.lines().count();
    assert!(
        line_count <= 1000,
        "{} has {line_count} lines; split before crossing the 1000-line threshold",
        path.display()
    );
}
