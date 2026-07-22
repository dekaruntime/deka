use super::*;

#[test]
fn pipe_single_bare_function() {
    let js = phpx_to_js("function dbl($n: int): int { return $n * 2; }\n$x = 5;\n$y = $x |> dbl;")
        .expect("should compile");
    assert!(
        js.contains("((__phpx_pipe_lhs) => dbl(__phpx_pipe_lhs))"),
        "expected pipe IIFE with bare function, got:\n{}",
        js
    );
}

#[test]
fn pipe_chained() {
    let js = phpx_to_js(
        "function dbl($n: int): int { return $n * 2; }\nfunction inc($n: int): int { return $n + 1; }\n$y = 5 |> dbl |> inc;"
    )
    .expect("should compile");
    assert!(
        js.contains("inc(__phpx_pipe_lhs)"),
        "expected outer inc call in chained pipe, got:\n{}",
        js
    );
    assert!(
        js.contains("dbl(__phpx_pipe_lhs)"),
        "expected inner dbl call in chained pipe, got:\n{}",
        js
    );
}

#[test]
fn pipe_into_arrow_function() {
    let js = phpx_to_js("$y = 5 |> fn($n: int): int => $n * 2;").expect("should compile");
    assert!(
        js.contains("((__phpx_pipe_lhs) => ((n) => (n * 2))(__phpx_pipe_lhs))"),
        "expected pipe into arrow function, got:\n{}",
        js
    );
}

#[test]
fn pipe_into_closure() {
    let js = phpx_to_js("$y = 5 |> function($n: int): int { return $n * 2; };")
        .expect("should compile");
    assert!(
        js.contains("((__phpx_pipe_lhs) => (function(n) {\n"),
        "expected pipe into closure, got:\n{}",
        js
    );
    assert!(
        js.contains("})(__phpx_pipe_lhs))"),
        "expected closure called with pipe lhs, got:\n{}",
        js
    );
}

#[test]
fn pipe_into_variable_callable() {
    let js = phpx_to_js("$f = fn($n: int): int => $n * 2;\n$y = 5 |> $f;")
        .expect("should compile");
    assert!(
        js.contains("((__phpx_pipe_lhs) => f(__phpx_pipe_lhs))"),
        "expected pipe into variable callable, got:\n{}",
        js
    );
}

#[test]
fn pipe_operator_evaluates_correctly() {
    let source = "function dbl($n: int): int { return $n * 2; }\nfunction inc($n: int): int { return $n + 1; }\n$result = 5 |> dbl |> inc;";
    let js = phpx_to_js(source).expect("should compile");
    let script = format!("{}\nconsole.log(globalThis.result);", js);
    match run_node(&script) {
        Err(e) if e.contains("node not available") => return,
        Err(e) => panic!("node error: {e}"),
        Ok(out) => assert_eq!(
            out, "11",
            "expected 5 |> dbl |> inc = 11, got: {out}"
        ),
    }
}

#[test]
fn pipe_operator_no_regression_bitor() {
    let js = phpx_to_js("function f(): int { return 1 | 2; }").expect("should compile");
    assert!(js.contains("|"), "expected bitwise OR, got:\n{}", js);
    assert!(
        !js.contains("__phpx_pipe_lhs"),
        "bitwise OR should not trigger pipe emission, got:\n{}",
        js
    );
}

#[test]
fn pipe_operator_no_regression_logical_or() {
    let js = phpx_to_js("function f(): bool { return true || false; }").expect("should compile");
    assert!(js.contains("||"), "expected logical OR, got:\n{}", js);
    assert!(
        !js.contains("__phpx_pipe_lhs"),
        "logical OR should not trigger pipe emission, got:\n{}",
        js
    );
}

fn run_node(script: &str) -> Result<String, String> {
    use std::process::Command;
    let out = Command::new("node")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|e| format!("node not available: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}
