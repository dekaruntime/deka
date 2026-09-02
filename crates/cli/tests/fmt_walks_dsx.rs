use std::fs;
use std::path::Path;
use std::process::Command;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

fn run(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(cli_bin())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run deka")
}

/// Regression test for #493: `fmt <dir>` must visit .dsx files, not just .ds.
/// Before the fix the walk matched only the "ds" extension, so .dsx files were
/// skipped silently — exit 0, no warning — and corpus normalization claimed
/// success over files it never attempted.
#[test]
fn fmt_directory_walks_ds_and_dsx() {
    let root = tempfile::tempdir().unwrap();
    let nested = root.path().join("nested");
    fs::create_dir_all(&nested).unwrap();
    fs::write(root.path().join("a.ds"), "export const a=1;\n").unwrap();
    fs::write(nested.join("b.dsx"), "export const b=2;\n").unwrap();

    let out = run(root.path(), &["fmt", "."]);
    assert!(
        out.status.success(),
        "fmt failed: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    let ds = fs::read_to_string(root.path().join("a.ds")).unwrap();
    let dsx = fs::read_to_string(nested.join("b.dsx")).unwrap();
    assert_eq!(ds, "export const a = 1\n", ".ds file was not reformatted");
    assert_eq!(
        dsx, "export const b = 2\n",
        ".dsx file was skipped by fmt directory walk"
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("visited 2 DekaScript file(s)"),
        "fmt did not report a visited count covering both extensions: {stderr}"
    );
}

#[test]
fn fmt_check_flags_dsx_that_would_change() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("page.dsx"), "export const b=2;\n").unwrap();

    let out = run(root.path(), &["fmt", "--check", "."]);
    assert!(
        !out.status.success(),
        "fmt --check must fail when a .dsx would be reformatted: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("page.dsx"),
        "fmt --check output did not name the .dsx file: {stderr}"
    );
    // --check must not modify the file
    let after = fs::read_to_string(root.path().join("page.dsx")).unwrap();
    assert_eq!(after, "export const b=2;\n");
}
