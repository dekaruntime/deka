#![cfg(feature = "compiler")]
use deka_vm::*;
use std::collections::BTreeMap;
fn schema() -> HostEnum {
    HostEnum {
        name: "FsError".into(),
        cases: vec![
            (
                "PermissionDenied".into(),
                Some(HostType::Struct(Box::new(HostStruct {
                    name: "FsPermission".into(),
                    fields: BTreeMap::from([
                        ("capability".into(), HostType::String),
                        ("target".into(), HostType::String),
                    ]),
                }))),
            ),
            ("UnsupportedHost".into(), None),
            ("InvalidPayload".into(), None),
            ("Failed".into(), Some(HostType::String)),
        ],
    }
}
fn hosts() -> Hosts {
    let mut hosts = Hosts::default();
    hosts
        .register(
            HostOp::new("fs_read", vec![], HostType::String, false, |_| {
                HostReply::Ready(Err("missing".into()))
            })
            .with_enum_result_channel(schema(), "Failed"),
        )
        .unwrap();
    let ty = HostType::Enum(Box::new(schema()));
    hosts
        .register(HostOp::new(
            "fs_roundtrip",
            vec![ty.clone()],
            ty,
            false,
            |mut args| HostReply::Ready(Ok(args.remove(0))),
        ))
        .unwrap();
    hosts
}
async fn run(source: &str) -> Result<HostValue> {
    let hosts = hosts();
    let program = compiler::compile(source, &hosts)?;
    let program = serde_json::from_str(&serde_json::to_string(&program).unwrap()).unwrap();
    Vm::new(program, hosts)?.run().await
}
#[tokio::test]
async fn typed_operational_errors_use_declared_enum_case_and_match_payload() {
    assert_eq!(run(r#"import { read } from "fs"
        fn main() string { return match read() { Ok(v) => v, Err(e) => match e { Failed(s) => s, PermissionDenied(p) => p.target, UnsupportedHost => "host", InvalidPayload => "payload" } }; }"#).await.unwrap(),HostValue::String("missing".into()));
}
#[tokio::test]
async fn enum_alias_and_payload_free_constructor_roundtrip() {
    assert_eq!(run(r#"import { FsError as Fault, roundtrip } from "fs"
        fn main() string { return match roundtrip(Fault.Failed("ready")) { Failed(s) => s, PermissionDenied(p) => p.target, UnsupportedHost => "host", InvalidPayload => "payload" }; }"#).await.unwrap(),HostValue::String("ready".into()));
    assert_eq!(run(r#"import { FsError, roundtrip } from "fs"
        fn main() string { return match roundtrip(FsError.UnsupportedHost) { Failed(s) => s, PermissionDenied(p) => p.target, UnsupportedHost => "host", InvalidPayload => "payload" }; }"#).await.unwrap(),HostValue::String("host".into()));
}
#[tokio::test]
async fn unrelated_same_spelled_authored_enum_remains_valid_but_cannot_cross_native_boundary() {
    assert_eq!(
        run(r#"enum FsError { UserFailure(string) }
        fn main() string { return match FsError.UserFailure("own") { UserFailure(s) => s }; }"#)
        .await
        .unwrap(),
        HostValue::String("own".into())
    );
    let err = run(r#"import { roundtrip } from "fs"
        enum FsError { UserFailure(string) }
        fn main() void { roundtrip(FsError.UserFailure("own")); }"#)
    .await
    .unwrap_err();
    assert!(
        err.contains("argument") || err.contains("expected"),
        "{err}"
    );
}
#[tokio::test]
async fn public_descriptor_and_printing_hide_native_brand() {
    assert_eq!(run(r#"import { FsError } from "fs"
        fn main() string { const value=FsError.Failed("missing"); return value.getType().toString() + ":" + string(value); }"#).await.unwrap(),HostValue::String("FsError:Failed(\"missing\")".into()));
}
#[test]
fn catalog_rejects_conflicting_nominal_schema_and_invalid_error_case() {
    let mut hosts = hosts();
    let mut conflict = schema();
    conflict.cases.push(("Other".into(), None));
    assert!(
        hosts
            .register(HostOp::new(
                "bad",
                vec![],
                HostType::Enum(Box::new(conflict)),
                false,
                |_| unreachable!()
            ))
            .unwrap_err()
            .contains("conflicting")
    );
    assert!(
        hosts
            .register(
                HostOp::new(
                    "bad_error",
                    vec![],
                    HostType::String,
                    false,
                    |_| unreachable!()
                )
                .with_enum_result_channel(schema(), "UnsupportedHost")
            )
            .unwrap_err()
            .contains("string-payload")
    );
}
#[tokio::test]
async fn malformed_rust_enum_result_is_protocol_fault() {
    for (case, payload) in [
        ("Missing", None),
        ("Failed", None),
        ("Failed", Some(Box::new(HostValue::Number(7.)))),
    ] {
        let mut hosts = hosts();
        hosts
            .register(HostOp::new(
                "bad",
                vec![],
                HostType::Enum(Box::new(schema())),
                false,
                move |_| {
                    HostReply::Ready(Ok(HostValue::Enum {
                        name: schema().brand(),
                        case: case.into(),
                        payload: payload.clone(),
                    }))
                },
            ))
            .unwrap();
        let program = compiler::compile(
            r#"import { bad } from "vm:host"; fn main() void {bad();}"#,
            &hosts,
        )
        .unwrap();
        assert!(
            Vm::new(program, hosts)
                .unwrap()
                .run()
                .await
                .unwrap_err()
                .contains("wrong result type")
        );
    }
}
#[tokio::test]
async fn named_struct_payload_preserves_its_fields_and_brand() {
    assert_eq!(run(r#"import { FsError as Fault, FsPermission as Permission, roundtrip } from "fs"
        fn main() string { return match roundtrip(Fault.PermissionDenied(Permission { capability: "read", target: "demo" })) { Failed(s) => s, PermissionDenied(p) => p.target, UnsupportedHost => "host", InvalidPayload => "payload" }; }"#).await.unwrap(),HostValue::String("demo".into()));
}
#[tokio::test]
async fn suspended_enum_error_and_nested_enum_values_use_same_constructor_path() {
    let mut hosts = hosts();
    hosts
        .register(
            HostOp::new("fs_pending", vec![], HostType::String, true, |_| {
                HostReply::Pending(Box::pin(async { Err("offline".into()) }))
            })
            .with_enum_result_channel(schema(), "Failed"),
        )
        .unwrap();
    let program=compiler::compile(r#"import { pending } from "fs"; async fn main() Promise<string> { return match await pending() { Ok(v) => v, Err(e) => match e { Failed(s)=>s, PermissionDenied(p)=>p.target, UnsupportedHost=>"host", InvalidPayload=>"payload" } }; }"#,&hosts).unwrap();
    assert_eq!(
        Vm::new(program, hosts).unwrap().run().await.unwrap(),
        HostValue::String("offline".into())
    );
}
#[tokio::test]
async fn imported_enum_type_through_a_barrel_keeps_canonical_identity() {
    let directory = tempfile::tempdir().unwrap();
    let barrel = directory.path().join("barrel.ds");
    let consumer = directory.path().join("main.ds");
    std::fs::write(
        barrel,
        "export { FsError as Fault, roundtrip } from \"fs\"\n",
    )
    .unwrap();
    std::fs::write(&consumer,r#"import { Fault as Issue, roundtrip } from "./barrel.ds"
        fn inspect(value: Issue) string { return match value { Failed(s)=>s, PermissionDenied(p)=>p.target, UnsupportedHost=>"host", InvalidPayload=>"payload" }; }
        fn main() string { return inspect(roundtrip(Issue.Failed("barrel"))); }"#).unwrap();
    let hosts = hosts();
    let program = compiler::compile_file(&consumer, &hosts, Some("main")).unwrap();
    assert_eq!(
        Vm::new(program, hosts).unwrap().run().await.unwrap(),
        HostValue::String("barrel".into())
    );
}
#[tokio::test]
async fn ordinary_imported_enum_alias_keeps_parameter_identity_issue_1347() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("model.ds"),
        r#"enum State { Missing, Found(number) }
        export { State };
        export fn read(value: State) number {return match value {Missing=>0,Found(n)=>n};}"#,
    )
    .unwrap();
    let consumer = directory.path().join("main.ds");
    std::fs::write(
        &consumer,
        r#"import type { State as Mode } from "./model.ds";
        import { read } from "./model.ds";
        fn consume(value: Mode) number { return read(value); }
        fn main() number { return 0; }"#,
    )
    .unwrap();
    let hosts = Hosts::default();
    let program = compiler::compile_file(&consumer, &hosts, Some("main")).unwrap();
    assert_eq!(
        Vm::new(program, hosts).unwrap().run().await.unwrap(),
        HostValue::Number(0.)
    );
    std::fs::write(
        &consumer,
        r#"import { State as Mode, read } from "./model.ds";
        fn consume(value: Mode) number { return read(value); }
        fn main() number { return consume(Mode.Found(9)); }"#,
    )
    .unwrap();
    let hosts = Hosts::default();
    let program = compiler::compile_file(&consumer, &hosts, Some("main")).unwrap();
    assert_eq!(
        Vm::new(program, hosts).unwrap().run().await.unwrap(),
        HostValue::Number(9.)
    );
}
#[tokio::test]
async fn ordinary_imported_struct_alias_keeps_parameter_identity() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("model.ds"),
        r#"struct Point { x:number }
        export {Point}; export fn read(value:Point) number {return value.x;}"#,
    )
    .unwrap();
    let consumer = directory.path().join("main.ds");
    std::fs::write(
        &consumer,
        r#"import { Point as Position, read } from "./model.ds";
        fn consume(value:Position) number {return read(value);}
        fn main() number { return consume(Position {x:12}); }"#,
    )
    .unwrap();
    let hosts = Hosts::default();
    let program = compiler::compile_file(&consumer, &hosts, Some("main")).unwrap();
    assert_eq!(
        Vm::new(program, hosts).unwrap().run().await.unwrap(),
        HostValue::Number(12.)
    );
}
#[tokio::test]
async fn nested_enum_option_roundtrip_and_signature_keep_nominal_identity() {
    let mut hosts = hosts();
    let ty = HostType::Option(Box::new(HostType::Enum(Box::new(schema()))));
    hosts
        .register(HostOp::new(
            "fs_nested",
            vec![ty.clone()],
            ty,
            false,
            |mut args| HostReply::Ready(Ok(args.remove(0))),
        ))
        .unwrap();
    let program=compiler::compile(r#"import { FsError, nested } from "fs";
        fn main() string { const value=nested(Some(FsError.Failed("nested"))); return match value {Some(e)=>e.signature().toString()+":"+string(e),None=>"empty"}; }"#,&hosts).unwrap();
    assert_eq!(
        Vm::new(program, hosts).unwrap().run().await.unwrap(),
        HostValue::String("FsError:Failed(\"nested\")".into())
    );
}
#[test]
fn plain_records_cannot_impersonate_named_enum_or_struct() {
    let ty = HostType::Enum(Box::new(schema()));
    let fake = HostValue::Record(BTreeMap::from([
        ("name".into(), HostValue::String("Failed".into())),
        ("value".into(), HostValue::String("fake".into())),
    ]));
    assert!(!ty.accepts(&fake));
    let Some(HostType::Struct(ty)) = &schema().cases[0].1 else {
        panic!()
    };
    let fake = HostValue::Record(BTreeMap::from([
        ("capability".into(), HostValue::String("read".into())),
        ("target".into(), HostValue::String("demo".into())),
    ]));
    assert!(!HostType::Struct(ty.clone()).accepts(&fake));
}

#[test]
fn imported_enum_constructors_do_not_bypass_type_only_imports() {
    for constructor in ["Fault.Failed(\"bad\")", "Fault.UnsupportedHost"] {
        let source = format!(
            "import type {{FsError as Fault}} from \"fs\"; fn main() void {{ {constructor}; }}"
        );
        let error = compiler::compile(&source, &hosts()).unwrap_err();
        assert!(error.contains("import type"), "{error}");
    }
}
#[tokio::test]
async fn typed_callbacks_exchange_named_enums_through_the_same_schema() {
    let mut hosts = hosts();
    let ty = HostType::Enum(Box::new(schema()));
    hosts
        .register(HostOp::new(
            "fs_invoke",
            vec![HostType::TypedCallback {
                args: vec![ty.clone()],
                result: Box::new(ty.clone()),
                result_channel: false,
            }],
            ty,
            true,
            |args| {
                let HostValue::Callback(callback) = &args[0] else {
                    panic!()
                };
                HostReply::Pending(
                    callback
                        .call_async(vec![HostValue::Enum {
                            name: schema().brand(),
                            case: "Failed".into(),
                            payload: Some(Box::new(HostValue::String("callback".into()))),
                        }])
                        .unwrap(),
                )
            },
        ))
        .unwrap();
    let program=compiler::compile(r#"import {FsError as Fault,invoke} from "fs";
        async fn echo(value:Fault) Promise<Fault> {return value;}
        async fn main() Promise<string> {return match await invoke(echo) {Failed(s)=>s,PermissionDenied(p)=>p.target,UnsupportedHost=>"host",InvalidPayload=>"payload"};}"#,&hosts).unwrap();
    assert_eq!(
        Vm::new(program, hosts).unwrap().run().await.unwrap(),
        HostValue::String("callback".into())
    );
}

#[tokio::test]
async fn builtin_nominal_exports_follow_operation_catalog_without_type_name_list() {
    let status = HostType::Enum(Box::new(HostEnum {
        name: "DataStatus".into(),
        cases: vec![("Ready".into(), Some(HostType::Number))],
    }));
    let mut hosts = Hosts::default();
    hosts
        .register(HostOp::new(
            "bytes_identity",
            vec![status.clone()],
            status,
            false,
            |mut args| HostReply::Ready(Ok(args.remove(0))),
        ))
        .unwrap();
    let program = compiler::compile(
        r#"import { DataStatus, identity } from "bytes"
        fn main() number { return match identity(DataStatus.Ready(7)) { Ready(n) => n }; }"#,
        &hosts,
    )
    .unwrap();
    assert_eq!(
        Vm::new(program, hosts.clone())
            .unwrap()
            .run()
            .await
            .unwrap(),
        HostValue::Number(7.)
    );
    assert!(
        compiler::compile(
            r#"import { DataStatus } from "time"
        fn main() void {}"#,
            &hosts
        )
        .is_err()
    );
}
