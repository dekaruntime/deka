#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;
use std::{cell::RefCell, rc::Rc};

async fn run(source: &str) -> HostValue {
    let hosts = Hosts::default();
    let program = compiler::compile(source, &hosts).unwrap();
    // Also exercise the source-free bytecode path used by packaged programs.
    let program = serde_json::from_str(&serde_json::to_string(&program).unwrap()).unwrap();
    Vm::new(program, hosts).unwrap().run().await.unwrap()
}

#[tokio::test]
async fn prelude_constructor_spellings_build_nominal_enum_values() {
    let value = run(r#"
        fn main() string {
            const a: Option<number> = Option.Some(7);
            const b: Option<number> = Some(8);
            const n: Option<number> = Option.None;
            const m: Option<number> = None;
            const ok: Result<number, string> = Result.Ok(1);
            const err: Result<number, string> = Result.Err("boom");
            const shortOk: Result<number, string> = Ok(2);
            const shortErr: Result<number, string> = Err("bad");
            return string(a) + "," + string(b) + "," + string(n) + "," + string(m)
                + "," + string(ok) + "," + string(err) + "," + string(shortOk) + "," + string(shortErr)
                + "," + a.getType().toString() + "," + m.getType().toString() + "," + ok.getType().toString();
        }
    "#).await;
    assert_eq!(
        value,
        HostValue::String(
            r#"Some(7),Some(8),None,None,Ok(1),Err("boom"),Ok(2),Err("bad"),Option,Option,Result"#
                .into()
        )
    );
}

#[tokio::test]
async fn declared_and_prelude_enums_share_nested_printing() {
    assert_eq!(
        run(r#"
        enum Box<T> { Empty, Full(T) }
        struct Point { x: number; }
        fn main() string {
            const nested: Option<Result<number, string>> = Some(Ok(7));
            return string(nested) + ";" + string(Box.Full(Point { x: 2 })) + ";" + string(Box.Empty)
                + ";" + string(Some("a\"b\nc")) + ";" + string(None);
        }
    "#)
        .await,
        HostValue::String(
            "Some(Ok(7));Full(Point { x: 2 });Empty;Some(\"a\\\"b\\nc\");None".into()
        )
    );
}

#[tokio::test]
async fn constructing_a_payload_evaluates_it_once_without_erasing_its_type() {
    assert_eq!(
        run(r#"
        fn main() string {
            let calls = 0;
            const next = fn() number { calls = calls + 1; return calls; };
            const value = Some(next());
            return string(value) + "," + string(calls);
        }
    "#)
        .await,
        HostValue::String("Some(1),1".into())
    );
}

#[tokio::test]
async fn console_and_echo_use_the_same_nominal_printer() {
    let captured = Rc::new(RefCell::new(vec![]));
    let sink = captured.clone();
    let mut hosts = Hosts::default();
    hosts
        .register(HostOp::new(
            "echo",
            vec![HostType::String],
            HostType::Unit,
            false,
            move |args| {
                let HostValue::String(text) = &args[0] else {
                    panic!("text output required")
                };
                sink.borrow_mut().push(text.clone());
                HostReply::Ready(Ok(HostValue::Unit))
            },
        ))
        .unwrap();
    let program = compiler::compile(
        r#"
        import { echo } from "io";
        enum Shape { Circle(number), Empty }
        struct Data { name: string; index: number; value: number; }
        fn main() void {
            console.log(Some(7), Option.None, Result.Err("boom"), Shape.Circle(5), Shape.Empty);
            echo(string(Some(7)));
            // Enum-shaped fields remain ordinary struct data.
            console.log(Data { name: "Some", index: 0, value: 7 });
        }
    "#,
        &hosts,
    )
    .unwrap();
    Vm::new(program, hosts).unwrap().run().await.unwrap();
    assert_eq!(
        *captured.borrow(),
        [
            r#"Some(7) None Err("boom") Circle(5) Empty"#,
            "Some(7)",
            r#"Data { name: "Some", index: 0, value: 7 }"#
        ]
    );
}

#[test]
fn invalid_cases_payloads_and_unprintable_payloads_are_rejected() {
    for source in [
        "const x = Option.Other;",
        "const x = Option.Some;",
        "const x = Option.None(1);",
        "const x: Result<number, string> = Result.Err(1);",
        "const x = string(Some(fn() number { return 1; }));",
        "const x = string(Some(Ok(fn() number { return 1; })));",
        "enum Recursive { Next(Recursive), End } const x = string(Recursive.End);",
        "const x = string({ name: \"Some\", index: 0, value: 7 });",
    ] {
        assert!(
            compiler::compile(&format!("{source}\nfn main() void {{}}"), &Hosts::default())
                .is_err(),
            "{source}"
        );
    }
}
