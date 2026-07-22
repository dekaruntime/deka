use std::{fs, process::Command};

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

fn git(repo: &std::path::Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .unwrap_or_else(|err| panic!("run git {}: {err}", args.join(" ")));
    assert!(
        output.status.success(),
        "git {} failed:\nstdout: {}\nstderr: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn publish_command_rejects_artifact_with_vendored_php_modules() {
    let package = tempfile::tempdir().expect("create package source tree");
    let root = package.path();
    git(root, &["init"]);
    git(root, &["config", "user.email", "test@tana.gg"]);
    git(
        root,
        &["config", "user.name", "Deka publish integration test"],
    );

    fs::write(
        root.join("deka.json"),
        r#"{"name":"@tana/publish-fixture","version":"1.0.0"}"#,
    )
    .expect("write package manifest");
    let vendored_module = root
        .join("src")
        .join("generated")
        .join("php_modules")
        .join("nested")
        .join("index.phpx");
    fs::create_dir_all(vendored_module.parent().expect("module parent"))
        .expect("create vendored module directory");
    fs::write(&vendored_module, "export const nested = true;").expect("write vendored module");

    // A harmless-looking symlink must not make a vendored dependency tree
    // publishable. Git records symlinks as blobs, which is why this needs the
    // real publish path rather than a filesystem-only assertion.
    #[cfg(unix)]
    std::os::unix::fs::symlink("src/generated/php_modules", root.join("assets"))
        .expect("create innocuous symlink into vendored modules");

    git(root, &["add", "."]);
    git(root, &["commit", "-m", "package fixture"]);

    let output = Command::new(cli_bin())
        .current_dir(root)
        .args([
            "publish",
            "--dry-run",
            "--yes",
            "--token",
            "test-token",
            "--registry-url",
            "http://127.0.0.1:9",
        ])
        .output()
        .expect("run deka publish");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        stderr.contains("publish rejected") && stderr.contains("php_modules"),
        "deka publish must reject the Git artifact before contacting the registry; \\
         status: {}\\nstderr: {}",
        output.status,
        stderr,
    );
}
