#![cfg(all(feature = "compiler", feature = "host"))]
//! Package consumption from ds_modules (deka#1212): bare specifiers resolve
//! against deka.json dependencies and deka.lock, with named errors. No network.
use deka_vm::*;
use std::path::Path;

fn write(dir: &Path, name: &str, content: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, content).unwrap();
    path
}

/// A project with `mathpkg` 1.2.3 installed: deka.json entry plus a helper
/// reached through a relative import inside the package.
fn project_with_mathpkg(dir: &Path) -> std::path::PathBuf {
    write(
        dir,
        "deka.json",
        "{\"name\":\"app\",\"version\":\"0.1.0\",\"dependencies\":{\"mathpkg\":\"1.2.3\"}}\n",
    );
    write(
        dir,
        "ds_modules/mathpkg/deka.json",
        "{\"name\":\"mathpkg\",\"version\":\"1.2.3\",\"entry\":\"main.ds\"}\n",
    );
    write(
        dir,
        "ds_modules/mathpkg/main.ds",
        "import { base } from \"./consts.ds\";\nexport fn answer() number { return base + 2; }\n",
    );
    write(
        dir,
        "ds_modules/mathpkg/consts.ds",
        "export const base = 40;\n",
    )
}

async fn result(path: &Path) -> Result<HostValue> {
    let program = compiler::compile_file(path, &Hosts::default(), Some("main"))?;
    Vm::new(program, Hosts::default())?.run().await
}

#[tokio::test]
async fn hand_placed_package_imports_and_runs() {
    let dir = tempfile::tempdir().unwrap();
    project_with_mathpkg(dir.path());
    let app = write(
        dir.path(),
        "app.ds",
        "import { answer } from \"mathpkg\";\nfn main() number { return answer(); }\n",
    );
    assert_eq!(result(&app).await.unwrap(), HostValue::Number(42.));
}

#[tokio::test]
async fn scoped_package_layout_resolves() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "deka.json",
        "{\"name\":\"app\",\"version\":\"0.1.0\",\"dependencies\":{\"@acme/tools\":\"2.0.0\"}}\n",
    );
    write(
        dir.path(),
        "ds_modules/@acme/tools/deka.json",
        "{\"name\":\"@acme/tools\",\"version\":\"2.0.0\",\"entry\":\"index.ds\"}\n",
    );
    write(
        dir.path(),
        "ds_modules/@acme/tools/index.ds",
        "export fn value() number { return 42; }\n",
    );
    let app = write(
        dir.path(),
        "app.ds",
        "import { value } from \"@acme/tools\";\nfn main() number { return value(); }\n",
    );
    assert_eq!(result(&app).await.unwrap(), HostValue::Number(42.));
}

#[test]
fn check_sees_package_exports_with_types() {
    let dir = tempfile::tempdir().unwrap();
    project_with_mathpkg(dir.path());
    // A typed misuse of a package export fails at check time, proving the
    // checker resolves the package's exports like any module's.
    let app = write(
        dir.path(),
        "app.ds",
        "import { answer } from \"mathpkg\";\nfn main() string { return answer(); }\n",
    );
    let error = compiler::compile_file(&app, &Hosts::default(), Some("main")).unwrap_err();
    assert!(!error.is_empty(), "expected a type error");
}

#[test]
fn undeclared_package_is_a_named_error() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "deka.json",
        "{\"name\":\"app\",\"version\":\"0.1.0\"}\n",
    );
    let app = write(
        dir.path(),
        "app.ds",
        "import { answer } from \"mathpkg\";\nfn main() number { return answer(); }\n",
    );
    let error = compiler::compile_file(&app, &Hosts::default(), Some("main")).unwrap_err();
    assert!(
        error.contains("package mathpkg is not declared in deka.json dependencies"),
        "{error}"
    );
}

#[test]
fn uninstalled_package_is_a_named_error() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "deka.json",
        "{\"name\":\"app\",\"version\":\"0.1.0\",\"dependencies\":{\"mathpkg\":\"1.2.3\"}}\n",
    );
    let app = write(
        dir.path(),
        "app.ds",
        "import { answer } from \"mathpkg\";\nfn main() number { return answer(); }\n",
    );
    let error = compiler::compile_file(&app, &Hosts::default(), Some("main")).unwrap_err();
    assert!(
        error.contains("package mathpkg is not installed (ds_modules/mathpkg is missing)"),
        "{error}"
    );
}

