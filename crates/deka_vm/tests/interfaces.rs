#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;

async fn run(source: &str) -> Result<HostValue> {
    let hosts = Hosts::default();
    let program = compiler::compile(source, &hosts)?;
    Vm::new(program, hosts)?.run().await
}

#[tokio::test]
async fn interface_method_finds_the_concrete_method_at_run_time() {
    let value = run(r#"
        interface Greeter {
            fn greet() string;
        }
        struct Person {
            name: string;
        }
        fn (p Person) greet() string {
            return "Hello, " + p.name;
        }
        fn welcome(g: Greeter) string {
            return g.greet();
        }
        fn main() string {
            return welcome(Person { name: "Deka" });
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::String("Hello, Deka".into()));
}

#[tokio::test]
async fn two_types_behind_one_interface() {
    let value = run(r#"
        interface Speaker {
            fn speak() string;
        }
        struct Dog {}
        struct Cat {}
        fn (d Dog) speak() string {
            return "woof";
        }
        fn (c Cat) speak() string {
            return "meow";
        }
        fn say(s: Speaker) string {
            return s.speak();
        }
        fn main() string {
            return say(Dog {  }) + "," + say(Cat {  });
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::String("woof,meow".into()));
}

#[tokio::test]
async fn mut_method_through_an_interface_mutates_the_original() {
    let value = run(r#"
        interface Counter {
            fn increment() void;
        }
        struct Clicker {
            count: number;
        }
        fn (c mut Clicker) increment() void {
            c.count = c.count + 1;
        }
        fn bump(i: Counter) void {
            i.increment();
        }
        fn main() number {
            let c = Clicker { count: 0 };
            bump(c);
            bump(c);
            return c.count;
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::Number(2.));
}

#[tokio::test]
async fn embedded_method_satisfies_the_interface() {
    let value = run(r#"
        interface Mover {
            fn move() string;
        }
        struct Legs {}
        fn (l Legs) move() string {
            return "walking";
        }
        struct Robot {
            Legs;
        }
        fn go(m: Mover) string {
            return m.move();
        }
        fn main() string {
            return go(Robot { Legs: Legs {  } });
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::String("walking".into()));
}

#[tokio::test]
async fn interface_method_with_params_and_return() {
    let value = run(r#"
        interface Counter {
            value: number;
            fn next() Counter;
        }
        struct Step {
            value: number;
        }
        fn (s Step) next() Counter {
            return Step { value: s.value + 1 };
        }
        fn run(c: Counter) number {
            return c.next().value;
        }
        fn main() number {
            return run(Step { value: 41 });
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::Number(42.));
}

#[tokio::test]
async fn record_field_holding_a_function_still_calls_plainly() {
    // Not a method: the field is data, no receiver is passed.
    let value = run(r#"
        fn main() number {
            const o = {f: fn (x: number) number { return x * 2 }};
            return o.f(21);
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::Number(42.));
}

#[tokio::test]
async fn attached_method_does_not_shadow_a_data_read() {
    // The `$greet` attachment is hidden: the record's data reads are
    // unaffected.
    let value = run(r#"
        interface Greeter {
            fn greet() string;
        }
        struct Person {
            name: string;
        }
        fn (p Person) greet() string {
            return "hi";
        }
        fn main() string {
            const p = Person { name: "Ada" };
            return p.name;
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::String("Ada".into()));
}
