#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;
use std::{
    cell::RefCell,
    rc::Rc,
    task::{Context, Poll, Waker},
};
fn setup(source: &str) -> (Vm, Rc<RefCell<Vec<String>>>) {
    let (mut hosts, output) = demo::hosts().unwrap();
    time::register(&mut hosts).unwrap();
    timers::register(&mut hosts).unwrap();
    let program = compiler::compile(source, &hosts).unwrap();
    (Vm::new(program, hosts).unwrap(), output)
}
fn pending(vm: &mut Vm, cx: &mut Context<'_>) {
    for _ in 0..8 {
        assert!(vm.poll(cx).is_pending());
    }
}
fn finish(vm: &mut Vm, cx: &mut Context<'_>) -> HostValue {
    for _ in 0..64 {
        if let Poll::Ready(result) = vm.poll(cx) {
            return result.unwrap();
        }
    }
    panic!("ready timer work did not finish");
}
#[tokio::test(start_paused = true)]
async fn timeout_retains_captures_and_keeps_main_alive_until_delivery() {
    let (mut vm, output) = setup(
        r#"
import { print } from "vm:host";
fn main() number {
    const data = {message: "alive"};
    const timer = setTimeout(fn() { print(data.message); }, 1000);
    return 7;
}
"#,
    );
    let mut cx = Context::from_waker(Waker::noop());
    pending(&mut vm, &mut cx);
    vm.collect().unwrap();
    assert!(output.borrow().is_empty());
    tokio::time::advance(std::time::Duration::from_millis(999)).await;
    assert!(vm.poll(&mut cx).is_pending());
    assert!(output.borrow().is_empty());
    tokio::time::advance(std::time::Duration::from_millis(1)).await;
    assert_eq!(finish(&mut vm, &mut cx), HostValue::Number(7.));
    assert_eq!(&*output.borrow(), &["alive"]);
    assert_eq!(vm.stats().live, 0);
}
#[tokio::test(start_paused = true)]
async fn an_interval_repeats_and_can_clear_itself_without_queued_extra_calls() {
    let (mut vm, output) = setup(
        r#"
import { print } from "vm:host";
let timer = 0;
let count = 0;
fn main() number {
    timer = setInterval(fn() {
        count += 1;
        print(string(count));
        if (count == 3) { clearInterval(timer); }
    }, 10);
    return 7;
}
"#,
    );
    let mut cx = Context::from_waker(Waker::noop());
    pending(&mut vm, &mut cx);
    let mut completion = None;
    for expected in 1..=3 {
        tokio::time::advance(std::time::Duration::from_millis(10)).await;
        for _ in 0..8 {
            match vm.poll(&mut cx) {
                Poll::Ready(result) => {
                    assert_eq!(expected, 3);
                    completion = Some(result.unwrap());
                }
                Poll::Pending => {}
            }
            if output.borrow().len() == expected {
                break;
            }
        }
        assert_eq!(output.borrow().len(), expected);
        if expected < 3 {
            pending(&mut vm, &mut cx);
        }
    }
    assert_eq!(
        completion.unwrap_or_else(|| finish(&mut vm, &mut cx)),
        HostValue::Number(7.)
    );
    assert_eq!(&*output.borrow(), &["1", "2", "3"]);
    assert_eq!(vm.stats().live, 0);
}
#[tokio::test(start_paused = true)]
async fn clear_before_delivery_and_aliasing_globals_work() {
    let (mut vm, output) = setup(
        r#"
import { print } from "vm:host";
fn main() number {
    const schedule = setTimeout;
    const cancelled = schedule(fn() { print("wrong"); }, 1000);
    clearInterval(cancelled);
    const timer = schedule(fn() { print("right"); }, 10);
    return 7;
}
"#,
    );
    let mut cx = Context::from_waker(Waker::noop());
    pending(&mut vm, &mut cx);
    vm.collect().unwrap();
    tokio::time::advance(std::time::Duration::from_millis(10)).await;
    assert_eq!(finish(&mut vm, &mut cx), HostValue::Number(7.));
    assert_eq!(&*output.borrow(), &["right"]);
    assert_eq!(vm.stats().live, 0);
}
#[tokio::test(start_paused = true)]
async fn async_timer_callbacks_survive_the_timer_finishing() {
    let (mut vm, output) = setup(
        r#"
import { print } from "vm:host";
import { sleep } from "time";
fn main() number {
    const timer = setTimeout(async fn() Promise<void> {
        await sleep(100); print("async");
    }, 10);
    return 7;
}
"#,
    );
    let mut cx = Context::from_waker(Waker::noop());
    pending(&mut vm, &mut cx);
    tokio::time::advance(std::time::Duration::from_millis(10)).await;
    pending(&mut vm, &mut cx);
    vm.collect().unwrap();
    assert!(output.borrow().is_empty());
    tokio::time::advance(std::time::Duration::from_millis(100)).await;
    assert_eq!(finish(&mut vm, &mut cx), HostValue::Number(7.));
    assert_eq!(&*output.borrow(), &["async"]);
    assert_eq!(vm.stats().live, 0);
}
#[tokio::test(start_paused = true)]
async fn cancelling_the_vm_disposes_pending_intervals() {
    let (mut vm, output) = setup(
        r#"
import { print } from "vm:host";
fn main() { const timer = setInterval(fn() { print("wrong"); }, 1000); }
"#,
    );
    let mut cx = Context::from_waker(Waker::noop());
    pending(&mut vm, &mut cx);
    vm.cancel().unwrap();
    tokio::time::advance(std::time::Duration::from_millis(1000)).await;
    assert!(matches!(vm.poll(&mut cx), Poll::Ready(Err(_))));
    assert!(output.borrow().is_empty());
    assert_eq!(vm.pending_tasks(), 0);
    assert_eq!(vm.stats().live, 0);
}
#[tokio::test(start_paused = true)]
async fn a_zero_interval_yields_before_calling_back_and_can_be_cleared() {
    let (mut vm, output) = setup(
        r#"
import { print } from "vm:host";
let timer = 0;
fn main() number {
    timer = setInterval(fn() { clearTimeout(timer); print("once"); }, 0);
    return 7;
}
"#,
    );
    let mut cx = Context::from_waker(Waker::noop());
    assert!(vm.poll(&mut cx).is_pending());
    tokio::time::advance(std::time::Duration::from_millis(1)).await;
    assert_eq!(finish(&mut vm, &mut cx), HostValue::Number(7.));
    assert_eq!(&*output.borrow(), &["once"]);
}
#[tokio::test(start_paused = true)]
async fn local_declarations_shadow_timer_globals_and_bad_arguments_are_checked() {
    let (mut vm, _) = setup(
        r#"
fn setTimeout(callback: fn() void, ms: number) number { callback(); return ms; }
let count = 0;
fn main() number { const timer = setTimeout(fn() { count += 1; }, 42); return count + timer; }
"#,
    );
    assert_eq!(vm.run().await.unwrap(), HostValue::Number(43.));
    let mut hosts = Hosts::default();
    timers::register(&mut hosts).unwrap();
    for source in [
        "fn main() { setTimeout(7, 10); }",
        "fn main() { setTimeout(fn() {}, \"wrong\"); }",
        "fn main() { setTimeout(fn(x: number) {}, 10); }",
        "fn main() { clearTimeout(\"wrong\"); }",
    ] {
        assert!(
            compiler::compile(source, &hosts).is_err(),
            "accepted {source}"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn repeatedly_clearing_timers_releases_callback_roots() {
    let (mut vm, output) = setup(
        r#"
fn main() number {
    for (let i = 0; i < 10000; i += 1) {
        const timer = setTimeout(fn() {}, 1000);
        clearTimeout(timer);
    }
    return 7;
}
"#,
    );
    let mut cx = Context::from_waker(Waker::noop());
    let mut peak = 0;
    for _ in 0..10000 {
        let state = vm.poll(&mut cx);
        peak = peak.max(vm.stats().slots);
        if let Poll::Ready(result) = state {
            assert_eq!(result.unwrap(), HostValue::Number(7.));
            assert!(peak < 1024, "peak VM slots: {peak}");
            assert!(vm.stats().allocations > 40000);
            assert_eq!(vm.stats().live, 0);
            assert!(output.borrow().is_empty());
            return;
        }
    }
    panic!("cleared timers prevented program completion");
}
