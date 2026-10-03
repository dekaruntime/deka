#![cfg(all(feature = "compiler", feature = "host", feature = "ui"))]
use deka_native_ui::{Host, Node};
use deka_vm::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
};
#[derive(Default)]
struct Wakes(AtomicUsize);
impl Wake for Wakes {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
fn text(node: &Node) -> String {
    let mut value = node.text.clone().unwrap_or_default();
    for child in &node.children {
        value.push_str(&text(child));
    }
    value
}
fn settle(host: &mut Host<ui::VmApp>, budget: usize) {
    for _ in 0..2000 {
        if !host.has_ready_work() {
            return;
        }
        host.run_turn(budget);
    }
    panic!("VM did not become idle");
}
fn gated(
    source: &str,
) -> (
    Host<ui::VmApp>,
    tokio::sync::oneshot::Sender<String>,
    Arc<Wakes>,
) {
    let (send, receive) = tokio::sync::oneshot::channel();
    let receive = Rc::new(RefCell::new(Some(receive)));
    let mut hosts = Hosts::default();
    hosts
        .register(HostOp::new(
            "gate",
            vec![],
            HostType::String,
            true,
            move |_| {
                let receive = receive.borrow_mut().take().unwrap();
                HostReply::Pending(Box::pin(async move {
                    receive
                        .await
                        .map(HostValue::String)
                        .map_err(|e| e.to_string())
                }))
            },
        ))
        .unwrap();
    let program = compiler::compile_entry(source, &hosts, "App").unwrap();
    let mut host = Host::new(ui::VmApp::with_hosts(program, hosts).unwrap());
    let wakes = Arc::new(Wakes::default());
    let waker = Waker::from(wakes.clone());
    host.set_waker(deka_native_ui::Waker::new(move || waker.wake_by_ref()));
    (host, send, wakes)
}
#[test]
fn async_on_click_suspends_and_updates_the_view_after_a_real_wake() {
    let (mut host, send, wakes) = gated(
        r#"import {gate} from "vm:host";
export fn App() { let message = "idle"; return <view><p>{message}</p><button onClick={async fn() { message = "loading"; message = await gate(); }}>Load</button></view>; }"#,
    );
    assert_eq!(text(&host.render()), "idleLoad");
    host.click(0);
    settle(&mut host, 32);
    assert_eq!(text(&host.render()), "loadingLoad");
    assert!(!host.has_ready_work());
    let before = wakes.0.load(Ordering::SeqCst);
    assert!(!host.run_turn(32));
    assert_eq!(
        wakes.0.load(Ordering::SeqCst),
        before,
        "idle work must not reschedule itself"
    );
    send.send("done".into()).unwrap();
    assert!(host.has_ready_work());
    assert!(wakes.0.load(Ordering::SeqCst) > before);
    settle(&mut host, 32);
    assert_eq!(text(&host.render()), "doneLoad");
    assert!(!host.has_ready_work());
}
#[test]
fn a_sync_handler_can_launch_work_that_survives_its_return() {
    let (mut host, send, _) = gated(
        r#"import {gate} from "vm:host";
export fn App() { let message="idle"; const load = async fn() { message=await gate(); };
return <view><p>{message}</p><button onClick={fn() { load(); }}>Load</button></view>; }"#,
    );
    host.click(0);
    settle(&mut host, 16);
    assert_eq!(text(&host.render()), "idleLoad");
    send.send("background".into()).unwrap();
    settle(&mut host, 16);
    assert_eq!(text(&host.render()), "backgroundLoad");
}
#[test]
fn background_work_started_during_initialization_survives_rendering() {
    let (mut host, send, _) = gated(
        r#"import {gate} from "vm:host";
export fn App() { let message="starting"; const load = async fn() { message=await gate(); }; load(); return <view><p>{message}</p></view>; }"#,
    );
    settle(&mut host, 16);
    for _ in 0..20 {
        assert_eq!(text(&host.render()), "starting");
    }
    send.send("initialized".into()).unwrap();
    settle(&mut host, 16);
    assert_eq!(text(&host.render()), "initialized");
}
#[test]
fn a_busy_handler_yields_between_turns_and_then_finishes() {
    let hosts = Hosts::default();
    let program=compiler::compile_entry(r#"export fn App() { let count=0; return <view><p>{count}</p><button onClick={fn() { for (let i=0; i < 10000; i+=1) { count += 1; } }}>Run</button></view>; }"#,&hosts,"App").unwrap();
    let mut host = Host::new(ui::VmApp::with_hosts(program, hosts).unwrap());
    host.click(0);
    assert!(host.has_ready_work());
    assert_ne!(
        text(&host.render()),
        "10000Run",
        "one event must not run the handler to completion"
    );
    settle(&mut host, 128);
    assert_eq!(text(&host.render()), "10000Run");
}
struct HeldResource(Rc<Cell<usize>>);
impl Drop for HeldResource {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}
#[test]
fn dropping_the_window_releases_the_pending_host_future() {
    let drops = Rc::new(Cell::new(0));
    let counter = drops.clone();
    let mut hosts = Hosts::default();
    hosts
        .register(HostOp::new(
            "wait",
            vec![],
            HostType::Unit,
            true,
            move |_| {
                let resource = HeldResource(counter.clone());
                HostReply::Pending(Box::pin(async move {
                    std::future::pending::<()>().await;
                    drop(resource);
                    Ok(HostValue::Unit)
                }))
            },
        ))
        .unwrap();
    let program=compiler::compile_entry(r#"import {wait} from "vm:host"; export fn App() { return <button onClick={async fn() { await wait(); }}>Wait</button>; }"#,&hosts,"App").unwrap();
    let mut host = Host::new(ui::VmApp::with_hosts(program, hosts).unwrap());
    host.click(0);
    settle(&mut host, 16);
    assert_eq!(drops.get(), 0);
    drop(host);
    assert_eq!(drops.get(), 1);
}
#[test]
fn one_instruction_turns_are_fair_and_register_the_pending_producers_wake() {
    let (mut hosts, output) = demo::hosts().unwrap();
    let (send, receive) = tokio::sync::oneshot::channel();
    let receive = Rc::new(RefCell::new(Some(receive)));
    hosts
        .register(HostOp::new(
            "gate",
            vec![],
            HostType::String,
            true,
            move |_| {
                let receive = receive.borrow_mut().take().unwrap();
                HostReply::Pending(Box::pin(async move {
                    receive
                        .await
                        .map(HostValue::String)
                        .map_err(|e| e.to_string())
                }))
            },
        ))
        .unwrap();
    let program = compiler::compile(
        r#"import {gate, print} from "vm:host";
async fn main() { const read = async fn() { print(await gate()); }; read(); for (let i=0; true; i+=1) { } }"#,
        &hosts,
    )
    .unwrap();
    let mut vm = Vm::new(program, hosts).unwrap();
    let wakes = Arc::new(Wakes::default());
    let waker = Waker::from(wakes);
    let mut cx = Context::from_waker(&waker);
    for _ in 0..200 {
        assert!(vm.run_turn(&mut cx, 1).unwrap().instructions <= 1);
    }
    send.send("fair".into()).unwrap();
    for _ in 0..200 {
        assert!(vm.run_turn(&mut cx, 1).unwrap().instructions <= 1);
        if !output.borrow().is_empty() {
            break;
        }
    }
    assert_eq!(&*output.borrow(), &["fair"]);
    vm.cancel().unwrap();
    assert_eq!(vm.stats().live, 0);
}
#[test]
fn zero_budget_does_not_lose_the_ready_work() {
    let hosts = Hosts::default();
    let program = compiler::compile("fn main() {}", &hosts).unwrap();
    let mut vm = Vm::new(program, hosts).unwrap();
    assert!(vm.has_ready_work());
    assert_eq!(
        vm.run_turn(&mut Context::from_waker(Waker::noop()), 0)
            .unwrap_err(),
        "turn budget must be positive"
    );
    assert!(vm.has_ready_work());
    for _ in 0..100 {
        let turn = vm
            .run_turn(&mut Context::from_waker(Waker::noop()), 1)
            .unwrap();
        assert!(turn.instructions <= 1);
        if vm.pending_tasks() == 0 {
            break;
        }
    }
    assert_eq!(
        vm.poll(&mut Context::from_waker(Waker::noop())),
        Poll::Ready(Ok(HostValue::Unit))
    );
}

#[test]
fn uncaught_throw_in_a_handler_is_reported_before_or_after_await() {
    for asynchronous in [false, true] {
        let (mut host, send, _) = gated(if asynchronous {
            r#"import {gate} from "vm:host"; export fn App() { return <button onClick={async fn() Promise<Exception<void, string>> { await gate(); Throw("handler failed"); }}>Go</button>; }"#
        } else {
            r#"export fn App() { return <button onClick={fn() Exception<void, string> { Throw("handler failed"); }}>Go</button>; }"#
        });
        host.click(0);
        if asynchronous {
            settle(&mut host, 16);
            send.send("complete".into()).unwrap();
            settle(&mut host, 16);
        }
        assert!(
            text(&host.render()).contains("uncaught Throw: handler failed"),
            "{}",
            text(&host.render())
        );
        assert!(!host.has_ready_work());
    }
}

#[tokio::test(start_paused = true)]
async fn desktop_sleep_updates_only_at_the_vms_clock_deadline() {
    let mut hosts = Hosts::default();
    deka_vm::time::register(&mut hosts).unwrap();
    let program = compiler::compile_entry(r#"import {sleep} from "time";
export fn App() { let message="ready"; return <view><p>{message}</p><button onClick={async fn() { message="waiting"; await sleep(1000); message="done"; }}>Start</button></view>; }"#, &hosts, "App").unwrap();
    let mut host = Host::new(ui::VmApp::with_hosts(program, hosts).unwrap());
    host.click(0);
    settle(&mut host, 16);
    assert_eq!(text(&host.render()), "waitingStart");
    tokio::time::advance(std::time::Duration::from_millis(999)).await;
    settle(&mut host, 16);
    assert_eq!(text(&host.render()), "waitingStart");
    tokio::time::advance(std::time::Duration::from_millis(1)).await;
    settle(&mut host, 16);
    assert_eq!(text(&host.render()), "doneStart");
}

#[test]
fn a_small_turn_budget_becomes_idle_when_all_tasks_are_suspended() {
    let (mut host, send, _) = gated(
        r#"import {gate} from "vm:host";
export fn App() { let message="idle"; return <view><p>{message}</p><button onClick={async fn() { message=await gate(); }}>Go</button></view>; }"#,
    );
    host.click(0);
    settle(&mut host, 1);
    assert!(!host.has_ready_work());
    send.send("awake".into()).unwrap();
    settle(&mut host, 1);
    assert_eq!(text(&host.render()), "awakeGo");
}

#[test]
fn vm_progress_without_a_view_change_never_requests_a_redraw() {
    let hosts = Hosts::default();
    let program = compiler::compile_entry(r#"export fn App() { let count=0; return <button onClick={fn() { for(let i=0; i<10000; i+=1) { count+=1; } }}>Work</button>; }"#, &hosts, "App").unwrap();
    let mut host = Host::new(ui::VmApp::with_hosts(program, hosts).unwrap());
    host.click(0);
    let before = host.app.instructions();
    let mut turns = 0;
    while host.has_ready_work() {
        assert!(!host.run_turn(128), "only a changed view requests a redraw");
        turns += 1;
        assert!(turns < 2000);
    }
    assert!(turns > 1);
    assert!(host.app.instructions() > before);
    assert_eq!(text(&host.render()), "Work");
    let before = host.app.instructions();
    for _ in 0..64 {
        assert!(!host.run_turn(128));
    }
    assert_eq!(host.app.instructions(), before, "idle turns do no VM work");
}
