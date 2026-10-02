#![cfg(all(feature = "compiler", feature = "host"))]
//! Module forms (deka#1208): default imports, type-only imports, and alias
//! erasure in the native VM.
use deka_vm::*;
use std::path::Path;

fn write(dir: &Path, name: &str, content: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, content).unwrap();
    path
}

async fn result(path: &Path) -> Result<HostValue> {
    let program = compiler::compile_file(path, &Hosts::default(), Some("main"))?;
    Vm::new(program, Hosts::default())?.run().await
}

#[tokio::test]
async fn default_import_runs_the_default_export() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "greeter.ds",
        "export default fn greet() number { return 42; }\n",
    );
    let a = write(
        dir.path(),
        "a.ds",
        "import greet from \"./greeter.ds\";\nfn main() number { return greet(); }\n",
    );
    assert_eq!(result(&a).await.unwrap(), HostValue::Number(42.));
}

#[tokio::test]
async fn mixed_default_and_named_imports_work() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "math.ds",
        "export default fn base() number { return 40; }\nexport fn two() number { return 2; }\n",
    );
    let a = write(
        dir.path(),
        "a.ds",
        "import base, { two } from \"./math.ds\";\nfn main() number { return base() + two(); }\n",
    );
    assert_eq!(result(&a).await.unwrap(), HostValue::Number(42.));
}

#[test]
fn default_import_without_a_default_export_is_a_named_error() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "plain.ds", "export fn other() number { return 1; }\n");
    let a = write(
        dir.path(),
        "a.ds",
        "import missing from \"./plain.ds\";\nfn main() number { return missing(); }\n",
    );
    let error = compiler::compile_file(&a, &Hosts::default(), Some("main")).unwrap_err();
    assert!(error.contains("cannot resolve imported name `default`"), "{error}");
}

#[tokio::test]
async fn alias_declarations_erase_and_stay_usable_in_type_positions() {
    let dir = tempfile::tempdir().unwrap();
    let a = write(
        dir.path(),
        "a.ds",
        "alias Count = number;\nfn main() Count { const c: Count = 42; return c; }\n",
    );
    assert_eq!(result(&a).await.unwrap(), HostValue::Number(42.));
}

#[tokio::test]
async fn type_only_imports_resolve_for_the_checker_and_erase() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "types.ds",
        "alias Count = number;\nexport { Count };\nexport fn answer(c: Count) Count { return c; }\n",
    );
    let a = write(
        dir.path(),
        "a.ds",
        "import type { Count } from \"./types.ds\";\nimport { answer } from \"./types.ds\";\nfn main() Count { return answer(42); }\n",
    );
    assert_eq!(result(&a).await.unwrap(), HostValue::Number(42.));
}

#[tokio::test]
async fn inline_type_specifiers_erase_alongside_value_imports() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "types.ds",
        "alias Count = number;\nexport { Count };\nexport fn answer(c: Count) Count { return c; }\n",
    );
    let a = write(
        dir.path(),
        "a.ds",
        "import { type Count, answer } from \"./types.ds\";\nfn main() Count { return answer(42); }\n",
    );
    assert_eq!(result(&a).await.unwrap(), HostValue::Number(42.));
}

#[test]
fn using_a_type_only_import_as_a_value_is_a_typeck_error() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "types.ds",
        "alias Count = number;\nexport { Count };\n",
    );
    let a = write(
        dir.path(),
        "a.ds",
        "import type { Count } from \"./types.ds\";\nfn main() number { return Count; }\n",
    );
    let error = compiler::compile_file(&a, &Hosts::default(), Some("main")).unwrap_err();
    assert!(!error.is_empty(), "expected a compile error");
}
