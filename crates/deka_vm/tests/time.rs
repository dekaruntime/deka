#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;
use std::task::{Context, Poll, Waker};
fn timer_vm(source: &str) -> Vm {
    let mut hosts = Hosts::default();
    deka_vm::time::register(&mut hosts).unwrap();
    Vm::new(compiler::compile(source, &hosts).unwrap(), hosts).unwrap()
}
fn finish(vm: &mut Vm, cx: &mut Context<'_>) -> HostValue {
    for _ in 0..64 {
        if let Poll::Ready(value) = vm.poll(cx) {
            return value.unwrap();
        }
    }
    panic!("timer did not complete at its deadline");
}
#[tokio::test(start_paused = true)]
async fn production_sleep_suspends_until_its_duration_and_releases_roots() {
    let mut vm = timer_vm(
        r#"
import { sleep } from "time";
async fn main() Promise<number> { await sleep(1000); return 7; }
"#,
    );
    let mut cx = Context::from_waker(Waker::noop());
    for _ in 0..8 {
        assert!(vm.poll(&mut cx).is_pending());
    }
    vm.collect().unwrap();
    assert!(vm.stats().live > 0);
    tokio::time::advance(std::time::Duration::from_millis(999)).await;
    assert!(vm.poll(&mut cx).is_pending());
    tokio::time::advance(std::time::Duration::from_millis(1)).await;
    assert_eq!(finish(&mut vm, &mut cx), HostValue::Number(7.));
    assert_eq!(vm.stats().live, 0);
}
#[tokio::test(start_paused = true)]
async fn concurrent_sleep_does_not_delay_ready_work() {
    let mut vm = timer_vm(
        r#"
import { sleep } from "time";
let state = 0;
async fn slow() Promise<void> { await sleep(1000); state += 10; }
async fn fast() Promise<void> { state += 1; }
async fn main() Promise<number> {
    const a = slow(); const b = fast();
    await b;
    if (state != 1) { return 0; }
    await a;
    return state;
}
"#,
    );
    let mut cx = Context::from_waker(Waker::noop());
    for _ in 0..8 {
        assert!(vm.poll(&mut cx).is_pending());
    }
    tokio::time::advance(std::time::Duration::from_millis(1000)).await;
    assert_eq!(finish(&mut vm, &mut cx), HostValue::Number(11.));
}
#[tokio::test(start_paused = true)]
async fn zero_duration_works_and_invalid_arguments_are_rejected() {
    assert_eq!(
        timer_vm(
            "import { sleep } from \"time\"; async fn main() Promise<void> { await sleep(0); }"
        )
        .run()
        .await
        .unwrap(),
        HostValue::Unit
    );
    for ms in [-1., f64::NAN, f64::INFINITY, f64::MAX] {
        assert!(deka_vm::time::sleep(ms).is_err());
    }
    let mut vm = timer_vm(
        "import { sleep } from \"time\"; async fn main() Promise<void> { await sleep(0 - 1); }",
    );
    assert!(
        vm.run()
            .await
            .unwrap_err()
            .contains("sleep duration must be finite, nonnegative milliseconds")
    );
    assert_eq!(vm.stats().live, 0);
    let mut hosts = Hosts::default();
    deka_vm::time::register(&mut hosts).unwrap();
    assert!(compiler::compile("import { sleep } from \"time\"; async fn main() Promise<void> { await sleep(\"wrong\"); }", &hosts).is_err());
}
#[tokio::test(start_paused = true)]
async fn cancelling_a_pending_timer_releases_the_vm() {
    let mut vm = timer_vm(
        "import { sleep } from \"time\"; async fn main() Promise<void> { await sleep(1000); }",
    );
    let mut cx = Context::from_waker(Waker::noop());
    for _ in 0..8 {
        assert!(vm.poll(&mut cx).is_pending());
    }
    vm.cancel().unwrap();
    tokio::time::advance(std::time::Duration::from_millis(1000)).await;
    assert_eq!(vm.stats().live, 0);
    assert_eq!(vm.pending_tasks(), 0);
    assert!(matches!(vm.poll(&mut cx), Poll::Ready(Err(_))));
}
