use std::{fs, process::Command};

#[test]
fn check_and_run_reject_duplicates_at_the_declaration_before_any_effects() {
    let project = tempfile::tempdir().unwrap();
    for source in [
        "import { echo } from \"io\";\nlet x = 1;\nconst x = 2;\necho(\"should not execute\");",
        "import { echo } from \"io\";\nconst x = 1;\nconst x = 2;\nfn read() number {return x;}\necho(string(read()));",
        "import { echo } from \"io\";\nconst x = 1;\nexport const x = 2;\necho(\"should not execute\");",
    ] {
        fs::write(project.path().join("app.ds"), source).unwrap();
        for command in ["check", "run"] {
            let output = Command::new(env!("CARGO_BIN_EXE_deka"))
                .current_dir(project.path())
                .args([command, "app.ds"])
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(1), "{command}: {output:?}");
            assert!(output.stdout.is_empty(), "{command}: {output:?}");
            let stderr = String::from_utf8(output.stderr).unwrap();
            assert!(
                stderr.contains("3:1: duplicate binding `x` in this scope"),
                "{stderr}"
            );
            assert_eq!(
                stderr.matches("duplicate binding `x`").count(),
                1,
                "{stderr}"
            );
        }
    }
}
