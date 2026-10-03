#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;

async fn run(source: &str) -> HostValue {
    let hosts = Hosts::default();
    let program = compiler::compile(source, &hosts).unwrap();
    let program = serde_json::from_str(&serde_json::to_string(&program).unwrap()).unwrap();
    Vm::new(program, hosts).unwrap().run().await.unwrap()
}

#[tokio::test]
async fn nested_cases_extract_data_and_failed_payload_patterns_try_the_next_arm() {
    assert_eq!(run(r#"
        fn label(v: Option<Result<number, string>>) string {
            return match (v) {
                Some(Ok(7)) => "seven",
                Some(Ok(x)) => "number:" + string(x),
                Some(Err(e)) => "error:" + e,
                None => "empty",
            };
        }
        fn main() string {
            return label(Option.Some(Result.Ok(7))) + "," + label(Some(Ok(8))) + "," + label(Option.Some(Result.Err("bad"))) + "," + label(None);
        }
    "#).await, HostValue::String("seven,number:8,error:bad,empty".into()));
}

#[tokio::test]
async fn bare_declared_case_names_and_or_patterns_are_tests_not_bindings() {
    assert_eq!(
        run(r#"
        enum Color { Red, Green, Blue }
        fn label(v: Color) string {
            return match(v) { Red | Green => "warm", Blue => "blue" };
        }
        fn main() string {
            return label(Color.Red) + "," + label(Color.Green) + "," + label(Color.Blue);
        }
    "#)
        .await,
        HostValue::String("warm,warm,blue".into())
    );
}

#[tokio::test]
async fn literal_defaults_numbers_strings_and_booleans_return_selected_values() {
    assert_eq!(run(r#"
        fn numberLabel(v: number) string { return match(v) { 1 => "one", _ => "other" }; }
        fn stringLabel(v: string) string { return match(v) { "hi" => "hello", other => other }; }
        fn boolLabel(v: boolean) string { return match(v) { true => "yes", _ => "no" }; }
        fn main() string {
            return numberLabel(1) + "," + numberLabel(2) + "," + stringLabel("hi") + "," + stringLabel("bye") + "," + boolLabel(true) + "," + boolLabel(false);
        }
    "#).await, HostValue::String("one,other,hello,bye,yes,no".into()));
}

#[tokio::test]
async fn match_evaluates_the_subject_once_and_only_runs_the_selected_body() {
    assert_eq!(
        run(r#"
        fn main() string {
            let calls = 0;
            const next = fn() Option<number> { calls = calls + 1; return Some(calls); };
            const result = match(next()) { None => panic("wrong branch"), Some(x) => x };
            return string(result) + "," + string(calls);
        }
    "#)
        .await,
        HostValue::String("1,1".into())
    );
}

#[tokio::test]
async fn arm_bindings_shadow_locally_and_closures_keep_their_iteration() {
    assert_eq!(run(r#"
        fn main() string {
            const x = 99;
            let callbacks: Array<fn() number> = [];
            for (let i = 0; i < 3; i = i + 1) {
                const value = match(Some(i)) { Some(x) => fn() number { return x; }, None => fn() number { return 10; } };
                callbacks.push(value);
            }
            return (callbacks.has(0) ? string(callbacks[0]()) : "missing") + ","
                + (callbacks.has(1) ? string(callbacks[1]()) : "missing") + ","
                + (callbacks.has(2) ? string(callbacks[2]()) : "missing") + "," + string(x);
        }
    "#).await, HostValue::String("0,1,2,99".into()));
}

#[tokio::test]
async fn nested_tuple_and_struct_fields_support_refutable_patterns() {
    assert_eq!(run(r#"
        struct Pair { first: number; second: number; }
        enum Item { Pair(Pair), Empty }
        fn label(v: Item) number {
            return match(v) {
                Pair(Pair { first: 1, second: n }) => n,
                Pair(Pair { first: a, second: b }) => a + b,
                Empty => 0,
            };
        }
        fn main() string {
            const pair: [number, number] = [2, 3];
            const sum = match(pair) { (1, x) => x, (a, b) => a + b };
            return string(label(Item.Pair(Pair { first: 1, second: 7 }))) + "," + string(label(Item.Pair(Pair { first: 2, second: 7 }))) + "," + string(sum);
        }
    "#).await, HostValue::String("7,9,5".into()));
}

#[tokio::test]
async fn union_patterns_use_nominal_metadata_and_do_not_read_a_wrong_kind_payload() {
    assert_eq!(run(r#"
        enum Shape { Circle(number), Empty }
        fn label(v: Shape | number | string) string {
            return match(v) { Circle(n) => "circle:" + string(n), Shape.Empty => "empty", number(n) => "number:" + string(n), string(s) => s };
        }
        fn literal(v: number | string) string { return match(v) { 1 => "one", _ => "other" }; }
        fn main() string {
            return label(Shape.Circle(3)) + "," + label(Shape.Empty) + "," + label(2) + "," + label("hi") + "," + literal("1");
        }
    "#).await, HostValue::String("circle:3,empty,number:2,hi,other".into()));
}

#[test]
fn non_exhaustiveness_keeps_the_original_checker_diagnostic() {
    let error = compiler::compile(
        "fn f(v: Option<number>) number { return match(v) { Some(n) => n }; }",
        &Hosts::default(),
    )
    .unwrap_err();
    assert!(
        error.contains("non-exhaustive match") && error.contains("Option::None"),
        "{error}"
    );
    let nested = compiler::compile("fn f(v: Option<Result<number, string>>) number { return match(v) { Some(Ok(n)) => n, None => 0 }; }", &Hosts::default()).unwrap_err();
    assert!(
        nested.contains("non-exhaustive match") && nested.contains("Result::Err"),
        "{nested}"
    );
}

#[tokio::test]
async fn ordinary_record_shape_cannot_match_a_nominal_enum_case() {
    let program = Program {
        version: 1,
        functions: vec![Function {
            name: "main".into(),
            parameters: 0,
            captures: 0,
            locals: 0,
            asynchronous: false,
            code: vec![
                Op::Const(Literal::String("Some".into())),
                Op::Const(Literal::Number(0.)),
                Op::Record(vec!["name".into(), "index".into()]),
                Op::MatchEnum {
                    name: Some("Option".into()),
                    case: "Some".into(),
                },
                Op::Return,
            ],
        }],
    };
    let value = Vm::new(program, Hosts::default())
        .unwrap()
        .run()
        .await
        .unwrap();
    assert_eq!(value, HostValue::Bool(false));
}
