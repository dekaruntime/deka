#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;
#[path = "support/http_server.rs"]
mod server;
use server::{Server, reply};
fn hosts() -> Hosts {
    let mut hosts = Hosts::default();
    abort::register(&mut hosts).unwrap();
    http_headers::register(&mut hosts).unwrap();
    http_response::register(&mut hosts).unwrap();
    fetch::register(&mut hosts).unwrap();
    hosts
}
async fn run(source: &str, hosts: Hosts) -> HostValue {
    let program = compiler::compile(source, &hosts).unwrap();
    let program = serde_json::from_slice(&serde_json::to_vec(&program).unwrap()).unwrap();
    let mut vm = Vm::new(program, hosts).unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(10), vm.run())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(vm.stats().live, 0);
    result
}
#[tokio::test]
async fn canonical_signal_aliases_first_reason_and_readonly_fields_work() {
    let mut catalog = hosts();
    let signal = HostType::Handle("AbortSignal".into());
    catalog
        .register(
            HostOp::new(
                "sameSignal",
                vec![signal.clone(), signal],
                HostType::Bool,
                false,
                |args| HostReply::Ready(Ok(HostValue::Bool(args[0] == args[1]))),
            )
            .with_global_binding(),
        )
        .unwrap();
    let source = r#"fn main() string {
        const controller=AbortController(); const signal=controller.signal; const shared=controller.signal;
        const before=string(signal.aborted)+":"+string(signal.reason)+":"+string(sameSignal(signal,shared));
        controller.abort("stop"); controller.abort("later");
        const other=AbortController(); other.abort();
        return before+";"+string(shared.aborted)+":"+string(shared.reason)+";"+string(other.signal.reason);
    }"#;
    assert_eq!(
        run(source, catalog).await,
        HostValue::String("false:None:true;true:Some(\"stop\");Some(\"AbortError\")".into())
    );
}
#[tokio::test(start_paused = true)]
async fn namespace_aliases_and_typed_timeout_results_execute() {
    let source = r#"fn main() string {
        const signals=AbortSignal; const timeout=signals.timeout;
        const signal=unwrap(timeout(0)) or{return "timeout";};
        return string(signal.aborted)+":"+string(signal.reason)+":"+match AbortSignal.timeout(-1){Ok(s)=>"bad",Err(e)=>"invalid"};
    }"#;
    assert_eq!(
        run(source, hosts()).await,
        HostValue::String("true:Some(\"TimeoutError\"):invalid".into())
    );
    assert_eq!(run(r#"fn main() string { const AbortSignal={timeout:fn(n:number) string{return "local";}};return AbortSignal.timeout(1); }"#,hosts()).await,HostValue::String("local".into()));
}
#[test]
fn incorrect_signal_options_and_forged_handles_are_compile_errors() {
    for source in [
        "fn main(){AbortController(1);}",
        "fn main(){AbortSignal.timeout(\"soon\");}",
        "fn main(){AbortSignal();}",
        "fn change(s:AbortSignal){s.aborted=true;}",
        "fn change(s:AbortSignal){s.reason=Some(\"stop\");}",
        "fn main(){const c=AbortController();c.abort(7);}",
        "fn main(){fetch(\"http://example.test\",{signal:Some(7)});}",
        "fn main(){fetch(\"http://example.test\",{signal:Some({aborted:true})});}",
    ] {
        assert!(compiler::compile(source, &hosts()).is_err(), "{source}");
    }
    compiler::compile("fn main(){fetch(\"http://example.test\",{});}", &hosts()).unwrap();
    compiler::compile(
        "fn main(){fetch(\"http://example.test\",{signal:None});}",
        &hosts(),
    )
    .unwrap();
}
#[tokio::test]
async fn already_aborted_requests_never_open_a_connection_and_do_not_throw() {
    let server = Server::new(|_, _| Ok(reply("200 OK", "", b"unexpected"))).unwrap();
    let source = format!(
        r#"async fn main() Promise<string>{{
        const c=AbortController();c.abort("pre-aborted");
        try {{return match await fetch("{}",{{signal:Some(c.signal)}}){{Ok(r)=>"bad",Err(e)=>e}};}}
        catch(e){{return "threw";}}
    }}"#,
        server.url("/never")
    );
    assert_eq!(
        run(&source, hosts()).await,
        HostValue::String("pre-aborted".into())
    );
    assert!(server.finish().unwrap().is_empty());
}
#[tokio::test]
async fn cancelling_one_real_request_disconnects_it_and_preserves_other_tasks() {
    use std::io::Read;
    let (seen_tx, seen_rx) = tokio::sync::oneshot::channel();
    let seen_tx = std::sync::Mutex::new(Some(seen_tx));
    let server = Server::new(move |path, stream| {
        if path == "/cancel" {
            let tx = seen_tx
                .lock()
                .map_err(|_| std::io::Error::other("poisoned gate"))?
                .take()
                .ok_or_else(|| std::io::Error::other("duplicate cancelled request"))?;
            tx.send(())
                .map_err(|_| std::io::Error::other("VM dropped arrival gate"))?;
            let mut byte = [0];
            match stream.read(&mut byte) {
                Ok(0) => Ok(vec![]),
                Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => Ok(vec![]),
                Ok(_) => Err(std::io::Error::other(
                    "cancelled request remained connected",
                )),
                Err(e) => Err(e),
            }
        } else {
            Ok(reply("200 OK", "", br#""other completed""#))
        }
    })
    .unwrap();
    let mut hosts = hosts();
    let arrival = std::cell::RefCell::new(Some(seen_rx));
    hosts
        .register(
            HostOp::new("arrived", vec![], HostType::Unit, true, move |_| {
                let receiver = arrival.borrow_mut().take().unwrap();
                HostReply::Pending(Box::pin(async move {
                    receiver.await.map_err(|e| e.to_string())?;
                    Ok(HostValue::Unit)
                }))
            })
            .with_global_binding(),
        )
        .unwrap();
    let source = format!(
        r#"async fn main() Promise<string>{{
        const c=AbortController();
        const cancelled=fetch("{}",{{signal:Some(c.signal)}});
        const other=fetch("{}",{{}});
        await arrived(); c.abort("one cancelled");
        const a=match await cancelled{{Ok(r)=>"bad",Err(e)=>e}};
        const b=unwrap(await other) or{{return "other failed";}};
        const text=unwrap(await b.json<string>()) or{{return "json failed";}};
        return a+";"+text;
    }}"#,
        server.url("/cancel"),
        server.url("/other")
    );
    assert_eq!(
        run(&source, hosts).await,
        HostValue::String("one cancelled;other completed".into())
    );
    let requests = server.finish().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().any(|p| p == "/cancel"));
    assert!(requests.iter().any(|p| p == "/other"));
}

