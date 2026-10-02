#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;

async fn run(source: &str) -> Result<HostValue> {
    let hosts = Hosts::default();
    let program = compiler::compile(source, &hosts)?;
    Vm::new(program, hosts)?.run().await
}

#[tokio::test]
async fn alias_is_the_same_value() {
    let value = run(r#"
        alias Name = string
        fn main() string {
            const n: Name = "hi";
            return n;
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::String("hi".into()));
}

#[tokio::test]
async fn newtype_constructs_and_unboxes_as_the_payload() {
    let value = run(r#"
        type Cents number
        fn main() number {
            const c = Cents(500);
            return unboxNumber(c);
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::Number(500.));
}

#[tokio::test]
async fn newtype_operators_are_the_plain_operators() {
    let value = run(r#"
        type Cents number
        fn main() number {
            const a = Cents(1);
            const b = Cents(2);
            return unboxNumber(a + b);
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::Number(3.));
}

#[tokio::test]
async fn string_newtype_converts_with_string() {
    let value = run(r#"
        type Label string
        fn main() string {
            const l = Label("x");
            return string(l);
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::String("x".into()));
}

#[tokio::test]
async fn opaque_declaration_is_accepted() {
    let value = run("opaque type Handle\nfn main() number { return 1; }")
        .await
        .unwrap();
    assert_eq!(value, HostValue::Number(1.));
}
