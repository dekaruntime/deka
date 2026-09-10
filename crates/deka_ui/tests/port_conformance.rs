//! Port conformance for the DekaScript ui modules (deka#771 phase 1).
//!
//! `ds/*.ds` is the authoritative source for `ui/jsx`, `ui/router`, `ui/form`,
//! and `ui/suspense`; `emit/*.js` is the pinned output of the pinned compiler
//! (`scripts/dsc-version`). These tests keep the two honest:
//!
//! 1. Emitted output is byte-for-byte what the pinned dsc emits from the
//!    sources, so the shipped JavaScript can never drift from the DekaScript.
//!    (`dsc transpile` on a directory also typechecks every module and
//!    resolves the relative imports between them, so this doubles as the
//!    typecheck gate for the sources.)
//! 2. The public API is typed: consumers passing the wrong thing are
//!    check-time errors naming the offending file, and well-typed consumers
//!    (including the shape the generated api entry uses) compile.

use std::path::{Path, PathBuf};
use std::process::Command;

fn dsc_bin() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("DEKA_DSC") {
        let path = PathBuf::from(&explicit);
        if path.is_file() {
            return Some(path);
        }
        panic!("DEKA_DSC is set but does not exist: {explicit}");
    }
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join("dsc");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// CI installs the pinned dsc before the workspace test step, so the gate
/// always runs there. A local `cargo test` without dsc skips loudly rather
/// than pretending to pass.
fn require_dsc() -> Option<PathBuf> {
    match dsc_bin() {
        Some(dsc) => Some(dsc),
        None => {
            eprintln!(
                "deka_ui: dsc not found (set DEKA_DSC or put dsc on PATH); skipping port conformance"
            );
            None
        }
    }
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn transpile_dir(dsc: &Path, input: &Path, out: &Path) -> Result<(), String> {
    let output = Command::new(dsc)
        .arg("transpile")
        .arg(input)
        .args(["--out", out.to_str().expect("utf-8 temp path")])
        .output()
        .map_err(|err| format!("failed to exec {}: {err}", dsc.display()))?;
    if output.status.success() {
        return Ok(());
    }
    Err(format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ))
}

#[test]
fn emit_matches_pinned_compiler_output() {
    let Some(dsc) = require_dsc() else { return };
    let manifest = manifest_dir();
    let out = tempfile::tempdir().expect("temp dir");
    transpile_dir(&dsc, &manifest.join("ds"), out.path())
        .unwrap_or_else(|e| panic!("ui sources must typecheck and emit: {e}"));
    for name in ["jsx.js", "router.js", "form.js", "suspense.js"] {
        let pinned = std::fs::read_to_string(manifest.join("emit").join(name))
            .unwrap_or_else(|e| panic!("read pinned {name}: {e}"));
        let generated = std::fs::read_to_string(out.path().join(name))
            .unwrap_or_else(|e| panic!("read emitted {name}: {e}"));
        assert_eq!(
            pinned, generated,
            "{name} is stale; regenerate with the pinned dsc (scripts/dsc-version): \
             dsc transpile crates/deka_ui/ds --out crates/deka_ui/emit"
        );
    }
}

