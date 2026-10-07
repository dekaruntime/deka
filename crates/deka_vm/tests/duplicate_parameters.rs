#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;

#[test]
fn every_function_form_rejects_names_repeated_across_parameters() {
    for source in [
        "fn f(x: number, x: number) number { return x; }",
        "export fn f(x: number, x: number) number { return x; }",
        "async fn f(x: number, x: number) number { return x; }",
        "fn f<T>(x: T, x: T) T { return x; }",
        "struct Box { n: number; } fn (b Box) f(x: number, x: number) number { return x; }",
        "const f = fn(x: number, x: number) number { return x; };",
        "const f = (x: number, x: number) => x;",
        "const f: fn(number, number) number = (x, x) => x;",
        "fn f(x: number, [x, y]: [number, number]) number { return x + y; }",
        "fn f([x, y]: [number, number], x: number) number { return x + y; }",
        "fn f([x, y]: [number, number], [x, z]: [number, number]) number { return x + y + z; }",
    ] {
        let source = format!("{source}\nfn main() void {{}}");
        let error = compiler::compile(&source, &Hosts::default()).unwrap_err();
        assert!(
            error.contains("duplicate parameter `x`"),
            "{source}: {error}"
        );
        assert_eq!(
            error.matches("duplicate parameter `x`").count(),
            1,
            "{error}"
        );
    }
}

#[tokio::test]
async fn parameters_can_shadow_outer_names_and_functions_keep_their_values() {
    let source = r#"
        const x = 100;
        fn f(f: number, x: number) number { return f + x; }
        fn pair([x, y]: [number, number], z: number) number { return x + y + z; }
        struct Box { n: number; }
        fn (b Box) add(x: number, y: number) number { return b.n + x + y; }
        fn main() number {
            const literal = fn(x: number, y: number) number { return x + y; };
            const arrow = (x: number, y: number) => x + y;
            return f(1, 2) + pair([3, 4], 5) + Box { n: 6 }.add(7, 8)
                + literal(9, 10) + arrow(11, 12) + x;
        }
    "#;
    let hosts = Hosts::default();
    let program = compiler::compile(source, &hosts).unwrap();
    let program = serde_json::from_str(&serde_json::to_string(&program).unwrap()).unwrap();
    assert_eq!(
        Vm::new(program, hosts).unwrap().run().await.unwrap(),
        HostValue::Number(178.)
    );
}

#[test]
fn duplicate_tuple_names_keep_the_existing_diagnostic() {
    let error = compiler::compile(
        "fn f([x, x]: [number, number]) number { return x; } fn main() void {}",
        &Hosts::default(),
    )
    .unwrap_err();
    assert!(error.contains("duplicate tuple binding `x`"), "{error}");
    assert!(!error.contains("duplicate parameter"), "{error}");
}
