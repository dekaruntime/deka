#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;

#[test]
fn duplicate_bindings_are_rejected_before_lowering_in_every_binding_form() {
    for source in [
        "let x = 1; const x = 2;",
        "const x = 1; const x = 2; fn read() number { return x; }",
        "fn f() number { let x = 1; let x = 2; return x; }",
        "fn f(x: number) number { const x = 2; return x; }",
        "const x = 1; const [x, y]: [number, number] = [2, 3];",
        "const [x, y]: [number, number] = [1, 2]; const [x, z]: [number, number] = [3, 4];",
        "let x = 1; const x = unwrap(Some(2)) or { 3 }",
        "const x = 1; export const x = 2;",
    ] {
        let source = format!("{source}\nfn main() void {{}}");
        let error = compiler::compile(&source, &Hosts::default()).unwrap_err();
        assert!(
            error.contains("duplicate binding `x` in this scope"),
            "{source}: {error}"
        );
        assert_eq!(error.matches("duplicate binding `x`").count(), 1, "{error}");
    }
}

#[tokio::test]
async fn nested_shadowing_module_seeds_and_repeated_inference_still_work() {
    let source = r#"
        const later = 1;
        fn read() number { return later; }
        fn main() number {
            let total = 0;
            const later = 2;
            { const later = 5; total = total + later; }
            for (const later of [7]) { total = total + later; }
            const [a, b]: [number, number] = [3, 4];
            const n = unwrap(Some(6)) or { 0 }
            return total + later + a + b + n + read();
        }
    "#;
    let hosts = Hosts::default();
    let program = compiler::compile(source, &hosts).unwrap();
    let program = serde_json::from_str(&serde_json::to_string(&program).unwrap()).unwrap();
    assert_eq!(
        Vm::new(program, hosts).unwrap().run().await.unwrap(),
        HostValue::Number(28.)
    );
}
