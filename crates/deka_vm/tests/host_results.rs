#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;

async fn execute(source: &str, hosts: Hosts) -> Result<HostValue> {
    let program = compiler::compile(source, &hosts)?;
    let program = serde_json::from_str(&serde_json::to_string(&program).unwrap()).unwrap();
    Vm::new(program, hosts)?.run().await
}
fn host(reply: impl Fn(Vec<HostValue>) -> HostReply + 'static, asynchronous: bool) -> Hosts {
    let mut hosts = Hosts::default();
    hosts
        .register(
            HostOp::new("read", vec![], HostType::String, asynchronous, None, reply)
                .with_result_channel(),
        )
        .unwrap();
    hosts
}

#[tokio::test]
async fn suspended_io_error_is_result_data_and_does_not_enter_catch() {
    let hosts = host(
        |_| {
            let mut pending = true;
            HostReply::Pending(Box::pin(std::future::poll_fn(move |cx| {
                if pending {
                    pending = false;
                    cx.waker().wake_by_ref();
                    return std::task::Poll::Pending;
                }
                std::task::Poll::Ready(Err("offline".into()))
            })))
        },
        true,
    );
    assert_eq!(
        execute(
            r#"
        import { read } from "vm:host"
        async fn main() Promise<string> {
            try {
                return match await read() { Ok(value) => value, Err(error) => "handled:" + error };
            } catch (e) { return "wrong: catch"; }
        }
    "#,
            hosts
        )
        .await
        .unwrap(),
        HostValue::String("handled:offline".into())
    );
}

#[tokio::test]
async fn match_arms_can_await_suspended_success_inside_try() {
    let hosts = host(
        |_| {
            let mut pending = true;
            HostReply::Pending(Box::pin(std::future::poll_fn(move |cx| {
                if pending {
                    pending = false;
                    cx.waker().wake_by_ref();
                    return std::task::Poll::Pending;
                }
                std::task::Poll::Ready(Ok(HostValue::String("ready".into())))
            })))
        },
        true,
    );
    assert_eq!(
        execute(
            r#"
        import { read } from "vm:host"
        async fn main() Promise<string> {
            try {
                return match Some(7) {
                    Some(n) => match await read() {
                        Ok(value) => value + string(n), Err(error) => error
                    },
                    None => "wrong: unused arm"
                };
            } catch (e) { return "wrong: catch"; }
        }
    "#,
            hosts
        )
        .await
        .unwrap(),
        HostValue::String("ready7".into())
    );
}

#[tokio::test]
async fn immediate_async_success_and_failure_use_the_nominal_constructor_path() {
    for (reply, expected) in [
        (Ok(HostValue::String("ready".into())), "Ok(\"ready\")"),
        (Err("offline".into()), "Err(\"offline\")"),
    ] {
        let hosts = host(move |_| HostReply::Ready(reply.clone()), true);
        assert_eq!(
            execute(
                r#"
            import { read } from "vm:host"
            async fn main() Promise<string> {
                const result = await read();
                return string(result);
            }
        "#,
                hosts
            )
            .await
            .unwrap(),
            HostValue::String(expected.into())
        );
    }
}

#[tokio::test]
async fn synchronous_result_outputs_are_data_too() {
    assert_eq!(
        execute(
            r#"
        import { read } from "vm:host"
        fn main() string { return match read() { Ok(v) => v, Err(e) => e }; }
    "#,
            host(|_| HostReply::Ready(Err("sync failure".into())), false)
        )
        .await
        .unwrap(),
        HostValue::String("sync failure".into())
    );
}

#[tokio::test]
async fn wrong_wire_types_and_missing_capabilities_remain_vm_faults() {
    let source = r#"
        import { read } from "vm:host"
        async fn main() Promise<string> {
            try { return match await read() { Ok(v) => v, Err(e) => e }; }
            catch (e) { return "wrong: caught"; }
        }
    "#;
    assert_eq!(
        execute(
            source,
            host(|_| HostReply::Ready(Ok(HostValue::Number(1.))), true)
        )
        .await
        .unwrap_err(),
        "host returned the wrong result type"
    );
    let mut hosts = Hosts::default();
    hosts
        .register(
            HostOp::new(
                "read",
                vec![],
                HostType::String,
                true,
                Some("network"),
                |_| panic!("denied handler must never be called"),
            )
            .with_result_channel(),
        )
        .unwrap();
    assert_eq!(
        execute(source, hosts).await.unwrap_err(),
        "permission denied: network"
    );
}

#[tokio::test]
async fn unit_payloads_remain_distinct_from_host_failures() {
    let mut hosts = Hosts::default();
    hosts
        .register(
            HostOp::new("complete", vec![], HostType::Unit, true, None, |_| {
                HostReply::Ready(Ok(HostValue::Unit))
            })
            .with_result_channel(),
        )
        .unwrap();
    assert_eq!(execute(r#"
        import { complete } from "vm:host"
        async fn main() Promise<number> { return match await complete() { Ok(_) => 7, Err(e) => 0 }; }
    "#, hosts).await.unwrap(), HostValue::Number(7.));
}

#[test]
fn registry_declarations_are_valid_and_the_checker_guards_result_payloads() {
    let mut hosts = Hosts::default();
    for (i, ty) in [
        HostType::Unit,
        HostType::Number,
        HostType::Bool,
        HostType::String,
        HostType::Strings,
    ]
    .iter()
    .enumerate()
    {
        for asynchronous in [false, true] {
            hosts
                .register(
                    HostOp::new(
                        &format!("op_{i}_{asynchronous}"),
                        vec![],
                        ty.clone(),
                        asynchronous,
                        None,
                        |_| unreachable!(),
                    )
                    .with_result_channel(),
                )
                .unwrap();
        }
    }
    let source = hosts.declarations();
    let arena = bumpalo::Bump::new();
    let parsed = deka_syntax::parse(&source, &arena);
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    parsed.program.unwrap();
    let mut names = Vec::new();
    let mut calls = String::new();
    for i in 0..5 {
        names.push(format!("op_{i}_false"));
        names.push(format!("op_{i}_true"));
        calls.push_str(&format!(
            "const a{i} = op_{i}_false(); const b{i} = await op_{i}_true();"
        ));
    }
    // Bodyless signatures belong to the host declaration module; checking that
    // file as an ordinary application would reject its ambient declarations.
    let consumer = format!(
        "import {{ {} }} from \"vm:host\"; async fn main() {{ {calls} }}",
        names.join(", ")
    );
    compiler::compile(&consumer, &hosts).unwrap();
    let hosts = host(|_| unreachable!(), true);
    let error = compiler::compile(
        r#"
        import { read } from "vm:host"
        async fn main() Promise<string> { const text: string = await read(); return text; }
    "#,
        &hosts,
    )
    .unwrap_err();
    assert!(error.contains("Result<string, string>"), "{error}");
}
