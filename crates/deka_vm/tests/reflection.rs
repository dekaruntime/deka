#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;

async fn run(source: &str) -> HostValue {
    let hosts = Hosts::default();
    let program = compiler::compile(source, &hosts).unwrap();
    Vm::new(program, hosts).unwrap().run().await.unwrap()
}

#[tokio::test]
async fn runtime_type_and_declared_union_signature_are_different_questions() {
    let value = run(r#"
        fn main() string {
            let value: number | string = "hello";
            const first = value.getType().toString();
            value = 42;
            return first + "," + value.getType().toString() + "," + value.signature().toString();
        }
    "#)
    .await;
    assert_eq!(
        value,
        HostValue::String("string,number,number | string".into())
    );
}

#[tokio::test]
async fn descriptors_are_interned_and_compare_by_identity() {
    assert_eq!(run(r#"fn main() boolean { return "a".getType() == "b".getType() && "a".getType() != 1.getType(); }"#).await, HostValue::Bool(true));
}

#[tokio::test]
async fn nominal_struct_enum_and_newtype_identity_survives_execution() {
    assert_eq!(run(r#"
        struct Point { x: number; }
        enum Color { Red, Green }
        type Cents number
        fn main() string {
            return Point { x: 1 }.getType().toString() + "," + Color.Red.getType().toString() + "," + Cents(5).getType().toString();
        }
    "#).await, HostValue::String("Point,Color,Cents".into()));
}

#[tokio::test]
async fn naming_a_newtype_does_not_brand_the_original_primitive() {
    assert_eq!(run(r#"
        type Cents number
        fn main() string {
            const n = 5;
            const c = Cents(n);
            const sum = c + Cents(2);
            return n.getType().toString() + "," + sum.getType().toString() + "," + unboxNumber(sum).getType().toString();
        }
    "#).await, HostValue::String("number,Cents,number".into()));
}

#[tokio::test]
async fn declared_user_methods_shadow_reflection_builtins() {
    assert_eq!(
        run(r#"
        fn (s string) getType() string { return "user:" + s; }
        struct Item { x: number; }
        fn (i Item) signature() string { return "custom"; }
        fn main() string { return "hi".getType() + "," + Item { x: 1 }.signature(); }
    "#)
        .await,
        HostValue::String("user:hi,custom".into())
    );
}

#[test]
fn reflection_keeps_checker_arity_errors() {
    let error =
        compiler::compile("fn main() void { 1.getType(2); }", &Hosts::default()).unwrap_err();
    assert!(error.contains("getType` expects no arguments"), "{error}");
}

#[tokio::test]
async fn repeated_reflection_reuses_one_descriptor_and_clears_it_at_exit() {
    let source = format!("fn main() void {{ {} }}", "\"x\".getType();".repeat(1000));
    let hosts = Hosts::default();
    let program = compiler::compile(&source, &hosts).unwrap();
    let encoded = serde_json::to_vec(&program).unwrap();
    let program: Program = serde_json::from_slice(&encoded).unwrap();
    let mut vm = Vm::new(program, hosts).unwrap();
    vm.run().await.unwrap();
    // One input string per call, one interned Type; rebuilding a descriptor
    // per call would more than double these allocations.
    assert!(vm.stats().allocations < 1050, "{:?}", vm.stats());
    assert_eq!(vm.stats().live, 0);
}

#[tokio::test]
async fn signature_evaluates_the_receiver_once_and_arrays_report_runtime_array() {
    assert_eq!(
        run(r#"
        fn main() string {
            let calls = 0;
            const next = fn() number { calls = calls + 1; return 7; };
            const signature = next().signature().toString();
            return signature + "," + string(calls) + "," + [1, 2].getType().toString();
        }
    "#)
        .await,
        HostValue::String("number,1,Array".into())
    );
}

#[tokio::test]
async fn newtype_arithmetic_preserves_the_checked_result_type() {
    assert_eq!(run(r#"
        type Cents number
        fn main() string {
            const c = Cents(8);
            return (-c).getType().toString() + "," + (c * 2).getType().toString() + "," + (c / Cents(2)).getType().toString();
        }
    "#).await, HostValue::String("Cents,Cents,number".into()));
}

#[tokio::test]
async fn record_shape_does_not_impersonate_enum_identity() {
    assert_eq!(run(r#"
        enum Shape { Circle(number) }
        fn main() string {
            return { name: "Circle", index: 0, value: 5 }.getType().toString() + "," + Shape.Circle(5).getType().toString();
        }
    "#).await, HostValue::String("Object,Shape".into()));
}

#[tokio::test]
async fn reflection_distinguishes_runtime_type_from_an_interface_view() {
    assert_eq!(
        run(r#"
        interface Named { name: string; }
        struct Person { name: string; }
        fn inspect(v: Named) string { return v.getType().toString(); }
        fn declared(v: Named) string { return v.signature().toString(); }
        fn main() string {
            const person = Person { name: "Ada" };
            return inspect(person) + "," + declared(person);
        }
    "#)
        .await,
        HostValue::String("Person,Named".into())
    );
}
