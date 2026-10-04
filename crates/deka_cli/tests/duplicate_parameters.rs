use std::{fs, process::Command};

#[test]
fn check_and_run_stop_at_the_duplicate_parameter_before_effects() {
    let project = tempfile::tempdir().unwrap();
    fs::write(project.path().join("app.ds"),
        "import { echo } from \"io\"\nfn add(x: number, x: number) number {\n  return x + x\n}\necho(string(add(1, 2)))\n",
    ).unwrap();
    for command in ["check", "run"] {
        let output = Command::new(env!("CARGO_BIN_EXE_deka"))
            .current_dir(project.path())
            .args([command, "app.ds"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{command}: {output:?}");
        assert!(output.stdout.is_empty(), "{command}: {output:?}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("2:19: duplicate parameter `x`"), "{stderr}");
        assert_eq!(
            stderr.matches("duplicate parameter `x`").count(),
            1,
            "{stderr}"
        );
    }
}