#[test]
fn package_without_an_entry_file_is_a_named_error() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "deka.json",
        "{\"name\":\"app\",\"version\":\"0.1.0\",\"dependencies\":{\"mathpkg\":\"1.2.3\"}}\n",
    );
    // No deka.json in the package and no index.ds either.
    write(
        dir.path(),
        "ds_modules/mathpkg/util.ds",
        "export fn x() number { return 1; }\n",
    );
    let app = write(
        dir.path(),
        "app.ds",
        "import { x } from \"mathpkg\";\nfn main() number { return x(); }\n",
    );
    let error = compiler::compile_file(&app, &Hosts::default(), Some("main")).unwrap_err();
    assert!(
        error.contains("package mathpkg has no entry file"),
        "{error}"
    );
}

#[test]
fn installed_version_must_match_the_declaration_and_lock() {
    let dir = tempfile::tempdir().unwrap();
    project_with_mathpkg(dir.path());
    // Downgrade the installed package without touching the project files.
    write(
        dir.path(),
        "ds_modules/mathpkg/deka.json",
        "{\"name\":\"mathpkg\",\"version\":\"1.0.0\",\"entry\":\"main.ds\"}\n",
    );
    let app = write(
        dir.path(),
        "app.ds",
        "import { answer } from \"mathpkg\";\nfn main() number { return answer(); }\n",
    );
    let error = compiler::compile_file(&app, &Hosts::default(), Some("main")).unwrap_err();
    assert!(
        error.contains(
            "package mathpkg version mismatch: deka.json expects 1.2.3, ds_modules has 1.0.0"
        ),
        "{error}"
    );

    // Versions agree with deka.json but disagree with the lock pin.
    let dir = tempfile::tempdir().unwrap();
    project_with_mathpkg(dir.path());
    write(
        dir.path(),
        "deka.lock",
        "{\"lockfileVersion\":1,\"packages\":{\"mathpkg\":[\"9.9.9\",\"https://example.invalid/t.tgz\",{},\"deadbeef\"]}}\n",
    );
    let app = write(
        dir.path(),
        "app.ds",
        "import { answer } from \"mathpkg\";\nfn main() number { return answer(); }\n",
    );
    let error = compiler::compile_file(&app, &Hosts::default(), Some("main")).unwrap_err();
    assert!(
        error.contains(
            "package mathpkg version mismatch: deka.lock pins 9.9.9, ds_modules has 1.2.3"
        ),
        "{error}"
    );
}

#[tokio::test]
async fn a_package_imports_another_package_from_the_same_root() {
    let dir = tempfile::tempdir().unwrap();
    project_with_mathpkg(dir.path());
    write(
        dir.path(),
        "deka.json",
        "{\"name\":\"app\",\"version\":\"0.1.0\",\"dependencies\":{\"mathpkg\":\"1.2.3\",\"doublepkg\":\"1.0.0\"}}\n",
    );
    write(
        dir.path(),
        "ds_modules/doublepkg/deka.json",
        "{\"name\":\"doublepkg\",\"version\":\"1.0.0\",\"entry\":\"index.ds\"}\n",
    );
    write(
        dir.path(),
        "ds_modules/doublepkg/index.ds",
        "import { answer } from \"mathpkg\";\nexport fn double() number { return answer() * 2; }\n",
    );
    let app = write(
        dir.path(),
        "app.ds",
        "import { double } from \"doublepkg\";\nfn main() number { return double(); }\n",
    );
    assert_eq!(result(&app).await.unwrap(), HostValue::Number(84.));
}

/// `deka.json` is needed only when a bare specifier is resolved: a program
/// that imports no package compiles and runs without one, and a package
/// import without one is a named error. (Meaningful only when no ancestor of
/// the temporary directory has a `deka.json`, which the first assertion pins.)
#[tokio::test]
async fn deka_json_is_needed_only_to_resolve_a_package() {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    assert!(
        !root.ancestors().any(|a| a.join("deka.json").exists()),
        "the temporary directory is inside a deka project: {}",
        root.display()
    );
    write(dir.path(), "lib.ds", "export const base = 40;\n");
    let app = write(
        dir.path(),
        "app.ds",
        "import { base } from \"./lib.ds\";\nfn main() number { return base + 2; }\n",
    );
    assert_eq!(result(&app).await.unwrap(), HostValue::Number(42.0));
    let app = write(
        dir.path(),
        "pkg.ds",
        "import { answer } from \"mathpkg\";\nfn main() number { return answer(); }\n",
    );
    let error = compiler::compile_file(&app, &Hosts::default(), Some("main")).unwrap_err();
    assert!(
        error.contains(
            "package mathpkg cannot be resolved: deka.json not found in this directory or any parent"
        ),
        "{error}"
    );
}
