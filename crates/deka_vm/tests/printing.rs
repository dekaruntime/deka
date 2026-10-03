#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;
use std::{cell::RefCell, rc::Rc};

async fn output(source: &str) -> Vec<String> {
    let captured = Rc::new(RefCell::new(vec![]));
    let sink = captured.clone();
    let mut hosts = Hosts::default();
    hosts
        .register(HostOp::new(
            "echo",
            vec![HostType::String],
            HostType::Unit,
            false,
            None,
            move |args| {
                let HostValue::String(text) = &args[0] else {
                    panic!("output must be text")
                };
                sink.borrow_mut().push(text.clone());
                HostReply::Ready(Ok(HostValue::Unit))
            },
        ))
        .unwrap();
    let program = compiler::compile(source, &hosts).unwrap();
    Vm::new(program, hosts).unwrap().run().await.unwrap();
    captured.borrow().clone()
}

#[tokio::test]
async fn struct_printing_uses_the_nominal_name_and_source_field_order() {
    let lines = output(
        r#"
        import { echo } from "io";
        struct Point { x: number; y: number; }
        fn (p Point) total() number { return p.x + p.y; }
        fn main() void {
            const p = Point { y: 2, x: 1 };
            console.log(p);
            echo(string(p));
        }
    "#,
    )
    .await;
    assert_eq!(lines, ["Point { y: 2, x: 1 }", "Point { y: 2, x: 1 }"]);
}

#[tokio::test]
async fn nested_strings_lists_and_empty_structs_share_the_printer() {
    let lines = output(r#"
        struct Empty {}
        struct Message { text: string; }
        struct Inbox { message: Message; counts: Array<number>; empty: Empty; }
        fn main() void {
            console.log(Inbox { message: Message { text: "a\"b\nc" }, counts: [1, 2], empty: Empty {} });
            console.log("plain", 3, true);
            console.log();
        }
    "#).await;
    assert_eq!(
        lines,
        [
            r#"Inbox { message: Message { text: "a\"b\nc" }, counts: [ 1, 2 ], empty: Empty {} }"#,
            "plain 3 true",
            ""
        ]
    );
}

#[test]
fn string_conversion_does_not_widen_arbitrary_records_or_functions() {
    for source in [
        "fn main() string { return string({ name: \"a\", index: 1 }); }",
        "fn main() string { return string(fn() number { return 1; }); }",
    ] {
        assert!(
            compiler::compile(source, &Hosts::default()).is_err(),
            "{source}"
        );
    }
}

#[tokio::test]
async fn concatenation_is_not_widened_to_structs() {
    let hosts = Hosts::default();
    let program = compiler::compile(
        r#"struct S { x: number; } fn main() string { return "value: " + S { x: 1 }; }"#,
        &hosts,
    )
    .unwrap();
    let error = Vm::new(program, hosts).unwrap().run().await.unwrap_err();
    assert!(error.contains("invalid arithmetic operands"), "{error}");
}

#[tokio::test]
async fn enum_lookalike_fields_remain_struct_data() {
    let lines = output(
        r#"
        struct Data { name: string; index: number; value: string; }
        fn main() void { console.log(Data { name: "Some", index: 0, value: "data" }); }
    "#,
    )
    .await;
    assert_eq!(lines, [r#"Data { name: "Some", index: 0, value: "data" }"#]);
}

#[test]
fn console_output_requires_a_real_host_sink() {
    let error =
        compiler::compile("fn main() void { console.log(1); }", &Hosts::default()).unwrap_err();
    assert!(
        error.contains("registered echo(string) output operation"),
        "{error}"
    );
}

#[tokio::test]
async fn printing_observes_mutation_without_changing_field_order() {
    let lines = output(
        r#"
        struct Point { x: number; y: number; }
        fn main() void {
            let p = Point { y: 2, x: 1 };
            p.x = 9;
            console.log(p);
        }
    "#,
    )
    .await;
    assert_eq!(lines, ["Point { y: 2, x: 9 }"]);
}
