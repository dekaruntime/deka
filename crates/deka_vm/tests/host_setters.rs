#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;
use std::{
    cell::RefCell,
    rc::Rc,
    task::{Context, Poll, Waker},
};

fn drive(program: Program, hosts: Hosts) -> Result<HostValue> {
    let mut vm = Vm::new(program, hosts)?;
    let mut cx = Context::from_waker(Waker::noop());
    for _ in 0..1000 {
        if let Poll::Ready(value) = vm.poll(&mut cx) {
            assert_eq!(vm.stats().live, 0);
            return value;
        }
    }
    panic!("VM did not complete");
}
fn run(source: &str, hosts: Hosts) -> Result<HostValue> {
    drive(compiler::compile(source, &hosts)?, hosts)
}
fn setter() -> HostOp {
    HostOp::new(
        "write_text",
        vec![HostType::Handle("Resource".into()), HostType::String],
        HostType::Unit,
        false,
        |_| HostReply::Ready(Ok(HostValue::Unit)),
    )
    .with_receiver_setter("Resource", "text")
}
fn getter(ty: HostType) -> HostOp {
    HostOp::new(
        "read_text",
        vec![HostType::Handle("Resource".into())],
        ty,
        false,
        |_| HostReply::Ready(Ok(HostValue::String("old".into()))),
    )
    .with_receiver_property("Resource", "text")
}
type ResourceFixture = (Hosts, Rc<RefCell<String>>, Rc<RefCell<Vec<&'static str>>>);
fn resource_hosts(writable: bool) -> ResourceFixture {
    let state = Rc::new(RefCell::new(String::from("old")));
    let events = Rc::new(RefCell::new(Vec::new()));
    let handle = HostHandle::new("Resource", state.clone());
    let mut hosts = Hosts::default();
    let log = events.clone();
    hosts
        .register(HostOp::new(
            "receiver",
            vec![],
            HostType::Handle("Resource".into()),
            false,
            move |_| {
                log.borrow_mut().push("receiver");
                HostReply::Ready(Ok(HostValue::Handle(handle.clone())))
            },
        ))
        .unwrap();
    let log = events.clone();
    hosts
        .register(HostOp::new(
            "rhs",
            vec![],
            HostType::String,
            false,
            move |_| {
                log.borrow_mut().push("rhs");
                HostReply::Ready(Ok(HostValue::String("new".into())))
            },
        ))
        .unwrap();
    let log = events.clone();
    hosts
        .register(
            HostOp::new(
                "read_text",
                vec![HostType::Handle("Resource".into())],
                HostType::String,
                false,
                move |args| {
                    log.borrow_mut().push("getter");
                    let HostValue::Handle(handle) = &args[0] else {
                        panic!("receiver");
                    };
                    let value = handle
                        .downcast_ref::<Rc<RefCell<String>>>()
                        .unwrap()
                        .borrow()
                        .clone();
                    HostReply::Ready(Ok(HostValue::String(value)))
                },
            )
            .with_receiver_property("Resource", "text"),
        )
        .unwrap();
    if writable {
        let log = events.clone();
        hosts
            .register(
                HostOp::new(
                    "write_text",
                    vec![HostType::Handle("Resource".into()), HostType::String],
                    HostType::Unit,
                    false,
                    move |args| {
                        log.borrow_mut().push("setter");
                        let HostValue::Handle(handle) = &args[0] else {
                            panic!("receiver");
                        };
                        let HostValue::String(value) = &args[1] else {
                            panic!("value");
                        };
                        *handle
                            .downcast_ref::<Rc<RefCell<String>>>()
                            .unwrap()
                            .borrow_mut() = value.clone();
                        HostReply::Ready(Ok(HostValue::Unit))
                    },
                )
                .with_receiver_setter("Resource", "text"),
            )
            .unwrap();
    }
    (hosts, state, events)
}
#[test]
fn resource_mutation_evaluates_receiver_then_rhs_once_and_returns_value() {
    let (hosts, state, events) = resource_hosts(true);
    assert_eq!(
        run(
            r#"import {receiver, rhs} from "vm:host";
fn main() string { return receiver().text = rhs(); }"#,
            hosts
        )
        .unwrap(),
        HostValue::String("new".into())
    );
    assert_eq!(&*state.borrow(), "new");
    assert_eq!(&*events.borrow(), &["receiver", "rhs", "setter"]);
}
#[test]
fn immutable_handle_aliases_observe_mutation_and_chained_assignment_values() {
    let (hosts, state, events) = resource_hosts(true);
    assert_eq!(
        run(
            r#"import {receiver, rhs} from "vm:host";
fn main() string { const node = receiver(); const shared = node;
const value = (node.text = (shared.text = rhs())); return value + ":" + shared.text; }"#,
            hosts
        )
        .unwrap(),
        HostValue::String("new:new".into())
    );
    assert_eq!(&*state.borrow(), "new");
    assert_eq!(
        &*events.borrow(),
        &["receiver", "rhs", "setter", "setter", "getter"]
    );
}
#[test]
fn getter_only_fields_and_ordinary_immutable_fields_remain_readonly() {
    let (hosts, _, _) = resource_hosts(false);
    for binding in ["const", "let"] {
        let source = format!(
            "import {{receiver}} from \"vm:host\"; fn main() {{ {binding} node = receiver(); node.text = \"new\"; }}"
        );
        let error = compiler::compile(&source, &hosts).unwrap_err();
        assert!(
            error.contains("cannot assign to read-only host property `text`"),
            "{error}"
        );
    }
    let error = compiler::compile(
        "fn main() { const value = {text: \"old\"}; value.text = \"new\"; }",
        &hosts,
    )
    .unwrap_err();
    assert!(error.contains("immutable"), "{error}");
}
#[test]
fn setter_assignment_is_checked_against_its_declared_value_type() {
    let (hosts, _, _) = resource_hosts(true);
    let error = compiler::compile(r#"import {receiver} from "vm:host"; fn main() { const node = receiver(); node.text = 42; }"#, &hosts).unwrap_err();
    assert!(
        error.contains("string") && error.contains("number"),
        "{error}"
    );
}
#[test]
fn setter_schema_rejects_defaults_channels_async_and_non_unit_outputs() {
    let mut cases = vec![
        setter().with_defaults(vec![HostValue::String("default".into())]),
        setter().with_result_channel(),
        setter().with_exception_channel(),
        setter().with_global_binding(),
        setter().with_namespace_binding("resource", "write"),
        setter().with_receiver_method("Resource", "write"),
        setter().with_json_body(),
    ];
    let mut asynchronous = setter();
    asynchronous.asynchronous = true;
    cases.push(asynchronous);
    let mut output = setter();
    output.result = HostType::String;
    cases.push(output);
    let mut receiver = setter();
    receiver.args[0] = HostType::String;
    cases.push(receiver);
    for op in cases {
        assert!(Hosts::default().register(op).is_err());
    }
    let mut hosts = Hosts::default();
    hosts.register(setter()).unwrap();
    let mut duplicate = setter();
    duplicate.name = "duplicate".into();
    assert!(
        hosts
            .register(duplicate)
            .unwrap_err()
            .contains("host setter")
    );
}
#[test]
fn getter_setter_pairs_share_the_same_type_in_either_registration_order() {
    for reverse in [false, true] {
        let mut hosts = Hosts::default();
        let (first, second) = if reverse {
            (setter(), getter(HostType::Number))
        } else {
            (getter(HostType::Number), setter())
        };
        hosts.register(first).unwrap();
        assert!(
            hosts
                .register(second)
                .unwrap_err()
                .contains("same total property value type")
        );
        let mut hosts = Hosts::default();
        let (first, second) = if reverse {
            (setter(), getter(HostType::String))
        } else {
            (getter(HostType::String), setter())
        };
        hosts.register(first).unwrap();
        hosts.register(second).unwrap();
        for read in [
            getter(HostType::String).with_result_channel(),
            getter(HostType::String).with_exception_channel(),
            getter(HostType::String).with_receiver_method("Resource", "text"),
        ] {
            let mut hosts = Hosts::default();
            hosts.register(setter()).unwrap();
            assert!(hosts.register(read).is_err());
        }
    }
}
#[test]
fn other_opaque_brands_and_structural_impostors_do_not_gain_setters() {
    let (mut hosts, _, _) = resource_hosts(true);
    hosts
        .register(HostOp::new(
            "other",
            vec![],
            HostType::Handle("Other".into()),
            false,
            |_| HostReply::Ready(Ok(HostValue::Handle(HostHandle::new("Other", ())))),
        ))
        .unwrap();
    for body in [
        "const node = other(); node.text = \"new\";",
        "const node = {text: \"old\"}; node.text = \"new\";",
    ] {
        assert!(
            compiler::compile(
                &format!("import {{other}} from \"vm:host\"; fn main() {{ {body} }}"),
                &hosts
            )
            .is_err()
        );
    }
}
#[test]
fn setter_metadata_survives_renamed_imports_and_source_free_bytecode() {
    let (hosts, state, _) = resource_hosts(true);
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("resource.ds"),
        r#"import {receiver} from "vm:host";
import type {Resource as LocalResource} from "vm:host";
export fn open() LocalResource { return receiver(); }
export fn update(node: LocalResource, value: string) string { return node.text = value; }"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("barrel.ds"),
        r#"export {open as create, update as change} from "./resource.ds";"#,
    )
    .unwrap();
    std::fs::write(dir.path().join("main.ds"), r#"import {create, change} from "./barrel.ds";
fn main() string { const node = create(); return change(node, "through modules") + ":" + node.text; }"#).unwrap();
    let program =
        compiler::compile_file(&dir.path().join("main.ds"), &hosts, Some("main")).unwrap();
    let bytes = serde_json::to_vec(&program).unwrap();
    for file in ["resource.ds", "barrel.ds", "main.ds"] {
        std::fs::remove_file(dir.path().join(file)).unwrap();
    }
    assert_eq!(
        drive(serde_json::from_slice(&bytes).unwrap(), hosts).unwrap(),
        HostValue::String("through modules:through modules".into())
    );
    assert_eq!(&*state.borrow(), "through modules");
}
