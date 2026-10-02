#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;

async fn run(source: &str) -> Result<HostValue> {
    let hosts = Hosts::default();
    let program = compiler::compile(source, &hosts)?;
    Vm::new(program, hosts)?.run().await
}

#[tokio::test]
async fn struct_method_call_is_static_dispatch() {
    let value = run(r#"
        struct Point {
            x: number;
            y: number;
        }
        fn (p Point) sum() number {
            return p.x + p.y;
        }
        fn main() number {
            const p = Point { x: 3, y: 4 };
            return p.sum();
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::Number(7.));
}

#[tokio::test]
async fn method_call_may_precede_the_declaration() {
    let value = run(r#"
        struct Counter {
            count: number;
        }
        fn main() number {
            const c = Counter { count: 1 };
            return c.incremented().count;
        }
        fn (c Counter) incremented() Counter {
            return Counter { count: c.count + 1 };
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::Number(2.));
}

#[tokio::test]
async fn mut_receiver_mutates_the_shared_value() {
    let value = run(r#"
        struct Counter {
            count: number;
        }
        fn (c mut Counter) inc() void {
            c.count = c.count + 1;
        }
        fn main() number {
            let c = Counter { count: 0 };
            c.inc();
            c.inc();
            return c.count;
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::Number(2.));
}

#[tokio::test]
async fn superpower_on_a_builtin_type() {
    let value = run(r#"
        fn (s string) shout() string {
            return s;
        }
        fn main() string {
            return "hi".shout();
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::String("hi".into()));
}

#[tokio::test]
async fn method_with_params_takes_them_after_the_receiver() {
    let value = run(r#"
        struct Point {
            x: number;
            y: number;
        }
        fn (p Point) moved(dx: number, dy: number) Point {
            return Point { x: p.x + dx, y: p.y + dy };
        }
        fn main() number {
            return Point { x: 1, y: 2 }.moved(10, 20).y;
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::Number(22.));
}

#[tokio::test]
async fn embedded_method_promotes_from_a_nested_literal() {
    let value = run(r#"
        struct Mover {}
        fn (m Mover) move() string {
            return "moving";
        }
        struct Robot {
            Mover;
        }
        fn main() string {
            const r = Robot { Mover: Mover {  } };
            return r.move();
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::String("moving".into()));
}

#[tokio::test]
async fn embedded_method_promotes_from_a_flat_literal() {
    let value = run(r#"
        struct Person {
            name: string;
        }
        fn (p Person) greet() string {
            return "hello";
        }
        struct Employee {
            Person;
        }
        fn main() string {
            const e = Employee { name: "Bob" };
            return e.greet();
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::String("hello".into()));
}

#[tokio::test]
async fn newtype_method_reads_the_payload() {
    let value = run(r#"
        type Cents number
        fn (c Cents) double() Cents {
            return Cents(unboxNumber(c) * 2);
        }
        fn main() number {
            return unboxNumber(Cents(21).double());
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::Number(42.));
}

#[tokio::test]
async fn struct_name_called_as_a_constructor() {
    let value = run(r#"
        struct Person {
            name: string;
        }
        fn (p Person) greet() string {
            return "hi, " + p.name;
        }
        fn main() string {
            const p = Person({name: "Ada"});
            return p.greet();
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::String("hi, Ada".into()));
}

#[tokio::test]
async fn method_on_one_type_does_not_leak_to_another() {
    // The typechecker rejects the call; the VM must not see a binding for it.
    let err = compiler::compile(
        r#"
        struct A { x: number; }
        struct B { x: number; }
        fn (a A) get() number { return a.x; }
        fn main() number {
            const b = B { x: 1 };
            return b.get();
        }"#,
        &Hosts::default(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("has no field"), "{err}");
}
