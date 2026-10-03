#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;
use std::{
    cell::{Cell, RefCell},
    future::Future,
    pin::Pin,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
};

#[derive(Default)]
struct Gate {
    waker: RefCell<Option<Waker>>,
    value: RefCell<Option<Result<HostValue>>>,
    drops: Cell<usize>,
}
impl Gate {
    fn release(&self, value: Result<HostValue>) {
        *self.value.borrow_mut() = Some(value);
        self.waker
            .borrow_mut()
            .take()
            .expect("gate was polled")
            .wake();
    }
}
struct Wait(Rc<Gate>);
impl Future for Wait {
    type Output = Result<HostValue>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(value) = self.0.value.borrow_mut().take() {
            Poll::Ready(value)
        } else {
            *self.0.waker.borrow_mut() = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}
impl Drop for Wait {
    fn drop(&mut self) {
        self.0.drops.set(self.0.drops.get() + 1);
    }
}
#[derive(Default)]
struct Wakes(AtomicUsize);
impl Wake for Wakes {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
fn hosts(gates: &[(&str, Rc<Gate>)]) -> (Hosts, Rc<RefCell<Vec<String>>>) {
    let mut hosts = Hosts::default();
    for (name, gate) in gates {
        let gate = gate.clone();
        hosts
            .register(HostOp::new(name, vec![], HostType::Unit, true, move |_| {
                HostReply::Pending(Box::pin(Wait(gate.clone())))
            }))
            .unwrap();
    }
    let output = Rc::new(RefCell::new(vec![]));
    let capture = output.clone();
    hosts
        .register(HostOp::new(
            "mark",
            vec![HostType::String],
            HostType::Unit,
            false,
            move |args| {
                let HostValue::String(value) = &args[0] else {
                    unreachable!()
                };
                capture.borrow_mut().push(value.clone());
                HostReply::Ready(Ok(HostValue::Unit))
            },
        ))
        .unwrap();
    (hosts, output)
}
fn vm(source: &str, hosts: Hosts) -> Vm {
    Vm::new(compiler::compile(source, &hosts).unwrap(), hosts).unwrap()
}
fn reach_gate(vm: &mut Vm, gate: &Gate, cx: &mut Context<'_>) {
    for _ in 0..64 {
        assert!(
            vm.poll(cx).is_pending(),
            "main exited before pending work completed"
        );
        if gate.waker.borrow().is_some() {
            return;
        }
    }
    panic!("pending operation was never polled");
}
fn complete(vm: &mut Vm, cx: &mut Context<'_>) -> Result<HostValue> {
    for _ in 0..64 {
        if let Poll::Ready(result) = vm.poll(cx) {
            return result;
        }
    }
    panic!("ready tasks did not finish");
}
const WORKER: &str = r#"
import { gate, mark } from "vm:host";
async fn worker() Promise<void> {
    const captured = {message: "alive"};
    await gate();
    mark(captured.message);
}
fn main() number { const pending = worker(); return 7; }
"#;
#[test]
fn main_result_waits_for_host_work_and_keeps_suspended_frames_rooted() {
    let gate = Rc::new(Gate::default());
    let (hosts, output) = hosts(&[("gate", gate.clone())]);
    let mut vm = vm(WORKER, hosts);
    let wakes = Arc::new(Wakes::default());
    let waker = Waker::from(wakes.clone());
    let mut cx = Context::from_waker(&waker);
    reach_gate(&mut vm, &gate, &mut cx);
    assert_eq!(gate.drops.get(), 0);
    vm.collect().unwrap();
    assert!(vm.stats().live > 0);
    // Once blocked, another idle turn must not spin or report completion.
    let before = wakes.0.load(Ordering::SeqCst);
    assert!(vm.poll(&mut cx).is_pending());
    assert_eq!(wakes.0.load(Ordering::SeqCst), before);
    gate.release(Ok(HostValue::Unit));
    assert!(wakes.0.load(Ordering::SeqCst) > before);
    assert_eq!(complete(&mut vm, &mut cx).unwrap(), HostValue::Number(7.));
    assert_eq!(&*output.borrow(), &["alive"]);
    assert_eq!(gate.drops.get(), 1);
    assert_eq!(vm.pending_tasks(), 0);
    assert_eq!(vm.stats().live, 0);
}
#[test]
fn tasks_spawned_during_draining_also_keep_the_program_alive() {
    let first = Rc::new(Gate::default());
    let second = Rc::new(Gate::default());
    let (hosts, output) = hosts(&[("first", first.clone()), ("second", second.clone())]);
    let mut vm = vm(
        r#"
import { first, second, mark } from "vm:host";
async fn child() Promise<void> { await second(); mark("child"); }
async fn worker() Promise<void> { await first(); const pending = child(); }
fn main() number { const pending = worker(); return 7; }
"#,
        hosts,
    );
    let mut cx = Context::from_waker(Waker::noop());
    reach_gate(&mut vm, &first, &mut cx);
    first.release(Ok(HostValue::Unit));
    reach_gate(&mut vm, &second, &mut cx);
    assert!(output.borrow().is_empty());
    second.release(Ok(HostValue::Unit));
    assert_eq!(complete(&mut vm, &mut cx).unwrap(), HostValue::Number(7.));
    assert_eq!(&*output.borrow(), &["child"]);
    assert_eq!(first.drops.get(), 1);
    assert_eq!(second.drops.get(), 1);
    assert_eq!(vm.stats().live, 0);
}
#[test]
fn explicit_cancellation_after_main_returns_drops_pending_work() {
    let gate = Rc::new(Gate::default());
    let (hosts, output) = hosts(&[("gate", gate.clone())]);
    let mut vm = vm(WORKER, hosts);
    let mut cx = Context::from_waker(Waker::noop());
    reach_gate(&mut vm, &gate, &mut cx);
    vm.cancel().unwrap();
    assert_eq!(gate.drops.get(), 1);
    assert!(output.borrow().is_empty());
    assert_eq!(vm.pending_tasks(), 0);
    assert_eq!(vm.stats().live, 0);
    assert_eq!(complete(&mut vm, &mut cx).unwrap_err(), "VM cancelled");
}
#[test]
fn a_background_host_wire_fault_cannot_be_hidden_by_successful_main() {
    let gate = Rc::new(Gate::default());
    let (hosts, output) = hosts(&[("gate", gate.clone())]);
    let mut vm = vm(WORKER, hosts);
    let mut cx = Context::from_waker(Waker::noop());
    reach_gate(&mut vm, &gate, &mut cx);
    gate.release(Ok(HostValue::String("wrong type".into())));
    assert_eq!(
        complete(&mut vm, &mut cx).unwrap_err(),
        "host returned the wrong result type"
    );
    assert!(output.borrow().is_empty());
    assert_eq!(gate.drops.get(), 1);
    assert_eq!(vm.stats().live, 0);
}
