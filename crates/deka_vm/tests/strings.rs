#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;

async fn run(source: &str) -> Result<HostValue> {
    let hosts = Hosts::default();
    let program = compiler::compile(source, &hosts)?;
    Vm::new(program, hosts)?.run().await
}

#[tokio::test]
async fn string_ordering_comparisons() {
    let value = run(r#"fn main() string {
            return string("a" < "b") + string("b" <= "b") + string("c" > "a") + string("a" >= "z");
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::String("truetruetruefalse".into()));
}

#[tokio::test]
async fn string_length_and_indexing() {
    let value = run(r#"fn main() string {
            const s = "hello";
            return string(s.length) + s[0] + s[4];
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::String("5ho".into()));
}

#[tokio::test]
async fn string_index_out_of_bounds_is_an_error() {
    let error = run(r#"fn main() string { return "abc"[3]; }"#)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "index out of bounds");
}

#[tokio::test]
async fn string_plus_number_concatenates_the_way_string_does() {
    let value = run(r#"fn main() string {
            return ("" + 1) + "," + ("n=" + 3.14) + "," + (0 + " zero") + "," + ("b=" + true);
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::String("1,n=3.14,0 zero,b=true".into()));
}
