#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;

async fn run(source: &str) -> Result<HostValue> {
    let hosts = Hosts::default();
    let program = compiler::compile(source, &hosts)?;
    Vm::new(program, hosts)?.run().await
}

#[tokio::test]
async fn string_widens_number_bool_and_passes_strings_through() {
    let value = run(
        r#"fn main() string {
            return string(7) + "," + string(3.14) + "," + string(true) + "," + string(false) + "," + string("hi");
        }"#,
    )
    .await
    .unwrap();
    assert_eq!(value, HostValue::String("7,3.14,true,false,hi".into()));
}

#[tokio::test]
async fn to_number_widens_bool_and_passes_numbers_through() {
    let value = run("fn main() number { return toNumber(true) + toNumber(false) + toNumber(7); }")
        .await
        .unwrap();
    assert_eq!(value, HostValue::Number(8.));
}

#[tokio::test]
async fn panic_stops_the_program_with_its_message() {
    let error = run(r#"fn main() void { panic("boom"); }"#)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "boom");
}

#[test]
fn a_builtin_the_vm_does_not_know_says_so() {
    let hosts = Hosts::default();
    let error = compiler::compile(r#"const x = parseNumber("42");"#, &hosts).unwrap_err();
    assert_eq!(
        error.to_string(),
        "<source>: 1:11: unknown built-in parseNumber"
    );
}

#[test]
fn a_forward_reference_keeps_its_own_error() {
    let hosts = Hosts::default();
    let error = compiler::compile("later();\nfn later() {}\n", &hosts).unwrap_err();
    assert_eq!(
        error.to_string(),
        "<source>: 1:1: binding later is unavailable here; forward references are unsupported"
    );
}
