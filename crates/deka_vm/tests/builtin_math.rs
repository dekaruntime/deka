#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;
fn hosts() -> Hosts {
    let mut hosts = Hosts::default();
    builtin_math::register(&mut hosts).unwrap();
    hosts
}
async fn run(source: &str) -> HostValue {
    let hosts = hosts();
    let program = compiler::compile(source, &hosts).unwrap();
    let program = serde_json::from_slice(&serde_json::to_vec(&program).unwrap()).unwrap();
    let mut vm = Vm::new(program, hosts).unwrap();
    let result = vm.run().await.unwrap();
    assert_eq!(vm.stats().live, 0);
    result
}
#[tokio::test]
async fn closed_pi_module_constants_and_aliases_are_numbers() {
    for module in ["math", "@deka/math"] {
        assert_eq!(
            run(&format!(
                "import {{PI as circle}} from \"{module}\";fn main() number{{return circle*2;}} "
            ))
            .await,
            HostValue::Number(std::f64::consts::TAU)
        );
    }
}
#[tokio::test]
async fn every_total_receiver_method_executes_native_arithmetic() {
    assert_eq!(run(r#"fn main() Array<number> {
        return [(3.7).floor(),(3.2).ceil(),(3.5).round(),(-4.5).abs(),(3.9).trunc(),(-8).sign(),(27).cbrt(),(0).exp(),(0).atan(),(0).sinh(),(0).cosh(),(0).tanh(),(2).max(7),(2).min(7)];
    }"#).await,HostValue::List([3.,4.,4.,4.5,3.,-1.,3.,1.,0.,0.,1.,0.,7.,2.].map(HostValue::Number).to_vec()));
}
#[tokio::test]
async fn every_partial_receiver_method_returns_a_real_option() {
    for (expression, number) in [
        ("(16).sqrt()", 4.),
        ("(1).log()", 0.),
        ("(1).log2()", 0.),
        ("(1).log10()", 0.),
        ("(0).asin()", 0.),
        ("(1).acos()", 0.),
        ("(1).acosh()", 0.),
        ("(0).atanh()", 0.),
        ("(0).sin()", 0.),
        ("(0).cos()", 1.),
        ("(0).tan()", 0.),
        ("(2).pow(10)", 1024.),
    ] {
        assert_eq!(
            run(&format!("fn main() Option<number>{{return {expression};}}")).await,
            HostValue::Option(Some(Box::new(HostValue::Number(number))))
        );
    }
    for expression in [
        "(-1).sqrt()",
        "(-1).log()",
        "(-1).log2()",
        "(-1).log10()",
        "(2).asin()",
        "(-2).acos()",
        "(0).acosh()",
        "(2).atanh()",
        "(1/0).sin()",
        "(1/0).cos()",
        "(1/0).tan()",
        "(-2).pow(0.5)",
    ] {
        assert_eq!(
            run(&format!("fn main() Option<number>{{return {expression};}}")).await,
            HostValue::Option(None)
        );
    }
}
#[tokio::test]
async fn infinities_signed_zero_and_nan_follow_existing_contracts() {
    for (expression, number) in [
        ("(0).log()", f64::NEG_INFINITY),
        ("(10).pow(400)", f64::INFINITY),
        ("(-0).pow(-3)", f64::NEG_INFINITY),
        ("(0/0).pow(0)", 1.),
    ] {
        assert_eq!(
            run(&format!("fn main() Option<number>{{return {expression};}}")).await,
            HostValue::Option(Some(Box::new(HostValue::Number(number))))
        );
    }
    for expression in ["(1).pow(1/0)", "(-1).pow(1/0)"] {
        assert_eq!(
            run(&format!("fn main() Option<number>{{return {expression};}}")).await,
            HostValue::Option(None)
        );
    }
    for expression in ["(-0.5).round()", "(-0).sign()", "(0).min(-0)"] {
        let HostValue::Number(number) =
            run(&format!("fn main() number{{return {expression};}}")).await
        else {
            panic!("expected number")
        };
        assert_eq!(number.to_bits(), (-0.0_f64).to_bits());
    }
    for expression in ["(0/0).min(1)", "(1).max(0/0)"] {
        let HostValue::Number(number) =
            run(&format!("fn main() number{{return {expression};}}")).await
        else {
            panic!("expected number")
        };
        assert!(number.is_nan());
    }
}
#[test]
fn unsupported_exports_wrong_arity_and_unwrapped_partial_values_fail_checking() {
    for source in [
        r#"import {PI} from "math";fn main(){PI();}"#,
        r#"import {E} from "math";fn main(){E;}"#,
        r#"import {floor} from "math";fn main(){floor(1);}"#,
        r#"fn main(){(1).floor(1);}"#,
        r#"fn main(){(1).pow();}"#,
        r#"fn main(){(1).max("x");}"#,
        r#"fn main() number{return (4).sqrt();}"#,
        r#"fn main(){Math.sqrt(4);}"#,
    ] {
        assert!(compiler::compile(source, &hosts()).is_err(), "{source}");
    }
}
#[tokio::test]
async fn user_receiver_extensions_shadow_native_math() {
    assert_eq!(
        run(r#"fn(n number)floor()string{return "user";}fn main()string{return (3.7).floor();}"#)
            .await,
        HostValue::String("user".into())
    );
}
#[tokio::test]
async fn constant_barrels_and_number_methods_survive_deleted_sources() {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("main.ds");
    std::fs::write(
        dir.path().join("constants.ds"),
        r#"export {PI as circle} from "math";"#,
    )
    .unwrap();
    std::fs::write(&main,r#"import {circle as angle} from "./constants.ds";fn main()number{const root=unwrap((16).sqrt()) or{return 0;};return angle+root;}"#).unwrap();
    let hosts = hosts();
    let program = compiler::compile_file(&main, &hosts, Some("main")).unwrap();
    let program = serde_json::from_slice(&serde_json::to_vec(&program).unwrap()).unwrap();
    drop(dir);
    let mut vm = Vm::new(program, hosts).unwrap();
    assert_eq!(
        vm.run().await.unwrap(),
        HostValue::Number(std::f64::consts::PI + 4.)
    );
    assert_eq!(vm.stats().live, 0);
}

#[tokio::test]
async fn nonfinite_numbers_keep_the_existing_language_formatter_spelling() {
    assert_eq!(
        run(r#"fn main() string {
        return string(1/0)+":"+string(-1/0)+":"+string(0/0)+":"+(1/0)+":"+((-1/0)+"suffix");
    }"#)
        .await,
        HostValue::String("Infinity:-Infinity:NaN:Infinity:-Infinitysuffix".into())
    );
    let output = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let capture = output.clone();
    let mut h = hosts();
    h.register(HostOp::new(
        "echo",
        vec![HostType::String],
        HostType::Unit,
        false,
        move |args| {
            let HostValue::String(text) = &args[0] else {
                panic!("typed console sink")
            };
            capture.borrow_mut().push(text.clone());
            HostReply::Ready(Ok(HostValue::Unit))
        },
    ))
    .unwrap();
    let program = compiler::compile("fn main(){console.log([1/0,-1/0]);}", &h).unwrap();
    Vm::new(program, h).unwrap().run().await.unwrap();
    assert_eq!(*output.borrow(), ["[ Infinity, -Infinity ]"]);
}
#[cfg(feature = "ui")]
#[test]
fn declarative_text_nodes_use_the_same_number_formatter_without_a_window() {
    let source = r#"fn App(){return <view><span>{1/0}</span><span>{-1/0}</span></view>;}"#;
    let program = compiler::compile_entry(source, &Hosts::default(), "App").unwrap();
    let session = ui::UiSession::new(program).unwrap();
    fn texts(node: &deka_native_ui::Node, out: &mut Vec<String>) {
        if let Some(text) = &node.text {
            out.push(text.clone());
        }
        for child in &node.children {
            texts(child, out);
        }
    }
    let mut output = vec![];
    texts(session.tree(), &mut output);
    assert_eq!(output, vec!["Infinity", "-Infinity"]);
}
