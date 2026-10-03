#![cfg(all(feature = "compiler", feature = "host"))]
//! Barrel re-exports (deka#1210): `export { x } from "./y.ds"` aliases the
//! origin's slot, and `export { a, b as c }` re-exports local values.
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
async fn barrel_reexport_is_importable() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "y.ds",
        "export fn answer() number { return 42; }\n",
    );
    write(
        dir.path(),
        "barrel.ds",
        "export { answer } from \"./y.ds\";\n",
    );
    let a = write(
        dir.path(),
        "a.ds",
        "import { answer } from \"./barrel.ds\";\nfn main() number { return answer(); }\n",
    );
    assert_eq!(result(&a).await.unwrap(), HostValue::Number(42.));
}

#[tokio::test]
async fn barrel_of_a_barrel_resolves_to_the_origin() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "y.ds",
        "export fn answer() number { return 42; }\n",
    );
    write(
        dir.path(),
        "inner.ds",
        "export { answer } from \"./y.ds\";\n",
    );
    write(
        dir.path(),
        "outer.ds",
        "export { answer } from \"./inner.ds\";\n",
    );
    let a = write(
        dir.path(),
        "a.ds",
        "import { answer } from \"./outer.ds\";\nfn main() number { return answer(); }\n",
    );
    assert_eq!(result(&a).await.unwrap(), HostValue::Number(42.));
}

#[tokio::test]
async fn renamed_and_default_barrel_reexports() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "y.ds",
        "export fn answer() number { return 42; }\n",
    );
    write(
        dir.path(),
        "barrel.ds",
        "export { answer as reply, answer as default } from \"./y.ds\";\n",
    );
    let a = write(
        dir.path(),
        "a.ds",
        "import reply, { reply as again } from \"./barrel.ds\";\nfn main() number { return reply() + again() - 42.0; }\n",
    );
    assert_eq!(result(&a).await.unwrap(), HostValue::Number(42.));
}

#[tokio::test]
async fn local_named_reexports_and_default_alias() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "lib.ds",
        "fn compute() number { return 40; }\nconst extra = 2;\nexport { compute, extra };\nexport { compute as default };\n",
    );
    let a = write(
        dir.path(),
        "a.ds",
        "import compute, { extra } from \"./lib.ds\";\nfn main() number { return compute() + extra; }\n",
    );
    assert_eq!(result(&a).await.unwrap(), HostValue::Number(42.));
}

#[tokio::test]
async fn cycle_through_a_barrel_edge_loads() {
    let dir = tempfile::tempdir().unwrap();
    // a re-exports from the barrel, the barrel re-exports from b, b imports a.
    let a = write(
        dir.path(),
        "a.ds",
        "import { from_b } from \"./barrel.ds\";\n\
         export fn from_a() number { return 1; }\n\
         fn main() number { return from_a() + from_b(); }\n",
    );
    write(
        dir.path(),
        "barrel.ds",
        "export { from_b } from \"./b.ds\";\n",
    );
    write(
        dir.path(),
        "b.ds",
        "import { from_a } from \"./a.ds\";\nexport fn from_b() number { return from_a() + 1; }\n",
    );
    assert_eq!(result(&a).await.unwrap(), HostValue::Number(3.));
}

#[tokio::test]
async fn load_time_read_through_a_barrel_names_the_origin() {
    let dir = tempfile::tempdir().unwrap();
    // b initializes first and reads a's x through the barrel at load time:
    // a's slot is not set yet, and the error names the origin.
    let a = write(
        dir.path(),
        "a.ds",
        "import { y } from \"./barrel.ds\";\n\
         export const x = 41;\n\
         fn main() number { return y; }\n",
    );
    write(
        dir.path(),
        "barrel.ds",
        "export { x } from \"./a.ds\";\nexport { y } from \"./b.ds\";\n",
    );
    write(
        dir.path(),
        "b.ds",
        "import { x } from \"./barrel.ds\";\nexport const y = x + 1;\n",
    );
    let error = result(&a).await.unwrap_err();
    assert!(error.contains("export `x`"), "{error}");
    assert!(error.contains("a.ds"), "{error}");
    assert!(error.contains("import cycle"), "{error}");
}

#[test]
fn barrel_self_import_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let a = write(
        dir.path(),
        "a.ds",
        "export fn x() number { return 1; }\nexport { x } from \"./a.ds\";\nfn main() number { return x(); }\n",
    );
    let error = compiler::compile_file(&a, &Hosts::default(), Some("main")).unwrap_err();
    assert!(error.contains("cyclic module import"), "{error}");
}

#[test]
fn barrel_of_a_missing_export_is_a_named_error() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "y.ds",
        "export fn other() number { return 1; }\n",
    );
    let a = write(
        dir.path(),
        "a.ds",
        "export { missing } from \"./y.ds\";\nfn main() number { return 1; }\n",
    );
    let error = compiler::compile_file(&a, &Hosts::default(), Some("main")).unwrap_err();
    assert!(!error.is_empty(), "expected a compile error");
}