/// The point of the port: callers of the ui modules are checked. Under
/// dsc 0.8.0 an imported function's RETURN type is checked strictly (and
/// nominally), which is what these consumers exercise; argument checking
/// against interface-typed parameters of imported functions is not enforced
/// yet (literals are only checked against locally declared interfaces), and
/// that gap is one of the findings recorded on deka#771.
#[test]
fn typed_api_rejects_misuse() {
    let Some(dsc) = require_dsc() else { return };
    let cases: &[(&str, &str)] = &[
        (
            // Form returns Component; a caller that treats it as a number
            // must not compile.
            "form_result_not_number",
            r#"import { Form } from "./form.ds"
export fn misuse() number {
  return Form({})
}
"#,
        ),
        (
            // Suspense returns Option<Array<Component>>; unwrapping is the
            // caller's job (the renderer intercepts the tag instead).
            "suspense_result_not_component",
            r#"import { Suspense } from "./suspense.ds"
export fn misuse() Component {
  return Suspense({})
}
"#,
        ),
        (
            // runApiRouter returns RouterResponse, not a node.
            "router_result_not_component",
            r#"import { runApiRouter } from "./router.ds"
export fn misuse() Component {
  return runApiRouter({ url: "u", pathname: "/", method: "GET", headers: { accept: "a" }, body: "b" }, {})
}
"#,
        ),
    ];
    for (name, source) in cases {
        let diagnostic = check_consumer(&dsc, name, source)
            .unwrap_or_else(|| panic!("{name}: misuse must be a check-time error"));
        assert!(
            diagnostic.contains(&format!("{name}.ds")),
            "{name}: diagnostic must name the offending file:\n{diagnostic}"
        );
        assert!(
            diagnostic.contains(':'),
            "{name}: diagnostic must carry a line:col span:\n{diagnostic}"
        );
    }
}

/// The positive half: consumers using the modules the way components and the
/// generated api entry do must typecheck and emit.
#[test]
fn typed_api_accepts_well_formed_consumers() {
    let Some(dsc) = require_dsc() else { return };
    let component_consumer = r#"import { jsx, jsxs, Fragment, isComponentNode } from "./jsx.ds"
import { Form } from "./form.ds"
import { Suspense } from "./suspense.ds"

interface GreetingProps {
  name: string
}

fn Greeting(props: GreetingProps) Component {
  return jsx("h1", { name: props.name }, Some("Hello"))
}

export const page = jsxs(Fragment, {}, [
  jsx(Greeting, { name: "Deka" }),
  jsx(Form, { action: "/api/cart", method: "post" }),
  jsx(Suspense, {}),
])

export fn checkNode(node: Component) boolean {
  return isComponentNode(node)
}
"#;
    if let Some(diagnostic) = check_consumer(&dsc, "component_consumer", component_consumer) {
        panic!("component consumer must typecheck:\n{diagnostic}");
    }

    // Mirrors the generated api/worker entry: the entry declares its own
    // request/response interfaces (dsc 0.8.0 cannot export interfaces, so
    // ui/router keeps its types module-local and interface identity is
    // nominal), and the response crosses the boundary through the same
    // `unsafe` + match idiom the generator emits.
    let api_consumer = r#"import { runApiRouter } from "./router.ds"

interface RequestHeaders {
  accept: string
}
interface Request {
  url: string
  pathname: string
  method: string
  headers: RequestHeaders
  body: string
}
interface ResponseHeaders {
  location: string
}
interface Response {
  status: number
  body: string
  headers: ResponseHeaders
}

export fn App(request: Request) Response {
  const boxed = unsafe { runApiRouter({ url: request.url, pathname: request.pathname, method: request.method, headers: request.headers, body: request.body }, {}) }
  return match (boxed) {
    Ok(r) => r,
    Err(_) => { status: 500, body: "Internal Server Error", headers: { location: "" } },
  }
}
"#;
    if let Some(diagnostic) = check_consumer(&dsc, "api_consumer", api_consumer) {
        panic!("api consumer must typecheck:\n{diagnostic}");
    }
}

/// Copy the ported sources plus one consumer into an isolated directory and
/// compile the whole directory (which typechecks and resolves the relative
/// imports). Returns Some(diagnostics) on failure.
fn check_consumer(dsc: &Path, name: &str, consumer: &str) -> Option<String> {
    let dir = tempfile::tempdir().expect("temp dir");
    for entry in std::fs::read_dir(manifest_dir().join("ds")).expect("read ds dir") {
        let entry = entry.expect("dir entry");
        std::fs::copy(entry.path(), dir.path().join(entry.file_name())).expect("copy source");
    }
    std::fs::write(dir.path().join(format!("{name}.ds")), consumer).expect("write consumer");
    let out = dir.path().join("out");
    match transpile_dir(dsc, dir.path(), &out) {
        Ok(()) => None,
        Err(diagnostic) => Some(diagnostic),
    }
}
