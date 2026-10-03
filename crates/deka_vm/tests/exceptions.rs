#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;

async fn execute(source: &str) -> Result<HostValue> {
    let hosts = Hosts::default();
    let program = compiler::compile(source, &hosts)?;
    let program = serde_json::from_str(&serde_json::to_string(&program).unwrap()).unwrap();
    Vm::new(program, hosts)?.run().await
}

#[tokio::test]
async fn calls_delegate_success_and_throw_to_the_authored_match() {
    assert_eq!(
        execute(
            r#"
        fn source(fail: boolean) Exception<number, string> {
            if (fail) { return Throw("bad"); } return Ok(7);
        }
        fn delegated(fail: boolean) Exception<number, string> { return source(fail); }
        fn label(fail: boolean) string {
            return "prefix:" + (match delegated(fail) { Ok(v) => string(v), Throw(e) => e });
        }
        fn main() string { return label(false) + "," + label(true); }
    "#
        )
        .await
        .unwrap(),
        HostValue::String("prefix:7,prefix:bad".into())
    );
}

#[tokio::test]
async fn local_and_nested_catches_preserve_payloads_and_scope() {
    assert_eq!(
        execute(
            r#"
        struct Fault { message: string; }
        fn source() Exception<number, Fault> { return Throw(Fault { message: "typed" }); }
        fn main() string {
            const e = "outside";
            let text = "";
            try {
                try { const n = source(); text = string(n); }
                catch (e) { text = e.message; Throw("again"); }
            } catch (e) { text = text + ":" + e; }
            return text + ":" + e;
        }
    "#
        )
        .await
        .unwrap(),
        HostValue::String("typed:again:outside".into())
    );
}

#[tokio::test]
async fn typed_partial_catches_rethrow_other_nominal_structs() {
    assert_eq!(
        execute(
            r#"
        struct TextError { message: string; }
        struct NumberError { code: number; }
        fn source(fail: boolean) Exception<number, TextError | NumberError> {
            if (fail) { return Throw(TextError { message: "text" }); }
            return Throw(NumberError { code: 3 });
        }
        fn partial(fail: boolean) Exception<string, NumberError> {
            try { const n = source(fail); return Ok(string(n)); }
            catch (e: TextError) { return Ok(e.message); }
        }
        fn label(fail: boolean) string {
            return match partial(fail) { Ok(v) => v, Throw(e) => string(e.code) };
        }
        fn main() string { return label(true) + "," + label(false); }
    "#
        )
        .await
        .unwrap(),
        HostValue::String("text,3".into())
    );
}

#[tokio::test]
async fn explicit_conversions_preserve_channels_and_do_not_recapture_handler_throws() {
    assert_eq!(
        execute(
            r#"
        fn source(fail: boolean) Exception<number, string> {
            if (fail) { return Throw("bad"); } return Ok(7);
        }
        fn raised() Exception<number, string> {
            return match source(false) { Ok(v) => Throw("handler"), Throw(e) => Ok(0) };
        }
        fn data(r: Result<number, string>) Exception<number, string> { return Exception.from(r); }
        fn main() string {
            const r = source(true).to_result();
            const s = source(false).to_result();
            return (match r { Ok(v) => string(v), Err(e) => e }) + "," +
                (match s { Ok(v) => string(v), Err(e) => e }) + "," +
                (match data(Ok(2)) { Ok(v) => string(v), Throw(e) => e }) + "," +
                (match data(Err("converted")) { Ok(v) => string(v), Throw(e) => e }) + "," +
                (match raised() { Ok(v) => string(v), Throw(e) => e });
        }
    "#
        )
        .await
        .unwrap(),
        HostValue::String("bad,7,2,converted,handler".into())
    );
}

