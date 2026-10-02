#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;

async fn run(source: &str) -> Result<HostValue> {
    let hosts = Hosts::default();
    let program = compiler::compile(source, &hosts)?;
    Vm::new(program, hosts)?.run().await
}

#[tokio::test]
async fn payload_free_case_reads_name_and_index() {
    let value = run(r#"
        enum Color {
            Red,
            Green,
        }
        fn main() string {
            return Color.Green.name + string(Color.Green.index);
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::String("Green1".into()));
}

#[tokio::test]
async fn case_access_may_precede_the_declaration() {
    let value = run(r#"
        fn main() string {
            return Color.Red.name;
        }
        enum Color {
            Red,
            Green,
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::String("Red".into()));
}

#[tokio::test]
async fn payload_free_cases_are_interned() {
    // The case reads lower to loads of the interned `Color$Red` binding; no
    // record is rebuilt in main.
    let hosts = Hosts::default();
    let program = compiler::compile(
        r#"
        enum Color {
            Red,
            Green,
        }
        fn main() string {
            const a = Color.Red;
            const b = Color.Red;
            return a.name;
        }"#,
        &hosts,
    )
    .unwrap();
    let main = program.functions.iter().find(|f| f.name == "main").unwrap();
    assert!(
        !main.code.iter().any(|op| matches!(op, Op::Record(_))),
        "case reads must not rebuild the record"
    );
}

#[tokio::test]
async fn payload_case_builds_a_record_with_the_value() {
    let value = run(r#"
        enum Shape {
            Circle(number),
            Empty,
        }
        fn main() string {
            const s = Shape.Circle(5);
            return s.name + string(s.index);
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::String("Circle0".into()));
}

#[tokio::test]
async fn unknown_case_is_a_compile_error() {
    let err = compiler::compile(
        r#"
        enum Color {
            Red,
        }
        fn main() string {
            return Color.Blue.name;
        }"#,
        &Hosts::default(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("Color"), "{err}");
}

#[tokio::test]
async fn generic_enum_declaration_erases_its_type_parameters() {
    let value = run(r#"
        enum Box<T> {
            Empty,
            Full(T),
        }
        fn main() number {
            const b = Box.Full(5);
            const e = Box.Empty;
            return 7;
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::Number(7.));
}