#[tokio::test]
async fn namespace_functions_survive_module_export_aliases_and_source_deletion() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("signals.ds"),
        "export const timeout=AbortSignal.timeout; export const make=AbortController;",
    )
    .unwrap();
    let entry = dir.path().join("main.ds");
    std::fs::write(&entry,r#"import {timeout,make} from "./signals.ds";
        fn main() string {const c=make();c.abort("module");const t=unwrap(timeout(0)) or{return "bad";};return string(t.aborted)+":"+string(c.signal.reason);}"#).unwrap();
    let hosts = hosts();
    let program = compiler::compile_file(&entry, &hosts, Some("main")).unwrap();
    let program = serde_json::from_slice(&serde_json::to_vec(&program).unwrap()).unwrap();
    drop(dir);
    let mut vm = Vm::new(program, hosts).unwrap();
    assert_eq!(
        vm.run().await.unwrap(),
        HostValue::String("true:Some(\"module\")".into())
    );
    assert_eq!(vm.stats().live, 0);
}
#[tokio::test]
async fn omitted_optional_record_fields_are_none_in_both_host_directions() {
    let schema =
        HostType::Record([("label".into(), HostType::Option(Box::new(HostType::String)))].into());
    let mut hosts = hosts();
    hosts
        .register(
            HostOp::new("optional", vec![schema.clone()], schema, false, |args| {
                let HostValue::Record(fields) = &args[0] else {
                    unreachable!()
                };
                assert_eq!(fields["label"], HostValue::Option(None));
                HostReply::Ready(Ok(HostValue::Record(Default::default())))
            })
            .with_defaults(vec![HostValue::Record(Default::default())])
            .with_global_binding(),
        )
        .unwrap();
    assert_eq!(
        run(
            "fn main() string{return string(optional({}).label)+string(optional().label);}",
            hosts
        )
        .await,
        HostValue::String("NoneNone".into())
    );
}
#[test]
fn namespace_catalog_rejects_duplicate_fields_and_both_global_collision_orders() {
    fn op(name: &str) -> HostOp {
        HostOp::new(name, vec![], HostType::Number, false, |_| {
            HostReply::Ready(Ok(HostValue::Number(1.)))
        })
    }
    let mut h = Hosts::default();
    h.register(op("static_a").with_namespace_binding("Tools", "read"))
        .unwrap();
    assert!(
        h.register(op("static_b").with_namespace_binding("Tools", "read"))
            .is_err()
    );
    assert!(h.register(op("Tools").with_global_binding()).is_err());
    assert!(
        h.register(op("bad").with_namespace_binding("Promise", "read"))
            .is_err()
    );
    let mut h = Hosts::default();
    h.register(op("Tools").with_global_binding()).unwrap();
    assert!(
        h.register(op("static_a").with_namespace_binding("Tools", "read"))
            .is_err()
    );
    assert!(
        h.register(
            op("mixed")
                .with_global_binding()
                .with_namespace_binding("Other", "read")
        )
        .is_err()
    );
}

