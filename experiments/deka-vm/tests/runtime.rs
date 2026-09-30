#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm_experiment::*;
use std::{
    cell::Cell,
    rc::Rc,
    task::{Context, Poll, Waker},
    time::Duration,
};
fn waker() -> Waker {
    Waker::noop().clone()
}
fn vm(source: &str, hosts: Hosts) -> Vm {
    Vm::new(compiler::compile(source, &hosts).unwrap(), hosts).unwrap()
}
#[tokio::test]
async fn source_calls_rust_and_closure_survives_return_and_await() {
    let (mut hosts, output) = demo::hosts().unwrap();
    hosts.grant("timer");
    let mut vm = vm(include_str!("../examples/host.ds"), hosts);
    let wake = waker();
    let mut cx = Context::from_waker(&wake);
    assert!(vm.poll(&mut cx).is_pending());
    assert!(vm.poll(&mut cx).is_pending());
    assert_eq!(
        &*output.borrow(),
        &["DekaScript continues while the Rust operation is pending"]
    );
    vm.collect().unwrap();
    assert_eq!(vm.run().await.unwrap(), HostValue::Number(83.));
    assert_eq!(output.borrow()[1], "Rust timer completed");
    assert_eq!(vm.stats().live, 0);
    assert_eq!(vm.pending_tasks(), 0);
}
#[tokio::test]
async fn awaiting_one_task_does_not_block_another() {
    let (mut hosts, output) = demo::hosts().unwrap();
    let (send, receive) = tokio::sync::oneshot::channel::<String>();
    let receiver = Rc::new(std::cell::RefCell::new(Some(receive)));
    hosts
        .register(HostOp::new(
            "gate",
            vec![],
            HostType::String,
            true,
            None,
            move |_| {
                let receive = receiver.borrow_mut().take().unwrap();
                HostReply::Pending(Box::pin(async move {
                    Ok(HostValue::String(receive.await.map_err(|e| e.to_string())?))
                }))
            },
        ))
        .unwrap();
    let mut vm = vm(
        r#"
        import {gate,print} from "vm:host";
        async fn slow() Promise<string> {const value=await gate();print(value);return value;}
        async fn fast() Promise<string> {print("fast");return "fast";}
        async fn main() Promise<string> {const a=slow();const b=fast();await a;return await b;}
    "#,
        hosts,
    );
    let wake = waker();
    let mut cx = Context::from_waker(&wake);
    for _ in 0..8 {
        assert!(vm.poll(&mut cx).is_pending());
    }
    assert_eq!(&*output.borrow(), &["fast"]);
    send.send("slow".into()).unwrap();
    assert_eq!(vm.run().await.unwrap(), HostValue::String("fast".into()));
    assert_eq!(&*output.borrow(), &["fast", "slow"]);
}
#[tokio::test]
async fn allocation_plateaus_and_completion_releases_roots() {
    let mut vm = vm(include_str!("../examples/allocation.ds"), Hosts::default());
    let wake = waker();
    let mut cx = Context::from_waker(&wake);
    let mut peak = 0;
    let mut polls = 0;
    let result = loop {
        let state = vm.poll(&mut cx);
        peak = peak.max(vm.stats().slots);
        polls += 1;
        if let Poll::Ready(result) = state {
            break result.unwrap();
        }
    };
    assert_eq!(result, HostValue::Number(4_999_950_000.));
    assert!(vm.stats().allocations > 500_000, "{:?}", vm.stats());
    assert!(peak < 600, "peak slots: {peak}");
    assert!(polls > 100);
    assert_eq!(vm.stats().live, 0);
}
#[tokio::test]
async fn scopes_do_not_leak_shadow_bindings() {
    let mut vm = vm(
        "fn main() number {const n=7; {const n=12;} return n;}",
        Hosts::default(),
    );
    assert_eq!(vm.run().await.unwrap(), HostValue::Number(7.));
}
#[tokio::test]
async fn recursive_frames_and_captured_mutation() {
    let mut vm = vm(
        "fn fib(n:number) number {if(n<2){return n;}return fib(n-1)+fib(n-2);} fn main() number {return fib(12);}",
        Hosts::default(),
    );
    assert_eq!(vm.run().await.unwrap(), HostValue::Number(144.));
}
#[test]
fn invalid_or_unsupported_source_is_rejected() {
    let (hosts, _) = demo::hosts().unwrap();
    for source in [
        "fn main() number {const x:number=\"wrong\";return x;}",
        "fn main() number {const x=1;x=2;return x;}",
        "import {sum} from \"vm:host\"; fn main() number {return sum(\"wrong\",1);}",
        "fn main() number {return missing();}",
        "fn main() number {return 5 % 2;}",
    ] {
        assert!(
            compiler::compile(source, &hosts).is_err(),
            "accepted {source}"
        );
    }
}
#[tokio::test]
async fn denied_capability_never_invokes_host() {
    let (hosts, _) = demo::hosts().unwrap();
    let mut vm = vm(
        "import {delay} from \"vm:host\"; async fn main() Promise<string> {return await delay(1,\"bad\");}",
        hosts,
    );
    assert!(
        vm.run()
            .await
            .unwrap_err()
            .contains("permission denied: timer")
    );
    assert_eq!(vm.stats().live, 0);
    assert_eq!(vm.pending_tasks(), 0);
}
struct Pending {
    dropped: Rc<Cell<usize>>,
}
impl std::future::Future for Pending {
    type Output = Result<HostValue>;
    fn poll(self: std::pin::Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Pending
    }
}
impl Drop for Pending {
    fn drop(&mut self) {
        self.dropped.set(self.dropped.get() + 1);
    }
}
#[test]
fn cancellation_drops_pending_future_and_all_vm_roots() {
    let dropped = Rc::new(Cell::new(0));
    let capture = dropped.clone();
    let mut hosts = Hosts::default();
    hosts
        .register(HostOp::new(
            "wait",
            vec![HostType::String],
            HostType::String,
            true,
            None,
            move |_| {
                HostReply::Pending(Box::pin(Pending {
                    dropped: capture.clone(),
                }))
            },
        ))
        .unwrap();
    let mut vm = vm(
        "import {wait} from \"vm:host\";async fn main() Promise<string> {const data={value: \"alive\"};return await wait(data.value);}",
        hosts,
    );
    let wake = waker();
    let mut cx = Context::from_waker(&wake);
    assert!(vm.poll(&mut cx).is_pending());
    assert!(vm.poll(&mut cx).is_pending());
    assert_eq!(dropped.get(), 0);
    vm.collect().unwrap();
    assert!(vm.stats().live > 0);
    vm.cancel().unwrap();
    assert_eq!(dropped.get(), 1);
    assert_eq!(vm.stats().live, 0);
    assert!(matches!(vm.poll(&mut cx), Poll::Ready(Err(_))));
}
#[tokio::test]
async fn asynchronous_failure_is_reported_and_scope_is_cleaned() {
    let mut hosts = Hosts::default();
    hosts
        .register(HostOp::new(
            "fail",
            vec![],
            HostType::String,
            true,
            None,
            |_| {
                HostReply::Pending(Box::pin(async {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                    Err("host failure".into())
                }))
            },
        ))
        .unwrap();
    let mut vm = vm(
        "import {fail} from \"vm:host\";async fn main() Promise<string> {return await fail();}",
        hosts,
    );
    assert_eq!(vm.run().await.unwrap_err(), "host failure");
    assert_eq!(vm.stats().live, 0);
    assert_eq!(vm.pending_tasks(), 0);
}
#[tokio::test]
async fn host_result_is_validated_at_runtime() {
    let mut hosts = Hosts::default();
    hosts
        .register(HostOp::new(
            "bad",
            vec![],
            HostType::Number,
            false,
            None,
            |_| HostReply::Ready(Ok(HostValue::String("bad".into()))),
        ))
        .unwrap();
    let mut vm = vm(
        "import {bad} from \"vm:host\";fn main() number {return bad();}",
        hosts,
    );
    assert_eq!(
        vm.run().await.unwrap_err(),
        "host returned the wrong result type"
    );
}
#[tokio::test]
async fn fuel_limit_stops_an_infinite_loop_and_cleans_up() {
    let mut vm = vm("fn main() number {for(;;){}return 0;}", Hosts::default());
    vm.set_instruction_limit(1000);
    assert_eq!(vm.run().await.unwrap_err(), "instruction limit exceeded");
    assert_eq!(vm.stats().live, 0);
}
#[tokio::test]
async fn bytecode_roundtrip_executes_and_malformed_operands_fail() {
    let p = compiler::compile("fn main() number {return 42;}", &Hosts::default()).unwrap();
    let bytes = serde_json::to_vec(&p).unwrap();
    let p = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        Vm::new(p, Hosts::default()).unwrap().run().await.unwrap(),
        HostValue::Number(42.)
    );
    let mut p = compiler::compile("fn main() number {return 42;}", &Hosts::default()).unwrap();
    p.functions[0].code[0] = Op::Load(9999);
    assert!(Vm::new(p, Hosts::default()).is_err());
}

#[tokio::test]
async fn module_state_is_shared_inside_one_vm_and_isolated_between_instances() {
    let source = "let total=0; fn add() {total+=1;} fn main() number {add();add();return total;}";
    let mut first = vm(source, Hosts::default());
    let mut second = vm(source, Hosts::default());
    assert_eq!(first.run().await.unwrap(), HostValue::Number(2.));
    assert_eq!(second.run().await.unwrap(), HostValue::Number(2.));
    assert_eq!(first.stats().live, 0);
}

#[tokio::test]
async fn checked_list_lookup_rejects_invalid_indices() {
    for (index, expected) in [(0., 11.), (1., 22.), (-1., 99.), (2., 99.), (0.5, 99.)] {
        let index = if index < 0. {
            format!("0 - {}", -index)
        } else {
            index.to_string()
        };
        let source = format!(
            "fn main() number {{ const values = [11,22]; const index = {index}; if (values.has(index)) {{ return values[index]; }} return 99; }}"
        );
        assert_eq!(
            vm(&source, Hosts::default()).run().await.unwrap(),
            HostValue::Number(expected)
        );
    }
}
