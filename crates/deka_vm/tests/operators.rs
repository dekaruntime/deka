#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;

async fn run(source: &str) -> Result<HostValue> {
    let hosts = Hosts::default();
    let program = compiler::compile(source, &hosts)?;
    Vm::new(program, hosts)?.run().await
}

#[tokio::test]
async fn comparison_operators() {
    let value = run(
        r#"fn main() string {
            return string(3 != 2) + string(2 <= 2) + string(3 > 2) + string(5 >= 5) + string("a" != "b");
        }"#,
    )
    .await
    .unwrap();
    assert_eq!(value, HostValue::String("truetruetruetruetrue".into()));
}

#[tokio::test]
async fn logic_short_circuits() {
    // false && … must not evaluate the right side; a panicking operand
    // proves it. panic() returns never, so wrap it behind a bool call.
    let value = run(r#"
        fn boom() boolean {
            if (true) {
                panic("right side ran")
            }
            return true
        }
        fn main() string {
            return string(false && boom()) + string(true || boom());
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::String("falsetrue".into()));
}

#[tokio::test]
async fn unary_and_modulo_and_compound_assignment() {
    let value = run(r#"fn main() number {
            let k = 10;
            k /= 4;
            k %= 2;
            return -3 + 10 % 3 + k;
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::Number(-1.5));
}

#[tokio::test]
async fn bitwise_operators_follow_javascript_int32() {
    let value = run(r#"fn main() string {
            return string(7 & 3) + "," + string(7 | 8) + "," + string(5 ^ 3) + ","
                + string(1 << 4) + "," + string(256 >> 4) + "," + string(-1 >> 1) + ","
                + string(1 << 33);
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::String("3,15,6,16,16,-1,2".into()));
}

#[tokio::test]
async fn pipe_inserts_first_argument_or_fills_the_hole() {
    let value = run(r#"
        fn add(a: number, b: number) number { return a + b }
        fn inc(n: number) number { return n + 1 }
        fn append(s: string, suffix: string) string { return s + suffix }
        fn main() string {
            return string(5 |> add(1) |> inc) + "," + ("1" |> append("3", _));
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::String("7,31".into()));
}

#[tokio::test]
async fn unawaited_promise_addition_preserves_identity_before_and_after_settlement() {
    let value = run(r#"
        async fn value() Promise<number> { return 7; }
        async fn main() Promise<string> {
            const pending = value();
            const before = pending + 2;
            const actual = await pending;
            return before + "|" + (2 + pending) + "|" + (pending + true)
                + "|" + (false + pending) + "|" + (pending + "text")
                + "|" + ("text" + pending) + "|" + (pending + pending)
                + "|" + string(actual + 2);
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::String(
        "[object Promise]2|2[object Promise]|[object Promise]true|false[object Promise]|[object Promise]text|text[object Promise]|[object Promise][object Promise]|9".into()
    ));
}

#[test]
fn promise_concatenation_does_not_enable_printing_conversion_or_numeric_arithmetic() {
    for expression in [
        "string(pending)",
        "pending - 2",
        "pending * 2",
        "pending / 2",
    ] {
        let source = format!(
            "async fn value() Promise<number> {{ return 7; }} fn main() {{ const pending = value(); const invalid = {expression}; }}"
        );
        assert!(
            compiler::compile(&source, &Hosts::default()).is_err(),
            "{expression}"
        );
    }
}

#[test]
fn unawaited_promises_still_have_no_direct_console_printable_form() {
    let error = compiler::compile(
        "async fn value() Promise<number> { return 7; } fn main() { const pending = value(); console.log(pending); }",
        &Hosts::default(),
    ).unwrap_err();
    assert!(
        error.contains("Promise<number> has no printable form"),
        "{error}"
    );
}
