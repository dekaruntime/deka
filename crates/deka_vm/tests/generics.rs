#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;

async fn run(source: &str) -> Result<HostValue> {
    let hosts = Hosts::default();
    let program = compiler::compile(source, &hosts)?;
    Vm::new(program, hosts)?.run().await
}

#[tokio::test]
async fn generic_function_erases_to_one_copy() {
    let value = run(r#"
        fn identity<T>(x: T) T {
            return x;
        }
        fn main() number {
            return identity(40) + identity(2);
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::Number(42.));
}

#[tokio::test]
async fn explicit_type_arguments_at_the_call() {
    let value = run(r#"
        fn identity<T>(x: T) T {
            return x;
        }
        fn main() string {
            return identity<string>("a") + string(identity<number>(1));
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::String("a1".into()));
}

#[tokio::test]
async fn generic_receiver_method() {
    let value = run(r#"
        struct Signal<T> {
            value: T;
        }
        fn (s Signal<T>) get() T {
            return s.value;
        }
        fn main() number {
            const s = Signal { value: 42 };
            return s.get();
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::Number(42.));
}

#[tokio::test]
async fn generic_interface_bound_still_dispatches() {
    let value = run(r#"
        interface Named {
            name: string;
        }
        struct Holder<T> {
            item: T;
        }
        fn (h Holder<T: Named>) label() string {
            return h.item.name;
        }
        struct User {
            name: string;
        }
        fn main() string {
            const h = Holder { item: User { name: "Ada" } };
            return h.label();
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::String("Ada".into()));
}
