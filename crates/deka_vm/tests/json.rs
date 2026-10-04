#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;

async fn execute(program: Program) -> HostValue {
    // JSON schemas and factory closures also survive source-free packaging.
    let program = serde_json::from_str(&serde_json::to_string(&program).unwrap()).unwrap();
    Vm::new(program, Hosts::default())
        .unwrap()
        .run()
        .await
        .unwrap()
}
async fn run(source: &str) -> HostValue {
    execute(compiler::compile(source, &Hosts::default()).unwrap()).await
}

#[tokio::test]
async fn global_and_receiver_json_use_the_same_typed_conversion() {
    assert_eq!(
        run(r#"
        alias Numbers = Array<number>;
        fn main() string {
            const values: Numbers = [1, 2, -3.5];
            const text = JSON.stringify(values);
            const parsed = JSON.parse<Numbers>(text);
            const number = match (parsed) { Ok(xs) => xs.has(2) ? xs[2] : 0, Err(e) => 0 };
            return text + ";" + values.toJSON() + ";" + string(number)
                + ";" + true.toJSON() + ";" + JSON.stringify("a\n雪\"b");
        }
    "#)
        .await,
        HostValue::String("[1,2,-3.5];[1,2,-3.5];-3.5;true;\"a\\n雪\\\"b\"".into())
    );
}

#[tokio::test]
async fn parsed_structs_keep_nominal_identity_and_interface_methods() {
    assert_eq!(
        run(r#"
        struct Person { name: string; }
        interface Greeter { fn greet() string; }
        fn (p Person) greet() string { return "Hello, " + p.name; }
        fn welcome(g: Greeter) string { return g.greet(); }
        fn main() string {
            const encoded = JSON.stringify(Person { name: "Deka" });
            return match (encoded.parseJSON<Person>()) {
                Ok(p) => encoded + ";" + welcome(p) + ";" + p.getType().toString(),
                Err(e) => e,
            };
        }
    "#)
        .await,
        HostValue::String("{\"Person\":{\"name\":\"Deka\"}};Hello, Deka;Person".into())
    );
}

#[tokio::test]
async fn nested_and_embedded_structs_are_hydrated_as_structs() {
    assert_eq!(
        run(r#"
        struct Point { x: number; }
        struct Box { Point; label: string; }
        fn main() string {
            const original = Box { Point: Point { x: 7 }, label: "nested" };
            return match (JSON.parse<Box>(original.toJSON())) {
                Ok(b) => string(b.x) + ";" + b.getType().toString() + ";" + b.label,
                Err(e) => e,
            };
        }
    "#)
        .await,
        HostValue::String("7;Box;nested".into())
    );
}

#[tokio::test]
async fn parse_errors_are_nominal_results_and_do_not_throw() {
    assert_eq!(run(r#"
        struct Point { x: number; }
        fn classify(r: Result<number, string>) string {
            return match (r) { Ok(v) => "ok", Err(e) => "err:" + r.getType().toString() };
        }
        fn main() string {
            const bad = classify(JSON.parse<number>("{"));
            const wrong = classify(JSON.parse<number>("\"wrong\""));
            const nil = classify("null".parseJSON<number>());
            const missing = match (JSON.parse<Point>("{\"Point\":{}}")) { Ok(p) => "bad", Err(e) => "missing" };
            return bad + ";" + wrong + ";" + nil + ";" + missing;
        }
    "#).await, HostValue::String("err:Result;err:Result;err:Result;missing".into()));
}

#[tokio::test]
async fn json_arguments_are_evaluated_once_and_lexical_json_is_not_intrinsic() {
    assert_eq!(
        run(r#"
        fn main() string {
            let calls = 0;
            const text = fn() string { calls = calls + 1; return "7"; };
            const value = match (JSON.parse<number>(text())) { Ok(n) => n, Err(e) => 0 };
            const JSON = { parse: fn(x: number) number { return x + 1; } };
            return string(value) + ";" + string(calls) + ";" + string(JSON.parse(8));
        }
    "#)
        .await,
        HostValue::String("7;1;9".into())
    );
}

#[tokio::test]
async fn namespace_stringify_does_not_run_a_user_to_json_method() {
    assert_eq!(
        run(r#"
        struct X { x: number; }
        fn (x X) toJSON() string { return "custom"; }
        fn main() string {
            const x = X { x: 2 };
            return x.toJSON() + ";" + JSON.stringify(x);
        }
    "#)
        .await,
        HostValue::String("custom;{\"X\":{\"x\":2}}".into())
    );
}

#[tokio::test]
async fn imported_alias_keeps_the_declaring_struct_identity() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("models.ds"),
        r#"
        struct Outer { x: number; }
        export { Outer };
        export fn describe(o: Outer) string { return o.getType().toString() + ":" + string(o.x); }
    "#,
    )
    .unwrap();
    let main = dir.path().join("main.ds");
    std::fs::write(
        &main,
        r#"
        import { Outer as Project } from "./models.ds";
        fn main() string {
            const text = "{\"Project\":{\"x\":7}}";
            return match (JSON.parse<Project>(text)) { Ok(p) => match (p) { Project { x } => p.getType().toString() + ":" + string(x) }, Err(e) => e };
        }
    "#,
    )
    .unwrap();
    let program = compiler::compile_file(&main, &Hosts::default(), Some("main")).unwrap();
    assert_eq!(
        execute(program).await,
        HostValue::String("Project:7".into())
    );
}

#[tokio::test]
async fn imported_struct_keeps_private_nested_factory_identity_and_methods() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("models.ds"), r#"
        struct Inner { x: number; }
        interface Reader { fn read() number; }
        fn (i Inner) read() number { return i.x; }
        struct Outer { inner: Inner; }
        export { Outer };
        fn read(i: Reader) number { return i.read(); }
        export fn describe(o: Outer) string { return o.inner.getType().toString() + ":" + string(read(o.inner)); }
    "#).unwrap();
    let main = dir.path().join("main.ds");
    std::fs::write(
        &main,
        r#"
        import { Outer, describe } from "./models.ds";
        fn main() string {
            const text = "{\"Outer\":{\"inner\":{\"Inner\":{\"x\":7}}}}";
            return match (JSON.parse<Outer>(text)) { Ok(p) => describe(p), Err(e) => e };
        }
    "#,
    )
    .unwrap();
    let program = compiler::compile_file(&main, &Hosts::default(), Some("main")).unwrap();
    assert_eq!(execute(program).await, HostValue::String("Inner:7".into()));
}

#[tokio::test]
async fn structural_records_are_static_json_shapes_without_nominal_wrappers() {
    let text = match run(r#"
        fn main() string {
            const point = { x: 1, active: true, detail: { name: "Deka" } };
            return JSON.stringify(point);
        }
    "#)
    .await
    {
        HostValue::String(text) => text,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&text).unwrap(),
        serde_json::json!({"x":1,"active":true,"detail":{"name":"Deka"}})
    );
}

#[test]
fn invalid_json_contracts_and_undecided_type_mappings_fail_compilation() {
    for source in [
        "const x = JSON.parse(\"7\");",
        "const x = JSON.parse<number>(7);",
        "const x = JSON.parse<number>(\"7\", \"extra\");",
        "const x = JSON.stringify<number>(7);",
        "const x = JSON.stringify(fn() number { return 7; });",
    ] {
        assert!(
            compiler::compile(&format!("{source}\nfn main() void {{}}"), &Hosts::default())
                .is_err(),
            "{source}"
        );
    }
}

#[test]
fn unsupported_json_shapes_have_explicit_compilation_diagnostics() {
    let source = "struct X { x?: number; } const x = JSON.parse<X>(\"{}\"); fn main() void {}";
    let error = compiler::compile(source, &Hosts::default()).unwrap_err();
    assert!(
        error.contains("optional field") && error.contains("not supported"),
        "{error}"
    );
}

#[tokio::test]
async fn barrel_exports_keep_promise_contracts_and_private_json_factories_together() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("joins.ds"),
        "export const all = Promise.all;",
    )
    .unwrap();
    std::fs::write(dir.path().join("models.ds"),r#"
struct Inner { x: number; }
interface Reader { fn read() number; }
fn (i Inner) read() number { return i.x; }
struct Outer { inner: Inner; }
export { Outer };
fn read(i: Reader) number { return i.read(); }
export fn describe(o: Outer) string { return o.inner.getType().toString() + ":" + string(read(o.inner)); }
"#).unwrap();
    std::fs::write(
        dir.path().join("barrel.ds"),
        r#"export { all } from "./joins.ds"; export { Outer, describe } from "./models.ds";"#,
    )
    .unwrap();
    let entry = dir.path().join("main.ds");
    let source = r#"
import { all, Outer, describe } from "./barrel.ds";
async fn ready(value: Outer) Promise<Exception<Outer,string>> { return Ok(value); }
async fn main() Promise<string> {
    const text="{\"Outer\":{\"inner\":{\"Inner\":{\"x\":7}}}}";
    return match JSON.parse<Outer>(text) {
        Ok(value) => match await all([ready(value)]) {
            Ok(values) => values.has(0) ? describe(values[0]) : "empty",
            Throw(error) => "failed:" + error,
        },
        Err(error) => "parse:" + error,
    };
}
"#;
    std::fs::write(&entry, source).unwrap();
    let program = compiler::compile_file(&entry, &Hosts::default(), Some("main")).unwrap();
    assert_eq!(execute(program).await, HostValue::String("Inner:7".into()));
    // Imported aliases must retain the checked Exception contract, not erase
    // it into Array<Outer> merely because JSON metadata was populated too.
    std::fs::write(
        &entry,
        format!(
            r#"{source}
async fn unchecked(value: Outer) Promise<string> {{
    const values = await all([ready(value)]);
    return values.has(0) ? describe(values[0]) : "empty";
}}
"#
        ),
    )
    .unwrap();
    let error = compiler::compile_file(&entry, &Hosts::default(), Some("main")).unwrap_err();
    assert!(error.contains("Exception"), "{error}");
}
