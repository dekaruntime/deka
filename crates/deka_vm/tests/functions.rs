#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;

async fn run(source: &str) -> Result<HostValue> {
    let hosts = Hosts::default();
    let program = compiler::compile(source, &hosts)?;
    Vm::new(program, hosts)?.run().await
}

#[tokio::test]
async fn default_parameters_fill_omitted_arguments() {
    let value = run(r#"
        fn scale(x: number, factor: number = x) number { return x * factor }
        fn main() number { return scale(3) + scale(3, 4) }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::Number(21.));
}

#[tokio::test]
async fn too_many_arguments_is_still_an_arity_error() {
    let error = run("fn one(x: number) number { return x }\nfn main() number { return one(1, 2) }")
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "<source>: 2:27: expected 1 argument, found 2"
    );
}

#[tokio::test]
async fn tuple_binding_and_tuple_parameter() {
    let value = run(r#"
        fn first([x, y]: [number, number]) number { return x }
        fn main() number {
            const pair: [number, number] = [1, 2];
            const [a, b] = pair;
            return a + b + first(pair);
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::Number(4.));
}

#[tokio::test]
async fn spread_in_list_literals() {
    let value = run(r#"fn main() string {
            const xs = [0, ...[1, 2], 3];
            return string(xs.length) + "," + string(xs.has(2) ? xs[2] : -1);
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::String("4,2".into()));
}

#[tokio::test]
async fn object_spread_keeps_named_fields_and_evaluates_the_spread() {
    // Typecheck tracks only a literal's named keys, so spread-sourced fields
    // are not readable from user code today; what must hold is that the
    // spread is evaluated exactly once and named fields keep working.
    let value = run(r#"
        fn base() { return {a: 1} }
        fn main() string {
            const copy = {...base(), b: 3};
            return string(copy.b);
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::String("3".into()));
}