#[tokio::test]
async fn a_deadline_interrupts_an_actual_partial_body_on_the_vm_clock() {
    use std::io::{Read, Write};
    use std::task::Poll;
    let (arrived, mut received) = tokio::sync::oneshot::channel();
    let arrived = std::sync::Mutex::new(Some(arrived));
    let server = Server::new(move |_, stream| {
        stream.write_all(
            b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nConnection: close\r\n\r\npartial",
        )?;
        arrived
            .lock()
            .map_err(|_| std::io::Error::other("poisoned arrival"))?
            .take()
            .ok_or_else(|| std::io::Error::other("duplicate timeout request"))?
            .send(())
            .map_err(|_| std::io::Error::other("arrival receiver closed"))?;
        let mut byte = [0];
        match stream.read(&mut byte) {
            Ok(0) => Ok(vec![]),
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => Ok(vec![]),
            Ok(_) => Err(std::io::Error::other("timed out body remained connected")),
            Err(e) => Err(e),
        }
    })
    .unwrap();
    let hosts = hosts();
    let source = format!(
        r#"async fn main() Promise<string>{{
        const signal=unwrap(AbortSignal.timeout(10000000)) or{{return "duration";}};
        const request=fetch("{}",{{signal:Some(signal)}});
        const result=match await request{{Ok(r)=>"bad",Err(e)=>e}};
        return result+":"+string(signal.aborted)+":"+string(signal.reason);
    }}"#,
        server.url("/partial")
    );
    let mut vm = Vm::new(compiler::compile(&source, &hosts).unwrap(), hosts).unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        std::future::poll_fn(|cx| {
            assert!(
                vm.poll(cx).is_pending(),
                "partial response unexpectedly completed"
            );
            if std::future::Future::poll(std::pin::Pin::new(&mut received), cx).is_ready() {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }),
    )
    .await
    .unwrap();
    // Do not let a paused clock auto-advance before real socket arrival.
    tokio::time::pause();
    tokio::time::advance(std::time::Duration::from_millis(10000000)).await;
    assert_eq!(
        vm.run().await.unwrap(),
        HostValue::String("TimeoutError:true:Some(\"TimeoutError\")".into())
    );
    assert_eq!(vm.stats().live, 0);
    tokio::time::resume();
    assert_eq!(server.finish().unwrap(), ["/partial"]);
}

#[tokio::test]
async fn typed_optional_fields_are_nominal_none_before_crossing_a_host_boundary() {
    assert_eq!(
        run(
            r#"fn read(options:{label:Option<string>}) string {
        return match options.label {Some(text)=>text,None=>"absent"};
    }
    fn main() string{return read({})+":"+read({label:Some("present")});}"#,
            hosts()
        )
        .await,
        HostValue::String("absent:present".into())
    );
    assert!(
        compiler::compile(
            "fn read(o:{required:string}) string{return o.required;} fn main(){read({});}",
            &hosts()
        )
        .is_err()
    );
}
