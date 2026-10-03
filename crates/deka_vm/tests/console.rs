#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;
use std::{cell::RefCell, rc::Rc};

type Lines = Rc<RefCell<Vec<(bool, String)>>>;
fn hosts() -> (Hosts, Lines) {
    let lines = Rc::new(RefCell::new(Vec::new()));
    let mut hosts = Hosts::default();
    for (name, diagnostic) in [("echo", false), (compiler::CONSOLE_ERROR_OPERATION, true)] {
        let output = lines.clone();
        hosts
            .register(HostOp::new(
                name,
                vec![HostType::String],
                HostType::Unit,
                false,
                None,
                move |args| {
                    let HostValue::String(text) = &args[0] else {
                        panic!("console sink expects text");
                    };
                    output.borrow_mut().push((diagnostic, text.clone()));
                    HostReply::Ready(Ok(HostValue::Unit))
                },
            ))
            .unwrap();
    }
    (hosts, lines)
}

#[tokio::test]
async fn each_console_level_uses_the_shared_struct_enum_option_and_result_printer() {
    for (method, diagnostic) in [
        ("log", false),
        ("info", false),
        ("debug", false),
        ("warn", true),
        ("error", true),
    ] {
        let (hosts, output) = hosts();
        let source = format!(
            r#"
            struct Point {{ x: number; }}
            enum Event {{ Ready, Loaded(Point), }}
            fn main() {{
                console.{method}("plain", Point {{ x: 7 }}, Event.Loaded(Point {{ x: 7 }}), Some(7), None, Ok(7), Err("io"), [1, 2]);
                console.{method}();
            }}
        "#
        );
        let program = compiler::compile(&source, &hosts).unwrap();
        let program = serde_json::from_str(&serde_json::to_string(&program).unwrap()).unwrap();
        Vm::new(program, hosts).unwrap().run().await.unwrap();
        assert_eq!(&*output.borrow(), &[
            (diagnostic, r#"plain Point { x: 7 } Loaded(Point { x: 7 }) Some(7) None Ok(7) Err("io") [ 1, 2 ]"#.into()),
            (diagnostic, String::new()),
        ], "{method}");
    }
}

#[tokio::test]
async fn console_arguments_run_once_in_source_order() {
    let (hosts, output) = hosts();
    let program = compiler::compile(
        r#"
        let n = 0;
        fn next() number { n += 1; return n; }
        fn main() { console.warn(next(), next()); console.info(next()); }
    "#,
        &hosts,
    )
    .unwrap();
    Vm::new(program, hosts).unwrap().run().await.unwrap();
    assert_eq!(
        &*output.borrow(),
        &[(true, "1 2".into()), (false, "3".into())]
    );
}

#[tokio::test]
async fn a_local_console_binding_uses_ordinary_function_types_and_calls() {
    let source = r#"
        fn main() number {
            const console = {warn: fn(f: fn(number) number) number { return f(4); }};
            return console.warn(fn(n: number) number { return n + 1; });
        }
    "#;
    let hosts = Hosts::default();
    let program = compiler::compile(source, &hosts).unwrap();
    assert_eq!(
        Vm::new(program, hosts).unwrap().run().await.unwrap(),
        HostValue::Number(5.)
    );
}

#[test]
fn every_console_level_rejects_function_values_before_running() {
    let (hosts, _) = hosts();
    for method in ["log", "info", "debug", "warn", "error"] {
        let source =
            format!("fn main() {{ console.{method}(fn(n:number) number {{return n;}}); }}");
        let error = compiler::compile(&source, &hosts).unwrap_err();
        assert!(
            error.contains(&format!(
                "console.{method}: value of type fn(number) number has no printable form"
            )),
            "{error}"
        );
    }
}

#[test]
fn diagnostic_output_requires_a_typed_stderr_sink() {
    let (mut hosts, _) = demo::hosts().unwrap();
    let source = "fn main() {console.error(7);}";
    let error = compiler::compile(source, &hosts).unwrap_err();
    assert!(
        error.contains("registered console_stderr(string) output operation"),
        "{error}"
    );
    hosts
        .register(HostOp::new(
            compiler::CONSOLE_ERROR_OPERATION,
            vec![HostType::String],
            HostType::Number,
            false,
            None,
            |_| HostReply::Ready(Ok(HostValue::Number(1.))),
        ))
        .unwrap();
    let error = compiler::compile(source, &hosts).unwrap_err();
    assert!(
        error.contains("registered console_stderr(string) output operation"),
        "{error}"
    );
}
