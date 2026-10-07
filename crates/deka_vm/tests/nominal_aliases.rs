#![cfg(feature = "compiler")]
use deka_vm::{HostValue, Hosts, Vm, compiler};
use std::fs;

#[tokio::test]
async fn multiple_local_names_keep_one_declaration_identity() {
    for (declaration, factory, body, expected) in [
        (
            "struct Thing {x:number}",
            "Thing{x:42}",
            "const value:Second=copy(make()); const other:First=Construct{x:1}; return match(value){Construct{x}=>x+other.x};",
            43.,
        ),
        (
            "enum Thing {One,Two}",
            "Thing.Two",
            "const value:Second=copy(make()); return match(value){Construct.One=>0,Construct.Two=>7};",
            7.,
        ),
        (
            "type Thing number;",
            "Thing(42)",
            "const value:Second=copy(make()); const other:First=make(); return unboxNumber(value)+unboxNumber(other);",
            84.,
        ),
    ] {
        for barrel in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            fs::write(
                dir.path().join("library.ds"),
                format!(
                    "{declaration} export {{Thing}}; export fn make() Thing {{return {factory};}}"
                ),
            )
            .unwrap();
            if barrel {
                fs::write(
                    dir.path().join("index.ds"),
                    "export {Thing as default,make} from \"./library.ds\";",
                )
                .unwrap();
            }
            let source = if barrel { "./index.ds" } else { "./library.ds" };
            let imported = if barrel { "default" } else { "Thing" };
            let values = if declaration.starts_with("type ") {
                "make"
            } else {
                &format!("{imported} as Construct,make")
            };
            fs::write(dir.path().join("main.ds"),format!("import type {{{imported} as First}} from \"{source}\"; import type {{{imported} as Second}} from \"{source}\"; import {{{values}}} from \"{source}\"; fn copy(value:First) Second {{return value;}} fn main() number {{{body}}}" )).unwrap();
            let program = compiler::compile_file(
                &dir.path().join("main.ds"),
                &Hosts::default(),
                Some("main"),
            )
            .unwrap_or_else(|error| panic!("{declaration}: {error}"));
            fs::remove_dir_all(dir.path()).unwrap();
            assert_eq!(
                Vm::new(program, Hosts::default())
                    .unwrap()
                    .run()
                    .await
                    .unwrap(),
                HostValue::Number(expected)
            );
        }
    }
}

#[test]
fn same_named_empty_declarations_remain_distinct() {
    for declaration in ["struct Thing {}", "enum Thing {}", "type Thing number;"] {
        let dir = tempfile::tempdir().unwrap();
        for name in ["left.ds", "right.ds"] {
            fs::write(
                dir.path().join(name),
                format!("{declaration} export {{Thing}};"),
            )
            .unwrap();
        }
        fs::write(dir.path().join("main.ds"),"import type {Thing as First,Thing as Alias} from \"./left.ds\"; import type {Thing as Other} from \"./right.ds\"; fn wrong(value:Alias) Other {return value;}" ).unwrap();
        let error = compiler::compile_file(&dir.path().join("main.ds"), &Hosts::default(), None)
            .unwrap_err();
        assert!(
            error.contains("expected return type `Other`"),
            "{declaration}: {error}"
        );
    }
}

#[test]
fn an_ordinary_alias_does_not_turn_a_type_only_alias_into_a_value() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("library.ds"),
        "enum Thing {One,Two} export {Thing};",
    )
    .unwrap();
    fs::write(dir.path().join("main.ds"),"import {Thing as Construct} from \"./library.ds\"; import type {Thing as Only} from \"./library.ds\"; const value=Only.Two;").unwrap();
    let error =
        compiler::compile_file(&dir.path().join("main.ds"), &Hosts::default(), None).unwrap_err();
    assert!(
        error.contains("`Only` was imported with `import type`")
            && error.contains("can only be used as a type"),
        "{error}"
    );
}
