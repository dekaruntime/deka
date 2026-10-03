#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;

async fn run(source: &str) -> HostValue {
    let hosts = Hosts::default();
    let program = compiler::compile(source, &hosts).unwrap();
    let program = serde_json::from_str(&serde_json::to_string(&program).unwrap()).unwrap();
    Vm::new(program, hosts).unwrap().run().await.unwrap()
}

#[tokio::test]
async fn option_fallback_is_lazy_and_the_subject_runs_once() {
    assert_eq!(
        run(r#"
        fn main() string {
            let calls = 0;
            const next = fn() Option<number> { calls = calls + 1; return Some(7); };
            const n = unwrap(next()) or { calls = calls + 100; 1 }
            const absent: Option<number> = None;
            const m = unwrap(absent) or { calls = calls + 10; 2 }
            return string(n) + "," + string(m) + "," + string(calls);
        }
    "#)
        .await,
        HostValue::String("7,2,11".into())
    );
}

#[tokio::test]
async fn return_in_a_fallback_exits_the_enclosing_function() {
    assert_eq!(
        run(r#"
        fn label(value: Option<string>) string {
            const text = unwrap(value) or { return "exited"; }
            return "got:" + text;
        }
        fn main() string { return label(Some("yes")) + "," + label(None); }
    "#)
        .await,
        HostValue::String("got:yes,exited".into())
    );
}

#[tokio::test]
async fn result_match_extracts_the_error_but_ok_keeps_its_payload() {
    assert_eq!(run(r#"
        fn label(value: Result<string, string>) string {
            const text = unwrap(value) or match { Err(error) => "failed:" + error };
            return text;
        }
        fn block(value: Result<string, string>) string {
            const text = unwrap(value) or { "fallback" }
            return text;
        }
        fn main() string {
            return label(Ok("yes")) + "," + label(Err("bad")) + "," + block(Ok("ok")) + "," + block(Err("bad"));
        }
    "#).await, HostValue::String("yes,failed:bad,ok,fallback".into()));
}

#[tokio::test]
async fn module_bindings_execute_in_order_and_alternative_locals_stay_scoped() {
    assert_eq!(
        run(r#"
        const source: Option<string> = Some("module");
        const top = unwrap(source) or { "wrong" }
        fn main() string {
            const detail = "outer";
            const absent: Option<string> = None;
            let text = unwrap(absent) or { const detail = "inner"; detail }
            text = text + "!";
            return top + "," + text + "," + detail;
        }
    "#)
        .await,
        HostValue::String("module,inner!,outer".into())
    );
}

#[tokio::test]
async fn conditional_fallback_returns_stay_in_the_same_frame() {
    assert_eq!(run(r#"
        fn label(v: Option<string>, flag: boolean) string {
            const text = unwrap(v) or { if (flag) { return "a"; } else { return "b"; } }
            return text;
        }
        fn main() string { return label(None, true) + "," + label(None, false) + "," + label(Some("c"), true); }
    "#).await, HostValue::String("a,b,c".into()));
}

#[tokio::test]
async fn safe_wrapper_and_logical_or_preserve_the_existing_short_circuit_model() {
    assert_eq!(
        run(r#"
        fn main() string {
            let calls = 0;
            const next = fn() boolean { calls = calls + 1; return true; };
            const a = safe { true || next() };
            const b = safe { false || next() };
            return string(a) + "," + string(b) + "," + string(calls);
        }
    "#)
        .await,
        HostValue::String("true,true,1".into())
    );
}

#[test]
fn missing_values_invalid_inputs_and_wrong_fallback_types_are_compile_errors() {
    for (source, expected) in [
        (
            "fn f(v: Option<number>) number { const n = unwrap(v) or {}; return n; }",
            "must produce a value or leave the function",
        ),
        (
            "fn f(v: Option<number>) number { const n = unwrap(v) or { const x = 1; }; return n; }",
            "must produce a value or leave the function",
        ),
        (
            "fn f(v: Option<number>) number { const n = unwrap(v) or { \"wrong\" }; return n; }",
            "unwrap alternative has type `string`, but the binding is `number`",
        ),
        (
            "fn f(v: Option<number>) number { const n = unwrap(v) or match { None => 0 }; return n; }",
            "has nothing to match on",
        ),
        (
            "fn f(v: number) number { const n = unwrap(v) or { 0 }; return n; }",
            "works on `Option` and `Result`",
        ),
    ] {
        let source = format!("{source}\nfn main() void {{}}");
        let error = compiler::compile(&source, &Hosts::default()).unwrap_err();
        assert!(
            error.contains(expected),
            "expected {expected:?}, got {error}"
        );
    }
}
