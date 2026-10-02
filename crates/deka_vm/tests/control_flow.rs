#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;

async fn run(source: &str) -> Result<HostValue> {
    let hosts = Hosts::default();
    let program = compiler::compile(source, &hosts)?;
    Vm::new(program, hosts)?.run().await
}

#[tokio::test]
async fn for_of_iterates_a_list() {
    let value = run(r#"fn main() string {
            let out = "";
            for (const item of ["a", "b", "c"]) {
                out = out + item;
            }
            return out;
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::String("abc".into()));
}

#[tokio::test]
async fn break_and_continue_in_both_loop_forms() {
    let value = run(r#"fn main() string {
            let out = "";
            for (let i = 0; i < 10; i = i + 1) {
                if (i == 2) { continue; }
                if (i == 4) { break; }
                out = out + string(i);
            }
            for (const x of [1, 2, 3, 4]) {
                if (x == 2) { continue; }
                if (x == 4) { break; }
                out = out + string(x);
            }
            return out;
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::String("01313".into()));
}

#[tokio::test]
async fn closures_created_in_a_loop_capture_that_turns_value() {
    // With one shared cell per binding, `last()` in turn 2 would read turn
    // 2's value (2, 3, 3). Per-iteration rebinding gives 1, 2, 3.
    let value = run(r#"fn main() string {
            let out = "";
            let last = fn() number { return -1 };
            for (const x of [1, 2, 3]) {
                const cur = fn() number { return x };
                if (x > 1) { out = out + string(last()) + ","; }
                last = cur;
            }
            return out + string(last());
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::String("1,2,3".into()));
}

#[tokio::test]
async fn await_inside_a_loop_body() {
    let value = run(r#"
        async fn double(x: number) Promise<number> { return x * 2 }
        async fn run() Promise<number> {
            let total = 0;
            for (const x of [1, 2, 3]) {
                total = total + await double(x);
            }
            return total;
        }
        async fn main() Promise<number> { return await run() }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::Number(12.));
}
