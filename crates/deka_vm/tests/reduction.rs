#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;

async fn run(source: &str) -> Result<HostValue> {
    let hosts = Hosts::default();
    let program = compiler::compile(source, &hosts)?;
    let program = serde_json::from_str(&serde_json::to_string(&program).unwrap()).unwrap();
    Vm::new(program, hosts)?.run().await
}

#[tokio::test]
async fn folds_left_with_a_named_closure_and_evaluates_inputs_once() {
    assert_eq!(
        run(r#"
        fn main() number {
            let calls = 0;
            const values = fn() Array<number> { calls = calls + 1; return [8, 2, 1]; };
            const reducer = fn() fn(number, number) number {
                calls = calls + 10;
                return fn(acc: number, item: number) number { return acc - item; };
            };
            const result = values().reduce(reducer());
            return result * 100 + calls;
        }
    "#)
        .await
        .unwrap(),
        HostValue::Number(511.)
    );
}

#[tokio::test]
async fn singleton_skips_the_callback_and_empty_input_has_a_named_error() {
    assert_eq!(
        run(r#"
        fn main() number {
            return [7].reduce(fn(acc: number, item: number) number { panic("unused"); });
        }
    "#)
        .await
        .unwrap(),
        HostValue::Number(7.)
    );
    assert_eq!(
        run(r#"
        fn main() number {
            const empty: Array<number> = [];
            return empty.reduce(fn(acc: number, item: number) number { return acc + item; });
        }
    "#)
        .await
        .unwrap_err(),
        "cannot reduce an empty array without an initial value"
    );
}

#[tokio::test]
async fn captures_initial_length_but_reads_elements_after_callback_mutations() {
    assert_eq!(
        run(r#"
        fn main() string {
            let items = [1, 2, 3];
            const result = items.reduce(fn(acc: number, item: number) number {
                items.push(9);
                if (items.has(2)) { items[2] = 7; }
                return acc + item;
            });
            return string(result) + ":" + string(items.length);
        }
    "#)
        .await
        .unwrap(),
        HostValue::String("10:5".into())
    );
    assert_eq!(
        run(r#"
        fn main() number {
            let items = [1, 2, 3, 4];
            return items.reduce(fn(acc: number, item: number) number {
                items.pop(); items.pop(); return acc + item;
            });
        }
    "#)
        .await
        .unwrap(),
        HostValue::Number(3.)
    );
}

#[tokio::test]
async fn ordinary_fields_and_methods_named_reduce_keep_their_dispatch() {
    assert_eq!(
        run(r#"
        fn main() number {
            const record = { reduce: fn(cb: fn(number, number) number) number {
                return cb(40, 2);
            } };
            return record.reduce(fn(acc: number, item: number) number { return acc + item; });
        }
    "#)
        .await
        .unwrap(),
        HostValue::Number(42.)
    );
    assert_eq!(
        run(r#"
        struct Counter { total: number; }
        fn (c Counter) reduce(amount: number) number { return c.total + amount; }
        fn main() number { return Counter { total: 40 }.reduce(2); }
    "#)
        .await
        .unwrap(),
        HostValue::Number(42.)
    );
}

#[tokio::test]
async fn callback_faults_propagate_and_closures_survive_task_slices_and_gc() {
    assert_eq!(run(r#"
        fn main() number {
            return [1, 2].reduce(fn(acc: number, item: number) number { panic("callback failed"); });
        }
    "#).await.unwrap_err(), "callback failed");
    assert_eq!(
        run(r#"
        fn main() number {
            let items = [0]; let i = 1; let calls = 0;
            for (; i < 900; i = i + 1) { items.push(i); }
            const result = items.reduce(fn(acc: number, item: number) number {
                calls = calls + 1; return acc + item;
            });
            return result + calls;
        }
    "#)
        .await
        .unwrap(),
        HostValue::Number(405449.)
    );
}

#[test]
fn checker_rejects_wrong_callback_types_and_extra_arguments() {
    for source in [
        "fn main() number { return [1, 2].reduce(7); }",
        "fn main() number { return [1, 2].reduce(fn(a: number, b: string) number { return a; }); }",
        "fn main() number { return [1, 2].reduce(fn(a: number, b: number) number { return a + b; }, 0); }",
    ] {
        let error = compiler::compile(source, &Hosts::default()).unwrap_err();
        assert!(!error.contains("unsupported"), "{error}");
    }
}
