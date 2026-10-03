#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;
use std::{
    cell::RefCell,
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
type HeldCallback = Rc<RefCell<Option<HostCallback>>>;
fn setup() -> (
    Vm,
    HeldCallback,
    tokio::sync::oneshot::Sender<()>,
    demo::Output,
) {
    let (mut hosts, output) = demo::hosts().unwrap();
    let held = Rc::new(RefCell::new(None));
    let capture = held.clone();
    hosts
        .register(HostOp::new(
            "hold",
            vec![HostType::Callback],
            HostType::Unit,
            false,
            move |args| {
                let HostValue::Callback(callback) = &args[0] else {
                    unreachable!()
                };
                *capture.borrow_mut() = Some(callback.clone());
                HostReply::Ready(Ok(HostValue::Unit))
            },
        ))
        .unwrap();
    let (send, receive) = tokio::sync::oneshot::channel();
    let receive = Rc::new(RefCell::new(Some(receive)));
    hosts
        .register(HostOp::new(
            "gate",
            vec![],
            HostType::Unit,
            true,
            move |_| {
                let receive = receive.borrow_mut().take().unwrap();
                HostReply::Pending(Box::pin(async move {
                    receive.await.map_err(|e| e.to_string())?;
                    Ok(HostValue::Unit)
                }))
            },
        ))
        .unwrap();
    let program = compiler::compile(
        r#"
import { hold, gate, print } from "vm:host";
fn retain() {
    const captured = {message: "rooted"};
    hold(fn() { print(captured.message); });
}
async fn main() Promise<number> { retain(); await gate(); return 7; }
"#,
        &hosts,
    )
    .unwrap();
    (Vm::new(program, hosts).unwrap(), held, send, output)
}
fn block(vm: &mut Vm, cx: &mut Context<'_>) {
    for _ in 0..8 {
        assert!(vm.poll(cx).is_pending());
    }
}
#[test]
fn retained_callback_roots_its_closure_and_enqueues_without_reentry() {
    let (mut vm, held, send, output) = setup();
    let wakes = Arc::new(Wakes::default());
    let waker = Waker::from(wakes.clone());
    let mut cx = Context::from_waker(&waker);
    block(&mut vm, &mut cx);
    vm.collect().unwrap();
    let before = wakes.0.load(Ordering::SeqCst);
    held.borrow().as_ref().unwrap().call(vec![]).unwrap();
    assert!(
        output.borrow().is_empty(),
        "host call must not re-enter language execution"
    );
    assert!(wakes.0.load(Ordering::SeqCst) > before);
    block(&mut vm, &mut cx);
    assert_eq!(&*output.borrow(), &["rooted"]);
    held.borrow_mut().take();
    vm.collect().unwrap();
    send.send(()).unwrap();
    for _ in 0..64 {
        if let Poll::Ready(result) = vm.poll(&mut cx) {
            assert_eq!(result.unwrap(), HostValue::Number(7.));
            assert_eq!(vm.stats().live, 0);
            return;
        }
    }
    panic!("released task did not complete");
}
#[test]
fn a_callback_cannot_enqueue_after_cancellation_or_vm_drop() {
    for cancel in [false, true] {
        let (mut vm, held, _send, _output) = setup();
        let mut cx = Context::from_waker(Waker::noop());
        block(&mut vm, &mut cx);
        let callback = held.borrow_mut().take().unwrap();
        if cancel {
            vm.cancel().unwrap();
            assert_eq!(vm.stats().live, 0);
        }
        drop(vm);
        assert_eq!(callback.call(vec![]).unwrap_err(), "VM cancelled");
    }
}
#[test]
fn cancelling_one_vm_does_not_dispose_another_vms_callback() {
    let (mut first, first_callback, _first_send, _) = setup();
    let (mut second, second_callback, send, output) = setup();
    let mut cx = Context::from_waker(Waker::noop());
    block(&mut first, &mut cx);
    block(&mut second, &mut cx);
    first.cancel().unwrap();
    assert!(
        first_callback
            .borrow()
            .as_ref()
            .unwrap()
            .call(vec![])
            .is_err()
    );
    second_callback
        .borrow()
        .as_ref()
        .unwrap()
        .call(vec![])
        .unwrap();
    block(&mut second, &mut cx);
    assert_eq!(&*output.borrow(), &["rooted"]);
    send.send(()).unwrap();
    for _ in 0..64 {
        if let Poll::Ready(result) = second.poll(&mut cx) {
            assert_eq!(result.unwrap(), HostValue::Number(7.));
            return;
        }
    }
    panic!("second VM did not complete");
}

#[test]
fn host_values_cannot_return_a_callback_owned_by_another_vm() {
    let (mut first, held, _send, _) = setup();
    let mut cx = Context::from_waker(Waker::noop());
    block(&mut first, &mut cx);
    let mut hosts = Hosts::default();
    hosts
        .register(HostOp::new(
            "foreign",
            vec![],
            HostType::Callback,
            false,
            move |_| {
                HostReply::Ready(Ok(HostValue::Callback(
                    held.borrow().as_ref().unwrap().clone(),
                )))
            },
        ))
        .unwrap();
    let program = compiler::compile(
        "import { foreign } from \"vm:host\"; fn main() { const callback = foreign(); }",
        &hosts,
    )
    .unwrap();
    let mut second = Vm::new(program, hosts).unwrap();
    assert_eq!(
        second.poll(&mut cx),
        Poll::Ready(Err("callback belongs to another VM".into()))
    );
    assert_eq!(second.stats().live, 0);
    first.cancel().unwrap();
}

#[test]
fn host_jobs_cannot_be_scheduled_into_a_different_context() {
    let first = HostContext::default();
    let second = HostContext::default();
    let job = first.job().unwrap();
    assert_eq!(
        second
            .spawn(job, Box::pin(async { Ok(HostValue::Unit) }))
            .unwrap_err(),
        "host job belongs to another VM"
    );
}
