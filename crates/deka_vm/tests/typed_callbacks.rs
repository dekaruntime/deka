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
fn callback_type(result_channel: bool) -> HostType {
    HostType::TypedCallback {
        args: vec![HostType::Bytes, HostType::Number],
        result: Box::new(HostType::String),
        result_channel,
    }
}
fn hosts(result_channel: bool) -> Hosts {
    let mut hosts = Hosts::default();
    hosts
        .register(
            HostOp::new(
                "invoke",
                vec![callback_type(result_channel)],
                HostType::String,
                true,
                |args| {
                    let HostValue::Callback(callback) = &args[0] else {
                        unreachable!()
                    };
                    HostReply::Pending(
                        callback
                            .call_async(vec![HostValue::Bytes(vec![65, 66]), HostValue::Number(7.)])
                            .unwrap(),
                    )
                },
            )
            .with_result_channel(),
        )
        .unwrap();
    text_codec::register(&mut hosts).unwrap();
    hosts
}
async fn execute(source: &str, hosts: Hosts) -> Result<HostValue> {
    let program = compiler::compile(source, &hosts)?;
    // Compiled payloads retain the same catalog contract without source files.
    let program = serde_json::from_slice(&serde_json::to_vec(&program).unwrap()).unwrap();
    let mut vm = Vm::new(program, hosts)?;
    let result = vm.run().await;
    assert_eq!(vm.stats().live, 0);
    result
}
#[tokio::test]
async fn sync_and_async_callbacks_receive_bytes_and_return_owned_values() {
    for asynchronous in [false, true] {
        let prefix = if asynchronous { "async " } else { "" };
        let output = if asynchronous {
            "Promise<string>"
        } else {
            "string"
        };
        let source = format!(
            r#"
            import {{ invoke }} from "vm:host";
            {prefix}fn chunk(data: bytes, count: number) {output} {{
                return string(count) + ":" + string(data.length);
            }}
            async fn main() Promise<string> {{
                const response = await invoke(chunk);
                return match(response) {{ Ok(v) => v, Err(e) => e }};
            }}
        "#
        );
        assert_eq!(
            execute(&source, hosts(false)).await.unwrap(),
            HostValue::String("7:2".into())
        );
    }
}
#[tokio::test]
async fn callback_result_uses_nominal_enum_path_and_failure_does_not_stop_other_work() {
    for (case, expected) in [
        ("Ok(\"done\")", "done"),
        ("Err(\"sink failed\")", "sink failed"),
    ] {
        let source = format!(
            r#"
            import {{ invoke }} from "vm:host";
            async fn chunk(data: bytes, count: number) Promise<Result<string, string>> {{ return {case}; }}
            async fn main() Promise<string> {{
                const response = await invoke(chunk);
                return match(response) {{ Ok(v) => v, Err(e) => e }};
            }}
        "#
        );
        assert_eq!(
            execute(&source, hosts(true)).await.unwrap(),
            HostValue::String(expected.into())
        );
    }
}
#[test]
fn checker_rejects_wrong_chunk_and_return_types() {
    for callback in [
        "fn(data: string, count: number) string { return data; }",
        "fn(data: bytes) string { return \"x\"; }",
        "fn(data: bytes, count: number) number { return count; }",
    ] {
        let source = format!(
            "import {{invoke}} from \"vm:host\"; async fn main() {{ await invoke({callback}); }}"
        );
        assert!(
            compiler::compile(&source, &hosts(false)).is_err(),
            "accepted {callback}"
        );
    }
}
#[test]
fn pending_async_callback_resolves_only_after_its_await_and_vm_cancellation_wakes_it() {
    for cancel in [false, true] {
        let held = Rc::new(RefCell::new(None));
        let capture = held.clone();
        let mut hosts = hosts(false);
        hosts
            .register(HostOp::new(
                "hold",
                vec![callback_type(false)],
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
        let (send, receive) = tokio::sync::oneshot::channel::<()>();
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
        let program = compiler::compile(r#"
            import {hold, gate} from "vm:host";
            async fn chunk(data: bytes, count: number) Promise<string> { await gate(); return "finished"; }
            fn main() { hold(chunk); }
        "#, &hosts).unwrap();
        let mut vm = Vm::new(program, hosts).unwrap();
        let wakes = Arc::new(Wakes::default());
        let waker = Waker::from(wakes.clone());
        let mut cx = Context::from_waker(&waker);
        for _ in 0..8 {
            vm.run_turn(&mut cx, 32).unwrap();
        }
        vm.collect().unwrap();
        let callback = held.borrow_mut().take().unwrap();
        assert_eq!(
            callback
                .call_async(vec![HostValue::String("wrong".into())])
                .err()
                .unwrap(),
            "invalid host callback arguments"
        );
        let mut future = callback
            .call_async(vec![HostValue::Bytes(vec![1]), HostValue::Number(1.)])
            .unwrap();
        assert!(future.as_mut().poll(&mut cx).is_pending());
        for _ in 0..8 {
            vm.run_turn(&mut cx, 32).unwrap();
        }
        assert!(
            future.as_mut().poll(&mut cx).is_pending(),
            "async return completed before its gate"
        );
        if cancel {
            let before = wakes.0.load(Ordering::SeqCst);
            vm.cancel().unwrap();
            assert!(
                wakes.0.load(Ordering::SeqCst) > before,
                "cancellation must wake the invocation waiter"
            );
            assert_eq!(
                future.as_mut().poll(&mut cx),
                Poll::Ready(Err("VM cancelled".into()))
            );
        } else {
            send.send(()).unwrap();
            for _ in 0..32 {
                vm.run_turn(&mut cx, 32).unwrap();
            }
            assert_eq!(
                future.as_mut().poll(&mut cx),
                Poll::Ready(Ok(HostValue::String("finished".into())))
            );
            vm.cancel().unwrap();
        }
        assert_eq!(vm.stats().live, 0);
    }
}

#[tokio::test]
async fn synchronous_callback_returning_a_promise_is_followed() {
    let source = r#"
        import {invoke} from "vm:host";
        async fn finish() Promise<string> { return "followed"; }
        fn chunk(data: bytes, count: number) Promise<string> { return finish(); }
        async fn main() Promise<string> {
            return match(await invoke(chunk)) { Ok(v) => v, Err(e) => e };
        }
    "#;
    assert_eq!(
        execute(source, hosts(false)).await.unwrap(),
        HostValue::String("followed".into())
    );
}

#[tokio::test]
async fn an_error_completion_does_not_cancel_another_callback() {
    let source = r#"
        import {invoke} from "vm:host";
        fn fail(data: bytes, count: number) Result<string, string> { return Err("failed"); }
        async fn succeed(data: bytes, count: number) Promise<Result<string, string>> { return Ok("kept running"); }
        async fn main() Promise<string> {
            const values = await Promise.all([invoke(fail), invoke(succeed)]);
            return (values.has(0) ? match(values[0]) { Ok(v) => v, Err(e) => e } : "missing") + ":" +
                (values.has(1) ? match(values[1]) { Ok(v) => v, Err(e) => e } : "missing");
        }
    "#;
    assert_eq!(
        execute(source, hosts(true)).await.unwrap(),
        HostValue::String("failed:kept running".into())
    );
}

#[cfg(feature = "ui")]
#[test]
fn desktop_button_can_await_a_chunk_callback_and_become_idle() {
    use deka_native_ui::{Host, Node};
    fn text(node: &Node) -> String {
        let mut value = node.text.clone().unwrap_or_default();
        for child in &node.children {
            value.push_str(&text(child));
        }
        value
    }
    let hosts = hosts(false);
    let source = r#"
        import {invoke} from "vm:host";
        export fn App() {
            let message = "idle";
            return (<view><p>{message}</p><button onClick={async fn() {
                message = "loading";
                const response = await invoke(async fn(data: bytes, count: number) Promise<string> {
                    let total = 0;
                    for (let i = 0; i < 1000; i += 1) { total += 1; }
                    return string(count) + ":" + string(data.length) + ":" + string(total);
                });
                message = match(response) { Ok(v) => v, Err(e) => e };
            }}>Run</button></view>);
        }
    "#;
    let program = compiler::compile_entry(source, &hosts, "App").unwrap();
    let mut window = Host::new(ui::VmApp::with_hosts(program, hosts).unwrap());
    assert_eq!(text(&window.render()), "idleRun");
    window.click(0);
    window.run_turn(1);
    assert_ne!(
        text(&window.render()),
        "7:2:1000Run",
        "one turn must not block the window"
    );
    for _ in 0..2000 {
        if !window.has_ready_work() {
            break;
        }
        window.run_turn(32);
    }
    assert_eq!(text(&window.render()), "7:2:1000Run");
    assert!(!window.has_ready_work());
    for _ in 0..32 {
        assert!(!window.run_turn(32));
    }
}

#[tokio::test]
async fn rust_waits_for_each_chunk_and_retains_language_captures() {
    let mut hosts = Hosts::default();
    hosts
        .register(
            HostOp::new(
                "consume",
                vec![callback_type(false)],
                HostType::String,
                true,
                |args| {
                    let HostValue::Callback(callback) = &args[0] else {
                        unreachable!()
                    };
                    let callback = callback.clone();
                    HostReply::Pending(Box::pin(async move {
                        callback
                            .call_async(vec![HostValue::Bytes(vec![65]), HostValue::Number(1.)])?
                            .await?;
                        callback
                            .call_async(vec![HostValue::Bytes(vec![66]), HostValue::Number(2.)])?
                            .await
                    }))
                },
            )
            .with_result_channel(),
        )
        .unwrap();
    let source = r#"
        import {consume} from "vm:host";
        async fn main() Promise<string> {
            let total = 0;
            const reply = await consume(async fn(data: bytes, count: number) Promise<string> {
                total += count;
                return string(total);
            });
            return match(reply) { Ok(v) => v, Err(e) => e } + ":" + string(total);
        }
    "#;
    assert_eq!(
        execute(source, hosts).await.unwrap(),
        HostValue::String("3:3".into())
    );
}

#[test]
fn cancellation_and_drop_resolve_a_callback_still_in_the_queue() {
    for cancel in [false, true] {
        let held = Rc::new(RefCell::new(None));
        let capture = held.clone();
        let mut hosts = Hosts::default();
        hosts
            .register(HostOp::new(
                "hold",
                vec![callback_type(false)],
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
        let program = compiler::compile(
            r#"
            import {hold} from "vm:host";
            fn main() { hold(fn(data: bytes, count: number) string { return "never ran"; }); }
        "#,
            &hosts,
        )
        .unwrap();
        let mut vm = Vm::new(program, hosts).unwrap();
        let wakes = Arc::new(Wakes::default());
        let waker = Waker::from(wakes.clone());
        let mut cx = Context::from_waker(&waker);
        for _ in 0..8 {
            vm.run_turn(&mut cx, 32).unwrap();
        }
        let callback = held.borrow_mut().take().unwrap();
        let mut future = callback
            .call_async(vec![HostValue::Bytes(vec![]), HostValue::Number(0.)])
            .unwrap();
        assert!(future.as_mut().poll(&mut cx).is_pending());
        let before = wakes.0.load(Ordering::SeqCst);
        if cancel {
            vm.cancel().unwrap();
            assert_eq!(vm.stats().live, 0);
        } else {
            drop(vm);
        }
        assert!(wakes.0.load(Ordering::SeqCst) > before);
        assert_eq!(
            future.as_mut().poll(&mut cx),
            Poll::Ready(Err("VM cancelled".into()))
        );
    }
}

#[tokio::test]
async fn a_vm_fault_in_an_awaited_callback_still_fails_the_program() {
    let source = r#"
        import {invoke} from "vm:host";
        fn fail(data: bytes, count: number) string { panic("callback fault"); return "unreachable"; }
        async fn main() Promise<string> {
            return match(await invoke(fail)) { Ok(v) => v, Err(e) => "hidden:" + e };
        }
    "#;
    assert_eq!(
        execute(source, hosts(false)).await.unwrap_err(),
        "callback fault"
    );
}

#[tokio::test]
async fn callback_receives_a_rust_owned_controller_with_typed_methods() {
    let chunks = Rc::new(RefCell::new(Vec::<Vec<u8>>::new()));
    let received = chunks.clone();
    let mut hosts = Hosts::default();
    hosts
        .register(
            HostOp::new(
                "enqueueChunk",
                vec![HostType::Handle("ChunkController".into()), HostType::Bytes],
                HostType::Unit,
                false,
                move |args| {
                    let [HostValue::Handle(controller), HostValue::Bytes(chunk)] = args.as_slice()
                    else {
                        unreachable!()
                    };
                    assert_eq!(controller.downcast_ref::<u32>(), Some(&7));
                    received.borrow_mut().push(chunk.clone());
                    HostReply::Ready(Ok(HostValue::Unit))
                },
            )
            .with_receiver_method("ChunkController", "enqueue"),
        )
        .unwrap();
    hosts
        .register(
            HostOp::new(
                "invokeController",
                vec![HostType::TypedCallback {
                    args: vec![HostType::Handle("ChunkController".into()), HostType::Bytes],
                    result: Box::new(HostType::Unit),
                    result_channel: false,
                }],
                HostType::Unit,
                true,
                |args| {
                    let HostValue::Callback(callback) = &args[0] else {
                        unreachable!()
                    };
                    HostReply::Pending(
                        callback
                            .call_async(vec![
                                HostValue::Handle(HostHandle::new("ChunkController", 7u32)),
                                HostValue::Bytes(vec![1, 2, 3]),
                            ])
                            .unwrap(),
                    )
                },
            )
            .with_result_channel(),
        )
        .unwrap();
    let source = r#"
        import {invokeController, type ChunkController} from "vm:host";
        async fn consume(controller: ChunkController, chunk: bytes) Promise<void> { controller.enqueue(chunk); }
        async fn main() Promise<number> {
            const response = await invokeController(consume);
            return match(response) { Ok(v) => 1, Err(e) => 0 };
        }
    "#;
    assert_eq!(execute(source, hosts).await.unwrap(), HostValue::Number(1.));
    assert_eq!(&*chunks.borrow(), &[vec![1, 2, 3]]);
}

#[tokio::test]
async fn callback_wire_contract_fault_cannot_be_hidden_as_an_operational_err() {
    let hosts = hosts(false);
    let source = r#"
        import {invoke} from "vm:host";
        fn chunk(data: bytes, count: number) string { return "corrupt me"; }
        async fn main() Promise<string> {
            return match(await invoke(chunk)) { Ok(v) => v, Err(e) => "hidden:" + e };
        }
    "#;
    let mut program = compiler::compile(source, &hosts).unwrap();
    let mut changed = false;
    for function in &mut program.functions {
        for op in &mut function.code {
            if matches!(op, Op::Const(Literal::String(value)) if value == "corrupt me") {
                *op = Op::Const(Literal::Bool(false));
                changed = true;
            }
        }
    }
    assert!(changed);
    let mut vm = Vm::new(program, hosts).unwrap();
    assert_eq!(
        vm.run().await.unwrap_err(),
        "callback returned the wrong result type"
    );
    assert_eq!(vm.stats().live, 0);
}
