use deka_vm::*;
use std::task::{Context, Poll, Waker};
fn schema() -> HostEnum {
    HostEnum {
        name: "NativeFault".into(),
        cases: vec![("Failed".into(), Some(HostType::String))],
    }
}
fn execute(hosts: Hosts) -> Result<HostValue> {
    let program = Program {
        version: 1,
        functions: vec![Function {
            name: "main".into(),
            parameters: 0,
            captures: 0,
            locals: 0,
            asynchronous: false,
            code: vec![
                Op::Host {
                    operation: "read".into(),
                    arguments: 0,
                },
                Op::Return,
            ],
        }],
    };
    let program = serde_json::from_str(&serde_json::to_string(&program).unwrap()).unwrap();
    let mut vm = Vm::new(program, hosts)?;
    let mut cx = Context::from_waker(Waker::noop());
    for _ in 0..32 {
        if let Poll::Ready(value) = vm.poll(&mut cx) {
            return value;
        }
    }
    panic!("core program did not finish");
}
#[test]
fn typed_enum_values_are_marshaled_without_a_compiler_or_async_host_runtime() {
    let mut hosts = Hosts::default();
    hosts
        .register(HostOp::new(
            "read",
            vec![],
            HostType::Enum(Box::new(schema())),
            false,
            |_| {
                HostReply::Ready(Ok(HostValue::Enum {
                    name: schema().brand(),
                    case: "Failed".into(),
                    payload: Some(Box::new(HostValue::String("core".into()))),
                }))
            },
        ))
        .unwrap();
    assert_eq!(
        execute(hosts).unwrap(),
        HostValue::Enum {
            name: schema().brand(),
            case: "Failed".into(),
            payload: Some(Box::new(HostValue::String("core".into())))
        }
    );
}
#[test]
fn typed_operational_error_is_nested_inside_result_without_a_compiler() {
    let mut hosts = Hosts::default();
    hosts
        .register(
            HostOp::new("read", vec![], HostType::Bytes, false, |_| {
                HostReply::Ready(Err("missing".into()))
            })
            .with_enum_result_channel(schema(), "Failed"),
        )
        .unwrap();
    assert_eq!(
        execute(hosts).unwrap(),
        HostValue::Enum {
            name: "Result".into(),
            case: "Err".into(),
            payload: Some(Box::new(HostValue::Enum {
                name: schema().brand(),
                case: "Failed".into(),
                payload: Some(Box::new(HostValue::String("missing".into())))
            }))
        }
    );
}
