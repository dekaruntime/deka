#![cfg(all(feature = "compiler", feature = "host"))]
//! Import cycles load with JavaScript module semantics (deka#1206):
//! run-time calls across a cycle work once both files have loaded, while a
//! load-time read of a not-yet-initialized export is a named error.
use deka_vm::*;
use std::path::Path;

fn write(dir: &Path, name: &str, content: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, content).unwrap();
    path
}

fn compile(path: &Path) -> Program {
    compiler::compile_file(path, &Hosts::default(), Some("main")).unwrap()
}

async fn result(path: &Path) -> Result<HostValue> {
    let program = compiler::compile_file(path, &Hosts::default(), Some("main"))?;
    Vm::new(program, Hosts::default())?.run().await
}

#[tokio::test]
async fn two_file_cycle_calls_both_directions_at_run_time() {
    let dir = tempfile::tempdir().unwrap();
    let a = write(
        dir.path(),
        "a.ds",
        "import { from_b } from \"./b.ds\";\n\
         export fn from_a() number { return 1; }\n\
         fn main() number { return from_a() + from_b(); }\n",
    );
    write(
        dir.path(),
        "b.ds",
        "import { from_a } from \"./a.ds\";\n\
         export fn from_b() number { return from_a() + 1; }\n",
    );
    assert_eq!(result(&a).await.unwrap(), HostValue::Number(3.));
}

#[tokio::test]
async fn three_file_cycle_resolves_run_time_calls() {
    let dir = tempfile::tempdir().unwrap();
    let a = write(
        dir.path(),
        "a.ds",
        "import { from_c } from \"./c.ds\";\n\
         export fn from_a() number { return 1; }\n\
         fn main() number { return from_c(); }\n",
    );
    write(
        dir.path(),
        "b.ds",
        "import { from_a } from \"./a.ds\";\n\
         export fn from_b() number { return from_a() + 1; }\n",
    );
    write(
        dir.path(),
        "c.ds",
        "import { from_b } from \"./b.ds\";\n\
         export fn from_c() number { return from_b() + 1; }\n",
    );
    assert_eq!(result(&a).await.unwrap(), HostValue::Number(3.));
}

#[tokio::test]
async fn load_time_read_across_a_cycle_names_the_export_and_both_files() {
    let dir = tempfile::tempdir().unwrap();
    let a = write(
        dir.path(),
        "a.ds",
        "import { y } from \"./b.ds\";\n\
         export const x = 41;\n\
         fn main() number { return y; }\n",
    );
    write(
        dir.path(),
        "b.ds",
        "import { x } from \"./a.ds\";\n\
         export const y = x + 1;\n",
    );
    let error = result(&a).await.unwrap_err();
    assert!(error.contains("export `x`"), "{error}");
    assert!(error.contains("a.ds"), "{error}");
    assert!(error.contains("b.ds"), "{error}");
    assert!(error.contains("import cycle"), "{error}");
}

#[tokio::test]
async fn closure_called_during_load_reads_across_a_cycle_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let a = write(
        dir.path(),
        "a.ds",
        "import { y } from \"./b.ds\";\n\
         export const x = 41;\n\
         fn main() number { return y; }\n",
    );
    write(
        dir.path(),
        "b.ds",
        "import { x } from \"./a.ds\";\n\
         export fn read_x() number { return x; }\n\
         export const y = read_x();\n",
    );
    let error = result(&a).await.unwrap_err();
    assert!(error.contains("export `x`"), "{error}");
    assert!(error.contains("import cycle"), "{error}");
}

#[test]
fn self_import_is_still_refused() {
    let dir = tempfile::tempdir().unwrap();
    let a = write(
        dir.path(),
        "a.ds",
        "import { x } from \"./a.ds\";\n\
         export const x = 1;\n\
         fn main() number { return x; }\n",
    );
    let error = compiler::compile_file(&a, &Hosts::default(), Some("main")).unwrap_err();
    assert!(error.contains("cyclic module import"), "{error}");
}

#[tokio::test]
async fn acyclic_modules_still_compile_without_cycle_machinery() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "base.ds", "export const base = 40;\n");
    let a = write(
        dir.path(),
        "a.ds",
        "import { base } from \"./base.ds\";\n\
         export fn answer() number { return base + 2; }\n\
         fn main() number { return answer(); }\n",
    );
    let program = compile(&a);
    let result = Vm::new(program, Hosts::default()).unwrap().run().await;
    assert_eq!(result.unwrap(), HostValue::Number(42.));
}
