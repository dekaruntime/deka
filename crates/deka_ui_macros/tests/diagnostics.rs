//! Compile-fail tests equivalent to trybuild, using only release-mode builds.
//! This test runs cargo fetch for a separate locked fixture workspace and needs
//! network access when its dependencies are not already cached. Checks run offline.
use std::{path::PathBuf, process::Command};

#[test]
fn errors_point_at_the_users_markup_lines() {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = crate_dir.parent().unwrap().parent().unwrap();
    let manifest = crate_dir.join("tests/ui/Cargo.toml");
    let target = root.join(".target/ui-diagnostics");
    std::fs::create_dir_all(&target).unwrap();
    // The fixture is a separate workspace: the outer build only fetches versions
    // from the root lockfile. Prepare its own locked dependencies before checking offline.
    let fetched = Command::new(env!("CARGO"))
        .args(["fetch", "--locked", "--manifest-path"])
        .arg(&manifest)
        .output()
        .unwrap();
    assert!(
        fetched.status.success(),
        "UI fixture dependency fetch failed: {}\nstderr:\n{}\nstdout:\n{}",
        fetched.status,
        String::from_utf8_lossy(&fetched.stderr),
        String::from_utf8_lossy(&fetched.stdout)
    );
    for (case, line, message) in [
        (
            "unknown",
            6,
            "cannot find function, tuple struct or tuple variant `Missing` in this scope",
        ),
        (
            "missing",
            8,
            "missing required prop `title` for component `Card`",
        ),
        ("wrong", 9, "mismatched types"),
        (
            "interpolation",
            4,
            "text interpolation requires an identifier (brace at line 4, column 40)",
        ),
    ] {
        let output = Command::new(env!("CARGO"))
            .args([
                "check",
                "--release",
                "--locked",
                "--offline",
                "--message-format=json",
                "--manifest-path",
            ])
            .arg(&manifest)
            .args(["--bin", case])
            .env("CARGO_TARGET_DIR", &target)
            .output()
            .unwrap();
        std::fs::write(target.join(format!("{case}.jsonl")), &output.stdout).unwrap();
        std::fs::write(target.join(format!("{case}.stderr")), &output.stderr).unwrap();
        assert_eq!(
            output.status.code(),
            Some(101),
            "{case}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let messages: Vec<serde_json::Value> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        let expected_file = format!("src/bin/{case}.rs");
        let diagnostics: Vec<_> = messages
            .iter()
            .filter(|entry| {
                entry["reason"] == "compiler-message" && entry["message"]["level"] == "error"
            })
            .collect();
        assert!(
            !diagnostics.is_empty(),
            "{case}: nested cargo check exited {} without compiler errors\nstderr:\n{}\nstdout:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(
            diagnostics.iter().any(|entry| {
                entry["message"]["message"]
                    .as_str()
                    .is_some_and(|text| text.contains(message))
                    && entry["message"]["spans"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|span| {
                            span["is_primary"] == true
                                && span["file_name"]
                                    .as_str()
                                    .is_some_and(|file| file.ends_with(&expected_file))
                                && span["line_start"] == line
                        })
            }),
            "{case}: expected primary error on markup line {line}: {diagnostics:#?}"
        );
        println!(
            "{case}: expected compile failure 101; primary diagnostic on user's markup line {line}"
        );
    }
    let output = Command::new(env!("CARGO"))
        .args([
            "run",
            "--release",
            "--locked",
            "--offline",
            "--manifest-path",
        ])
        .arg(&manifest)
        .args(["--bin", "hygiene"])
        .env("CARGO_TARGET_DIR", &target)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "hygiene fixture failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "Global Title Value:local"
    );
}
