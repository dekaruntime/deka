#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;
use std::path::Path;

fn write(dir: &Path, name: &str, source: &str) {
    std::fs::write(dir.join(name), source).unwrap();
}
async fn run_after_deleting_sources(dir: &Path) -> Result<HostValue> {
    let program = compiler::compile_file(&dir.join("main.ds"), &Hosts::default(), Some("main"))?;
    let bytes = serde_json::to_vec(&program).unwrap();
    for name in ["library.ds", "inner.ds", "outer.ds", "main.ds"] {
        std::fs::remove_file(dir.join(name)).unwrap();
    }
    Vm::new(serde_json::from_slice(&bytes).unwrap(), Hosts::default())?
        .run()
        .await
}

#[tokio::test]
async fn erased_types_cross_renamed_default_and_multihop_barrels_with_origin_factories() {
    for (kind, source, body, expected) in [
        (
            "alias",
            "alias Thing = number; export {Thing}; export fn make() Thing {return 42;}",
            "const value: Public = make(); return value;",
            HostValue::Number(42.),
        ),
        (
            "interface",
            "interface Thing {fn name() string;} struct Item {id:number;} fn(this Item) name() string {return \"Ada\";} export {Thing}; export fn make() Thing {return Item({id:1});}",
            "const value: Public = make(); return value.name();",
            HostValue::String("Ada".into()),
        ),
        (
            "enum",
            "enum Thing {One, Two} export {Thing}; export fn make() Thing {return Thing.Two;}",
            "const value: Public = make(); return value.index;",
            HostValue::Number(1.),
        ),
        (
            "newtype",
            "type Thing number; export {Thing}; export fn make() Thing {return Thing(42);}",
            "const value: Public = make(); return unboxNumber(value);",
            HostValue::Number(42.),
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "library.ds", source);
        write(
            dir.path(),
            "inner.ds",
            "export {Thing as Renamed, make as factory} from \"./library.ds\";",
        );
        write(
            dir.path(),
            "outer.ds",
            "export {Renamed as default, factory} from \"./inner.ds\";",
        );
        // All imported type aliases use the consumer's spelling, including
        // nominal enum/newtype factories forwarded through default barrels.
        let local_type = "Public";
        let return_type = if kind == "interface" {
            "string"
        } else {
            "number"
        };
        write(
            dir.path(),
            "main.ds",
            &format!(
                "import type {{default as {local_type}}} from \"./outer.ds\"; import {{factory as make}} from \"./outer.ds\"; fn main() {return_type} {{{body}}}"
            ),
        );
        assert_eq!(
            run_after_deleting_sources(dir.path())
                .await
                .unwrap_or_else(|e| panic!("{kind}: {e}")),
            expected,
            "{kind}"
        );
    }
}

#[tokio::test]
async fn an_opaque_type_can_be_forwarded_without_inventing_a_runtime_value() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "library.ds",
        "opaque type Handle; export {Handle};",
    );
    write(
        dir.path(),
        "inner.ds",
        "export {Handle as Resource} from \"./library.ds\";",
    );
    write(
        dir.path(),
        "outer.ds",
        "export {Resource as default} from \"./inner.ds\";",
    );
    write(
        dir.path(),
        "main.ds",
        "import type {default as Handle} from \"./outer.ds\"; fn use_handle(value: Handle) number {return 42;} fn main() number {return 7;}",
    );
    assert_eq!(
        run_after_deleting_sources(dir.path()).await.unwrap(),
        HostValue::Number(7.)
    );
}

#[test]
fn an_erased_forwarded_type_is_not_a_value_and_missing_names_still_fail() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "library.ds",
        "alias Count = number; export {Count};",
    );
    write(
        dir.path(),
        "inner.ds",
        "export {Count} from \"./library.ds\";",
    );
    write(
        dir.path(),
        "main.ds",
        "import type {Count} from \"./inner.ds\"; fn main() number {return Count;}",
    );
    let error =
        compiler::compile_file(&dir.path().join("main.ds"), &Hosts::default(), Some("main"))
            .unwrap_err();
    assert!(
        error.contains("import type") && error.contains("can only be used as a type"),
        "{error}"
    );
    write(
        dir.path(),
        "main.ds",
        "export {Missing} from \"./inner.ds\"; fn main() number {return 1;}",
    );
    let error =
        compiler::compile_file(&dir.path().join("main.ds"), &Hosts::default(), Some("main"))
            .unwrap_err();
    assert!(error.contains("Missing"), "{error}");
}

#[tokio::test]
async fn same_spelled_nominal_factories_in_one_barrel_remain_distinct() {
    for kind in ["enum", "newtype"] {
        let dir = tempfile::tempdir().unwrap();
        let source = if kind == "enum" {
            "enum Thing {One, Two} export {Thing}; export fn make() Thing {return Thing.Two;}"
        } else {
            "type Thing number; export {Thing}; export fn make() Thing {return Thing(42);}"
        };
        write(dir.path(), "left.ds", source);
        write(dir.path(), "right.ds", source);
        write(
            dir.path(),
            "inner.ds",
            "export {Thing as Left, make as left} from \"./left.ds\"; export {Thing as Right, make as right} from \"./right.ds\";",
        );
        write(
            dir.path(),
            "outer.ds",
            "export {Left as First, Right as Second, left, right} from \"./inner.ds\";",
        );
        let read = if kind == "enum" {
            "a.index + b.index"
        } else {
            "unboxNumber(a) + unboxNumber(b)"
        };
        write(
            dir.path(),
            "main.ds",
            &format!(
                "import type {{First as A, Second as B}} from \"./outer.ds\"; import {{left, right}} from \"./outer.ds\"; fn main() number {{const a:A=left(); const b:B=right(); return {read};}}"
            ),
        );
        let program =
            compiler::compile_file(&dir.path().join("main.ds"), &Hosts::default(), Some("main"))
                .unwrap_or_else(|e| panic!("{kind}: {e}"));
        assert_eq!(
            Vm::new(program, Hosts::default())
                .unwrap()
                .run()
                .await
                .unwrap(),
            HostValue::Number(if kind == "enum" { 2. } else { 84. })
        );
        write(
            dir.path(),
            "main.ds",
            "import type {First as A, Second as B} from \"./outer.ds\"; import {left, right} from \"./outer.ds\"; fn main() number {const a:A=right(); return 0;}",
        );
        let error =
            compiler::compile_file(&dir.path().join("main.ds"), &Hosts::default(), Some("main"))
                .unwrap_err();
        assert!(
            error.contains("expected type `A`, found type `B`"),
            "{kind}: {error}"
        );
    }
}

#[test]
fn an_unexported_same_spelled_type_is_not_renamed_to_another_origin() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "left.ds",
        "type Thing number; export {Thing}; export fn left() Thing {return Thing(1);}",
    );
    write(
        dir.path(),
        "right.ds",
        "type Thing number; export fn right() Thing {return Thing(2);}",
    );
    write(
        dir.path(),
        "inner.ds",
        "export {Thing as Public, left} from \"./left.ds\"; export {right} from \"./right.ds\";",
    );
    write(
        dir.path(),
        "main.ds",
        "import type {Public as Local} from \"./inner.ds\"; import {right} from \"./inner.ds\"; const value: Local = right();",
    );
    let error =
        compiler::compile_file(&dir.path().join("main.ds"), &Hosts::default(), None).unwrap_err();
    assert!(
        error.contains("expected type `Local`, found type `Thing`"),
        "{error}"
    );
}
