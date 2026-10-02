#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;

async fn run(source: &str) -> Result<HostValue> {
    let hosts = Hosts::default();
    let program = compiler::compile(source, &hosts)?;
    Vm::new(program, hosts)?.run().await
}

#[tokio::test]
async fn template_parts_become_text_the_way_string_does() {
    let value = run(r#"fn main() string { return `n=${3.14} b=${true} s=${"x"} z=${7}`; }"#)
        .await
        .unwrap();
    assert_eq!(value, HostValue::String("n=3.14 b=true s=x z=7".into()));
}

#[tokio::test]
async fn multi_line_templates_keep_their_line_breaks() {
    let value = run("fn main() string { return `line1\nline2`; }")
        .await
        .unwrap();
    assert_eq!(value, HostValue::String("line1\nline2".into()));
}

#[tokio::test]
async fn escaped_interpolation_renders_literally() {
    let value = run(r#"fn main() string { return `\${notInterp}`; }"#)
        .await
        .unwrap();
    assert_eq!(value, HostValue::String("${notInterp}".into()));
}

#[tokio::test]
async fn nested_templates_and_empty_template() {
    let value = run(r#"fn main() string {
            const name = "deka";
            return `${`inner ${name}`}` + "|" + ``;
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::String("inner deka|".into()));
}
