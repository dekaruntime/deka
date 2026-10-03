#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    task::{Context, Poll, Waker},
};
fn record<const N: usize>(fields: [(&str, HostValue); N]) -> HostValue {
    HostValue::Record(fields.into_iter().map(|(k, v)| (k.into(), v)).collect())
}
fn schema<const N: usize>(fields: [(&str, HostType); N]) -> HostType {
    HostType::Record(fields.into_iter().map(|(k, v)| (k.into(), v)).collect())
}
fn run(source: &str, hosts: Hosts) -> Result<HostValue> {
    let program = compiler::compile(source, &hosts)?;
    let mut vm = Vm::new(program, hosts)?;
    let mut cx = Context::from_waker(Waker::noop());
    for _ in 0..1000 {
        if let Poll::Ready(result) = vm.poll(&mut cx) {
            assert_eq!(vm.stats().live, 0);
            return result;
        }
    }
    panic!("VM did not complete");
}
fn response_type() -> HostType {
    schema([
        ("status", HostType::Number),
        (
            "headers",
            HostType::List(Box::new(schema([
                ("name", HostType::String),
                ("value", HostType::String),
            ]))),
        ),
        ("body", HostType::Bytes),
    ])
}
fn response() -> HostValue {
    record([
        ("status", HostValue::Number(201.)),
        (
            "headers",
            HostValue::List(vec![record([
                ("name", HostValue::String("test".into())),
                ("value", HostValue::String("yes".into())),
            ])]),
        ),
        ("body", HostValue::Bytes(vec![0, 128, 255])),
    ])
}
#[test]
fn nested_records_lists_and_bytes_round_trip_without_losing_values() {
    let mut hosts = Hosts::default();
    hosts
        .register(HostOp::new(
            "response",
            vec![],
            response_type(),
            false,
            None,
            |_| HostReply::Ready(Ok(response())),
        ))
        .unwrap();
    hosts
        .register(HostOp::new(
            "inspect",
            vec![response_type()],
            HostType::Bool,
            false,
            None,
            |args| HostReply::Ready(Ok(HostValue::Bool(args[0] == response()))),
        ))
        .unwrap();
    assert_eq!(run(r#"import { response, inspect } from "vm:host";
fn main() boolean { const r = response(); const body = r.body; return r.status == 201 && body.length == 3 && body[2] == 255 && inspect(r); }"#,hosts).unwrap(),HostValue::Bool(true));
}
#[test]
fn authored_records_numeric_and_empty_lists_cross_to_rust() {
    let mut hosts = Hosts::default();
    let ty = schema([
        ("items", HostType::List(Box::new(HostType::Number))),
        ("flags", HostType::List(Box::new(HostType::Bool))),
    ]);
    hosts
        .register(HostOp::new(
            "sum",
            vec![ty],
            HostType::Number,
            false,
            None,
            |args| {
                let HostValue::Record(fields) = &args[0] else {
                    panic!("not a record")
                };
                let HostValue::List(items) = &fields["items"] else {
                    panic!("not a list")
                };
                assert_eq!(fields["flags"], HostValue::List(vec![]));
                let sum = items
                    .iter()
                    .map(|v| {
                        let HostValue::Number(n) = v else {
                            panic!("not a number")
                        };
                        n
                    })
                    .sum();
                HostReply::Ready(Ok(HostValue::Number(sum)))
            },
        ))
        .unwrap();
    assert_eq!(run(r#"import { sum } from "vm:host"; fn main() number { return sum({items: [2, 3, 7], flags: []}); }"#,hosts).unwrap(),HostValue::Number(12.));
}
#[test]
fn legacy_strings_and_generic_string_lists_use_the_same_wire_path() {
    let mut hosts = Hosts::default();
    hosts
        .register(HostOp::new(
            "old",
            vec![HostType::Strings],
            HostType::Strings,
            false,
            None,
            |args| {
                assert_eq!(args[0], HostValue::Strings(vec!["hi".into()]));
                HostReply::Ready(Ok(args[0].clone()))
            },
        ))
        .unwrap();
    hosts
        .register(HostOp::new(
            "new",
            vec![HostType::List(Box::new(HostType::String))],
            HostType::Bool,
            false,
            None,
            |args| {
                HostReply::Ready(Ok(HostValue::Bool(
                    args[0] == HostValue::List(vec![HostValue::String("hi".into())]),
                )))
            },
        ))
        .unwrap();
    assert_eq!(
        run(
            r#"import { old, new } from "vm:host"; fn main() boolean { return new(old(["hi"])); }"#,
            hosts
        )
        .unwrap(),
        HostValue::Bool(true)
    );
}
#[test]
fn async_rich_responses_and_failures_use_the_existing_result_constructor() {
    let mut hosts = Hosts::default();
    hosts
        .register(
            HostOp::new(
                "load",
                vec![HostType::Bool],
                response_type(),
                true,
                None,
                |args| {
                    let fail = args[0] == HostValue::Bool(true);
                    HostReply::Pending(Box::pin(async move {
                        if fail {
                            Err("offline".into())
                        } else {
                            Ok(response())
                        }
                    }))
                },
            )
            .with_result_channel(),
        )
        .unwrap();
    assert_eq!(
        run(
            r#"import { load } from "vm:host";
async fn main() Promise<string> {
    const good = await load(false); const bad = await load(true);
    const label = match(good) { Ok(r) => string(r.body.length), Err(e) => e };
    return label + match(bad) { Ok(r) => "wrong", Err(e) => e };
}"#,
            hosts
        )
        .unwrap(),
        HostValue::String("3offline".into())
    );
}
#[test]
fn malformed_host_outputs_are_protocol_errors_including_async_results() {
    for asynchronous in [false, true] {
        let mut hosts = Hosts::default();
        hosts
            .register(
                HostOp::new(
                    "wrong",
                    vec![],
                    response_type(),
                    asynchronous,
                    None,
                    move |_| {
                        let value = Ok(record([(
                            "status",
                            HostValue::String("not a number".into()),
                        )]));
                        if asynchronous {
                            HostReply::Pending(Box::pin(async move { value }))
                        } else {
                            HostReply::Ready(value)
                        }
                    },
                )
                .with_result_channel(),
            )
            .unwrap();
        let source = if asynchronous {
            "import { wrong } from \"vm:host\"; async fn main() { await wrong(); }"
        } else {
            "import { wrong } from \"vm:host\"; fn main() { wrong(); }"
        };
        assert_eq!(
            run(source, hosts).unwrap_err(),
            "host returned the wrong result type"
        );
    }
}
#[test]
fn structural_inputs_and_opaque_brands_are_checked_before_execution() {
    let mut hosts = Hosts::default();
    hosts
        .register(HostOp::new(
            "consume",
            vec![response_type()],
            HostType::Unit,
            false,
            None,
            |_| panic!("invalid source must not run"),
        ))
        .unwrap();
    for value in [
        "{status: 1}",
        "{status: \"oops\", headers: [], body: []}",
        "{status: 1, headers: [2], body: []}",
    ] {
        assert!(
            compiler::compile(
                &format!("import {{consume}} from \"vm:host\"; fn main() {{ consume({value}); }}"),
                &hosts
            )
            .is_err()
        );
    }
    hosts
        .register(HostOp::new(
            "socket",
            vec![],
            HostType::Handle("Socket".into()),
            false,
            None,
            |_| HostReply::Ready(Ok(HostValue::Handle(HostHandle::new("Socket", 7u32)))),
        ))
        .unwrap();
    hosts
        .register(HostOp::new(
            "body",
            vec![HostType::Handle("Body".into())],
            HostType::Unit,
            false,
            None,
            |_| panic!("wrong brand"),
        ))
        .unwrap();
    for statement in [
        "body(socket());",
        "body({});",
        "const s = socket(); const x = s.value;",
    ] {
        assert!(
            compiler::compile(
                &format!("import {{body, socket}} from \"vm:host\"; fn main() {{ {statement} }}"),
                &hosts
            )
            .is_err()
        );
    }
}
struct Resource(Rc<Cell<usize>>);
impl Drop for Resource {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}
fn resource_hosts(drops: Rc<Cell<usize>>) -> Hosts {
    let mut hosts = Hosts::default();
    hosts
        .register(HostOp::new(
            "open",
            vec![],
            HostType::Handle("Socket".into()),
            false,
            None,
            move |_| {
                HostReply::Ready(Ok(HostValue::Handle(HostHandle::new(
                    "Socket",
                    Resource(drops.clone()),
                ))))
            },
        ))
        .unwrap();
    hosts
        .register(HostOp::new(
            "useSocket",
            vec![HostType::Handle("Socket".into())],
            HostType::Bool,
            false,
            None,
            |args| {
                let HostValue::Handle(handle) = &args[0] else {
                    panic!("not a handle")
                };
                HostReply::Ready(Ok(HostValue::Bool(
                    handle.downcast_ref::<Resource>().is_some(),
                )))
            },
        ))
        .unwrap();
    hosts
}
#[test]
fn opaque_aliases_round_trip_and_rust_ownership_outlives_vm_when_returned() {
    let drops = Rc::new(Cell::new(0));
    let value = run(r#"import {open, useSocket} from "vm:host"; fn main() { const a = open(); const b = a; if (useSocket(b)) { return b; } return a; }"#,resource_hosts(drops.clone())).unwrap();
    assert_eq!(drops.get(), 0);
    let HostValue::Handle(handle) = &value else {
        panic!("not an opaque handle")
    };
    assert!(handle.downcast_ref::<Resource>().is_some());
    drop(value);
    assert_eq!(drops.get(), 1);
}
#[test]
fn gc_releases_resources_from_returned_frames_and_cancellation_drops_pending_owners() {
    let drops = Rc::new(Cell::new(0));
    let mut hosts = resource_hosts(drops.clone());
    let held = Rc::new(RefCell::new(None));
    let capture = held.clone();
    hosts
        .register(HostOp::new(
            "gate",
            vec![HostType::Handle("Socket".into())],
            HostType::Unit,
            true,
            None,
            move |args| {
                *capture.borrow_mut() = Some(args[0].clone());
                HostReply::Pending(Box::pin(async move {
                    std::future::pending::<()>().await;
                    drop(args);
                    Ok(HostValue::Unit)
                }))
            },
        ))
        .unwrap();
    let program = compiler::compile(
        r#"import {open, useSocket, gate} from "vm:host";
fn discard() { const resource = open(); useSocket(resource); }
async fn main() { discard(); await gate(open()); }"#,
        &hosts,
    )
    .unwrap();
    let mut vm = Vm::new(program, hosts).unwrap();
    let mut cx = Context::from_waker(Waker::noop());
    for _ in 0..8 {
        assert!(vm.poll(&mut cx).is_pending());
    }
    vm.collect().unwrap();
    assert_eq!(
        drops.get(),
        1,
        "returned frame should not retain its resource"
    );
    held.borrow_mut().take();
    assert_eq!(drops.get(), 1, "pending future owns second resource");
    vm.cancel().unwrap();
    assert_eq!(drops.get(), 2);
    assert_eq!(vm.stats().live, 0);
}
#[test]
fn a_wrong_handle_brand_from_rust_is_rejected() {
    let mut hosts = Hosts::default();
    hosts
        .register(HostOp::new(
            "wrong",
            vec![],
            HostType::Handle("Body".into()),
            false,
            None,
            |_| HostReply::Ready(Ok(HostValue::Handle(HostHandle::new("Socket", 7u32)))),
        ))
        .unwrap();
    assert_eq!(
        run(
            "import {wrong} from \"vm:host\"; fn main() { wrong(); }",
            hosts
        )
        .unwrap_err(),
        "host returned the wrong result type"
    );
}
#[test]
fn host_schema_names_cannot_inject_declarations() {
    for ty in [
        HostType::Handle("Body; export fn injected() void".into()),
        schema([("field: number}", HostType::Number)]),
    ] {
        assert!(
            Hosts::default()
                .register(HostOp::new("bad", vec![], ty, false, None, |_| {
                    HostReply::Ready(Ok(HostValue::Unit))
                }))
                .is_err()
        );
    }
}

#[test]
fn byte_indexing_and_union_patterns_use_the_byte_value_kind() {
    let mut hosts = Hosts::default();
    hosts
        .register(HostOp::new(
            "buffer",
            vec![],
            HostType::Bytes,
            false,
            None,
            |_| HostReply::Ready(Ok(HostValue::Bytes(vec![128, 255]))),
        ))
        .unwrap();
    assert_eq!(run(r#"import {buffer} from "vm:host";
fn classify(value: bytes | string) number { return match(value) { bytes(b) => b[0], string(s) => 0 }; }
fn main() number { return classify(buffer()); }"#, hosts.clone()).unwrap(), HostValue::Number(128.));
    assert_eq!(run(r#"import {buffer} from "vm:host"; fn main() number { const b = buffer(); return b[2]; }"#,hosts).unwrap_err(), "index out of bounds");
}

#[test]
fn structural_record_annotations_check_fields_in_local_functions() {
    let hosts = Hosts::default();
    let source = r#"fn status(value: {status: number, label: string}) number { return value.status; }
fn main() number { return status({status: 204, label: "ok"}); }"#;
    assert_eq!(run(source, hosts.clone()).unwrap(), HostValue::Number(204.));
    assert!(compiler::compile(&source.replace("status: 204", "status: true"), &hosts).is_err());
}

#[test]
fn bytes_are_immutable_even_when_the_binding_is_mutable() {
    let mut hosts = Hosts::default();
    hosts
        .register(HostOp::new(
            "buffer",
            vec![],
            HostType::Bytes,
            false,
            None,
            |_| HostReply::Ready(Ok(HostValue::Bytes(vec![7]))),
        ))
        .unwrap();
    for binding in ["let", "const"] {
        let error = compiler::compile(&format!("import {{buffer}} from \"vm:host\"; fn main() {{ {binding} b = buffer(); b[0] = 42; }}"), &hosts).unwrap_err();
        assert!(
            error.contains("cannot assign to immutable bytes"),
            "{error}"
        );
    }
}
