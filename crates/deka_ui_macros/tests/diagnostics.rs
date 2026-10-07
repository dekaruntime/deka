//! Compile-fail tests equivalent to trybuild, using only release-mode builds.
use std::{path::PathBuf, process::Command};

#[test]
fn errors_point_at_the_users_markup_lines() {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = crate_dir.parent().unwrap().parent().unwrap();
    let target = root.join(".target/ui-diagnostics");
    std::fs::create_dir_all(&target).unwrap();
    for (case, line, message) in [
        ("unknown", 6, "Missing"),
        ("missing", 8, "build"),
        ("wrong", 9, "mismatched types"),
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
            .arg(crate_dir.join("tests/ui/Cargo.toml"))
            .args(["--bin", case])
            .env("CARGO_TARGET_DIR", &target)
            .output()
            .unwrap();
        std::fs::write(target.join(format!("{case}.jsonl")), &output.stdout).unwrap();
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
}