#[tokio::test]
async fn bodyless_literal_and_wildcard_arms_forward_the_original_payload() {
    assert_eq!(execute(r#"
        fn source(fail: boolean) Exception<number, string> {
            if (fail) { return Throw("wild"); } return Ok(7);
        }
        fn relay(fail: boolean) Exception<number, string> {
            return match source(fail) { Ok(_), Throw("wild"), Throw(_) };
        }
        fn data(r: Result<number, string>) Result<number, string> { return match r { Ok(_), Err(_) }; }
        fn main() string {
            return (match relay(false) { Ok(v) => string(v), Throw(e) => e }) + "," +
                (match relay(true) { Ok(v) => string(v), Throw(e) => e }) + "," +
                (match data(Err("kept")) { Ok(v) => string(v), Err(e) => e });
        }
    "#).await.unwrap(), HostValue::String("7,wild,kept".into()));
}

#[tokio::test]
async fn leaving_try_with_break_continue_or_return_removes_only_its_handlers() {
    assert_eq!(
        execute(
            r#"
        fn source() Exception<number, string> { return Throw("outer"); }
        fn success() number { try { return 1; } catch (e) { return 0; } }
        fn main() string {
            let text = "";
            try {
                for (let i = 0; i < 3; i = i + 1) {
                    try { if (i == 0) { continue; } break; }
                    catch (e) { text = "wrong"; }
                }
                const n = source(); text = string(n);
            } catch (e) { text = e; }
            return text;
        }
    "#
        )
        .await
        .unwrap(),
        HostValue::String("outer".into())
    );
}

#[tokio::test]
async fn nested_ordinary_matches_do_not_acquire_an_exception_wrapper() {
    assert_eq!(
        execute(
            r#"
        fn source() Exception<number, string> { return Ok(7); }
        fn main() string {
            return match (match source() { Ok(v) => v, Throw(e) => 0 }) { 7 => "yes", _ => "no" };
        }
    "#
        )
        .await
        .unwrap(),
        HostValue::String("yes".into())
    );
}

#[tokio::test]
async fn throws_across_await_remain_rooted_while_tasks_are_suspended() {
    assert_eq!(
        execute(
            r#"
        struct Fault { message: string; }
        async fn source() Promise<Exception<number, Fault>> {
            for (let i = 0; i < 700; i = i + 1) { const temporary = [i]; }
            return Throw(Fault { message: "async" });
        }
        async fn main() Promise<string> {
            try { const n = await source(); return string(n); }
            catch (e) { return e.message; }
        }
    "#
        )
        .await
        .unwrap(),
        HostValue::String("async".into())
    );
}

#[tokio::test]
async fn uncaught_throw_is_an_error_and_vm_faults_are_not_language_throws() {
    assert_eq!(
        execute("fn main() Exception<number, string> { return Throw(\"fatal\"); }")
            .await
            .unwrap_err(),
        "uncaught Throw: fatal"
    );
    assert_eq!(execute(r#"fn main() string { try { panic("fault"); } catch (e) { return "wrong"; } return "wrong"; }"#).await.unwrap_err(), "fault");
}

#[test]
fn checker_keeps_consumption_exhaustiveness_and_channels_guarded() {
    for (source, message) in [
        (
            "fn source() Exception<number, string> { return Ok(1); } fn main() void { const later = source(); }",
            "must be consumed at the call site",
        ),
        (
            "fn main() number { return Throw(\"bad\"); }",
            "escaping `Throw` forces",
        ),
        (
            "fn source() Exception<number, string> { return Ok(1); } fn main() number { return match source() { Ok(v) => v }; }",
            "non-exhaustive match: missing Exception::Throw",
        ),
        (
            "fn main() Exception<number, string> { return match Result.Err(\"bad\") { Ok(v) => Ok(1), Err(e) }; }",
            "raise it into the exception channel explicitly",
        ),
    ] {
        let error = compiler::compile(source, &Hosts::default()).unwrap_err();
        assert!(error.contains(message), "{message}: {error}");
    }
}

#[tokio::test]
async fn imported_aliases_and_barrels_keep_declaration_identity() {
    let directory =
        std::env::temp_dir().join(format!("deka-exception-identity-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("left.ds"),
        "struct Fault { message: string; } export { Fault }",
    )
    .unwrap();
    std::fs::write(directory.join("right.ds"), "struct Fault { message: string; } export { Fault }; export fn source() Exception<number, Fault> { return Throw(Fault { message: \"right\" }); }").unwrap();
    std::fs::write(
        directory.join("barrel.ds"),
        "export { Fault, source } from \"./right.ds\"",
    )
    .unwrap();
    std::fs::write(
        directory.join("main.ds"),
        r#"
        import { Fault as Left } from "./left.ds"
        import { Fault as Right, source } from "./barrel.ds"
        fn partial() Exception<string, Right> {
            try { const n = source(); return Ok(string(n)); }
            catch (e: Left) { return Ok("wrong"); }
        }
        fn main() string {
            try { const text = partial(); return text; }
            catch (e: Right) { return e.message; }
        }
    "#,
    )
    .unwrap();
    let program =
        compiler::compile_file(&directory.join("main.ds"), &Hosts::default(), Some("main"));
    std::fs::remove_dir_all(&directory).unwrap();
    let program = program.unwrap();
    let program = serde_json::from_str(&serde_json::to_string(&program).unwrap()).unwrap();
    assert_eq!(
        Vm::new(program, Hosts::default())
            .unwrap()
            .run()
            .await
            .unwrap(),
        HostValue::String("right".into())
    );
}

#[test]
fn imported_defaults_never_read_the_importers_shadowing_bindings() {
    let directory =
        std::env::temp_dir().join(format!("deka-imported-default-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("source.ds"), "const fallback = \"right\"; struct Fault { message: string = fallback; } export { Fault };").unwrap();
    std::fs::write(directory.join("main.ds"), "import { Fault } from \"./source.ds\"\nconst fallback = \"wrong\"; fn main() string { return Fault {}.message; }").unwrap();
    let result =
        compiler::compile_file(&directory.join("main.ds"), &Hosts::default(), Some("main"));
    std::fs::remove_dir_all(&directory).unwrap();
    assert!(
        result
            .unwrap_err()
            .contains("default `message` requires declaration-module evaluation")
    );
}
