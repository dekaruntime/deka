#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;

async fn run(source: &str) -> Result<HostValue> {
    let hosts = Hosts::default();
    let program = compiler::compile(source, &hosts)?;
    Vm::new(program, hosts)?.run().await
}

#[tokio::test]
async fn struct_literal_reads_fields() {
    let value = run(r#"
        struct Point {
            x: number;
            y: number;
        }
        fn main() number {
            const p = Point { x: 1, y: 2 };
            return p.x + p.y;
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::Number(3.));
}

#[tokio::test]
async fn omitted_field_falls_back_to_its_default() {
    let value = run(r#"
        struct Config {
            name: string;
            retries: number = 3;
        }
        fn main() number {
            const c = Config { name: "web" };
            return c.retries;
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::Number(3.));
}

#[tokio::test]
async fn given_field_overrides_the_default() {
    let value = run(r#"
        struct Config {
            name: string;
            retries: number = 3;
        }
        fn main() number {
            const c = Config { name: "db", retries: 9 };
            return c.retries;
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::Number(9.));
}

#[tokio::test]
async fn optional_field_stays_absent_when_omitted() {
    // The typechecker types an omitted optional field's read as Option, so
    // absence is observed in the bytecode: the record has no `next` key.
    let hosts = Hosts::default();
    let program = compiler::compile(
        r#"
        struct Node {
            value: number;
            next?: number;
        }
        fn main() number {
            const n = Node { value: 1 };
            return n.value;
        }"#,
        &hosts,
    )
    .unwrap();
    let records: Vec<&Vec<String>> = program
        .functions
        .iter()
        .flat_map(|f| &f.code)
        .filter_map(|op| match op {
            Op::Record(names) => Some(names),
            _ => None,
        })
        .collect();
    assert_eq!(records, vec![&vec!["value".to_string()]]);
}

#[test]
fn defaults_are_appended_in_declaration_order() {
    let hosts = Hosts::default();
    let program = compiler::compile(
        r#"
        struct Config {
            name: string;
            retries: number = 3;
            verbose: boolean = false;
        }
        fn main() string {
            const c = Config { name: "web", verbose: true };
            return c.name;
        }"#,
        &hosts,
    )
    .unwrap();
    let names = program
        .functions
        .iter()
        .flat_map(|f| &f.code)
        .find_map(|op| match op {
            Op::Record(names) => Some(names.clone()),
            _ => None,
        })
        .unwrap();
    // Written fields first in written order, then the omitted defaulted
    // field; the given `verbose` is not filled twice.
    assert_eq!(names, vec!["name", "verbose", "retries"]);
}
