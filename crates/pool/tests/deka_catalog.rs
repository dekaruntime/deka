//! RFD 21 `deka.*` catalog acceptance tests (deka#754).
//!
//! End-to-end through `IsolatePool` with real on-disk projects compiled by
//! the pinned `dsc` (0.8.2, see `scripts/dsc-version`): stdlib packages
//! calling `safe { deka.kind.method(...) }` (non-throwing, declared type)
//! and `unsafe { deka.kind.method(...) }` (throwing → `Result`), plus the
//! closed-catalog enforcement: unknown helpers, wrong arity, and user-package
//! use are source diagnostics. Also proves the catalog is not published on
//! `globalThis`.
//!
//! dsc types the ambient `deka` global as `Infer`, so a `safe` result must be
//! bound to an annotated `const` (its declared type) before it can be matched
//! or converted — the tests below model that discipline.
//!
//! Tests that need dsc skip (print + early return) when no compiler is
//! available; Rust-only assertions are unconditional.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use pool::{ExecutionMode, HandlerKey, IsolatePool, PoolConfig, RequestData, RequestParts};

const EMPTY_DEKA_LOCK: &str = r#"{"lockfileVersion":1,"packages":{}}"#;

/// Locate the pinned dsc once and point `DEKA_DSC` at it so pool worker
/// threads resolve the same binary (same provisioning as host_grants tests).
fn ensure_dsc() -> bool {
    static DSC: OnceLock<Option<PathBuf>> = OnceLock::new();
    let resolved = DSC
        .get_or_init(|| {
            if std::env::var_os("DEKA_NO_DSC").is_some() {
                return None;
            }
            if let Ok(path) = std::env::var("DEKA_DSC")
                && Path::new(&path).is_file()
            {
                return Some(PathBuf::from(path));
            }
            let local = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp/dsc");
            if local.is_file() {
                return Some(local);
            }
            runtime_core::dsc::find_dsc().ok().flatten()
        })
        .clone();
    match resolved {
        Some(path) => {
            // SAFETY: test-binary-wide, idempotent value; every dsc-gated test
            // in this file agrees on the same binary.
            unsafe { std::env::set_var("DEKA_DSC", &path) };
            true
        }
        None => false,
    }
}

fn write(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("mkdir fixture");
    }
    std::fs::write(path, body).expect("write fixture");
}

/// Write a `deka.lock` entry pinning `dep` (a package directory under
/// `project/ds_modules`) exactly as `deka install` would.
fn pin_lock(project: &Path, dep: &Path, name: &str) {
    let integrity = deka_host::integrity::compute_package_integrity(dep).expect("package integrity");
    let lock = serde_json::json!({
        "lockfileVersion": 1,
        "packages": {
            name: [
                format!("{name}@1.0.0"),
                format!("linkhash:{name}"),
                {
                    "repo": "https://github.com/dekaruntime/deka.git",
                    "gitRef": "v1.0.0",
                    "source": "deka.gg",
                    "dependencies": [],
                    "moduleGraph": { "algo": "sha256", "hash": integrity.module_graph },
                    "fsGraph": { "algo": "sha256", "hash": integrity.fs_graph }
                },
                ""
            ]
        }
    });
    write(
        project,
        "deka.lock",
        &serde_json::to_string_pretty(&lock).expect("lock json"),
    );
}

fn catalog_pool() -> IsolatePool {
    let config = PoolConfig {
        num_workers: 1,
        max_isolates_per_worker: 2,
        idle_timeout_secs: 30,
        enable_metrics: false,
        enable_code_cache: false,
        request_timeout_ms: 10_000,
        queue_timeout_ms: 10_000,
        ..PoolConfig::default()
    };
    IsolatePool::new(config, Arc::new(platform_server::extensions_for_php_server))
}

fn module_request(entry: &Path, root: &Path) -> RequestData {
    RequestData {
        handler_code: String::new(),
        handler_entry: Some(entry.to_string_lossy().into_owned()),
        module_root: Some(root.to_string_lossy().into_owned()),
        request_value: serde_json::Value::Null,
        request_parts: Some(RequestParts {
            url: "http://localhost/".to_string(),
            method: "GET".to_string(),
            headers: Vec::new(),
            body: None,
        }),
        mode: ExecutionMode::Request,
        security: None,
    }
}

fn body_of(response: &pool::IsolateResponse) -> String {
    assert!(response.success, "execution failed: {:?}", response.error);
    response
        .result
        .as_ref()
        .expect("response result")
        .get("body")
        .and_then(serde_json::Value::as_str)
        .expect("response body")
        .to_string()
}

fn error_of(response: &pool::IsolateResponse) -> String {
    assert!(!response.success, "execution must fail");
    response.error.clone().unwrap_or_default()
}

/// An official `@deka/*` workspace root whose entry exercises the whole
/// bytes family through `safe`, the strict decode through `unsafe`, and the
/// failure paths — every failure is a value the program handles, never a
/// fabricated fallback.
#[tokio::test]
async fn safe_returns_declared_type_and_failures_are_values() {
    if !ensure_dsc() {
        println!("SKIP safe_returns_declared_type_and_failures_are_values: dsc unavailable");
        return;
    }
    let project = tempfile::tempdir().expect("tempdir");
    write(project.path(), "deka.json", r#"{"name":"@deka/catalogtest"}"#);
    write(project.path(), "deka.lock", EMPTY_DEKA_LOCK);
    write(
        project.path(),
        "main.ds",
        r#"fn lenOf(b: bytes) number {
  return safe { deka.bytes.len(b) }
}

fn hexOf(b: bytes) string {
  return safe { deka.bytes.to_hex(b) }
}

export fn app(req: string) string {
  const b: bytes = safe { deka.bytes.from_string("hello") }
  const len: number = safe { deka.bytes.len(b) }
  const secondOpt: Option<number> = safe { deka.bytes.get(b, 1) }
  const second = match (secondOpt) {
    Some(v) => v,
    None => -1,
  }
  const oobOpt: Option<number> = safe { deka.bytes.get(b, 99) }
  const oob = match (oobOpt) {
    Some(_) => 999,
    None => -1,
  }
  const hexed: string = safe { deka.bytes.to_hex(b) }
  const backOpt: Option<bytes> = safe { deka.bytes.from_hex(hexed) }
  const back = match (backOpt) {
    Some(v) => hexOf(v),
    None => "none",
  }
  const badHexOpt: Option<bytes> = safe { deka.bytes.from_hex("zz") }
  const badHex: string = match (badHexOpt) {
    Some(_) => "fabricated",
    None => "none",
  }
  const oddHexOpt: Option<bytes> = safe { deka.bytes.from_hex("abc") }
  const oddHex: string = match (oddHexOpt) {
    Some(_) => "fabricated",
    None => "none",
  }
  const b64: string = safe { deka.bytes.to_base64(b) }
  const b64Opt: Option<bytes> = safe { deka.bytes.from_base64(b64) }
  const b64back = match (b64Opt) {
    Some(v) => hexOf(v),
    None => "none",
  }
  const badB64Opt: Option<bytes> = safe { deka.bytes.from_base64("!!") }
  const badB64: string = match (badB64Opt) {
    Some(_) => "fabricated",
    None => "none",
  }
  const twice: bytes = safe { deka.bytes.concat(b, b) }
  const joined: string = safe { deka.bytes.to_hex(twice) }
  const strict: Result<string, string> = unsafe { deka.bytes.to_string(b) }
  const validUtf8 = match (strict) {
    Ok(s) => s,
    Err(_) => "decode-failed",
  }
  const ff: bytes = safe { deka.bytes.from_array([255]) }
  const broken: Result<string, string> = unsafe { deka.bytes.to_string(ff) }
  const invalidUtf8 = match (broken) {
    Ok(_) => "lossy",
    Err(_) => "err",
  }
  const parsed: Result<string, string> = unsafe { deka.json.parse("{\"a\":1}") }
  const parsedOk = match (parsed) {
    Ok(_) => "parsed",
    Err(_) => "err",
  }
  const brokenJson: Result<string, string> = unsafe { deka.json.parse("{") }
  const brokenJsonOk = match (brokenJson) {
    Ok(_) => "fabricated",
    Err(_) => "err",
  }
  const validJson: boolean = safe { deka.json.validate("{\"a\":1}") }
  const invalidJson: boolean = safe { deka.json.validate("{") }
  const before: number = safe { deka.time.now() }
  const nowOk = before > 0
  safe { deka.io.echo("catalog-echo-line") }
  return string(len) + ":" + string(second) + ":" + string(oob) + ":" + hexed + ":" + back + ":" + badHex + ":" + oddHex + ":" + b64back + ":" + badB64 + ":" + joined + ":" + validUtf8 + ":" + invalidUtf8 + ":" + parsedOk + ":" + brokenJsonOk + ":" + string(validJson) + ":" + string(invalidJson) + ":" + (nowOk ? "now-ok" : "now-bad")
}
"#,
    );
    let entry = project.path().join("main.ds");

    let pool = catalog_pool();
    let response = pool
        .execute(
            HandlerKey::new("catalog_safe_happy_path"),
            module_request(&entry, project.path()),
        )
        .await
        .expect("pool execution");
    // len=5, second='e'=101, oob=None(-1), hex roundtrip, invalid decoders
    // are None (not fabricated), concat, strict decode both directions,
    // json parse/validate, time.now.
    assert_eq!(
        body_of(&response),
        "5:101:-1:68656c6c6f:68656c6c6f:none:none:68656c6c6f:none:68656c6c6f68656c6c6f:hello:err:parsed:err:true:false:now-ok"
    );
}

/// The catalog is not an ambient global: from application code (raw JS-mode
/// is the only place an app may inspect), `globalThis.deka` carries no
/// catalog kinds, and the internal surface stays frozen.
#[tokio::test]
async fn catalog_is_not_published_on_global_this() {
    if !ensure_dsc() {
        println!("SKIP catalog_is_not_published_on_global_this: dsc unavailable");
        return;
    }
    let project = tempfile::tempdir().expect("tempdir");
    write(project.path(), "deka.json", r#"{"name":"my-app"}"#);
    write(project.path(), "deka.lock", EMPTY_DEKA_LOCK);
    write(
        project.path(),
        "main.ds",
        r#"export fn app(req: string) string {
  const probe: Result<string, string> = unsafe {
    typeof globalThis.deka === "undefined"
      ? "no-deka:no-deka"
      : typeof globalThis.deka.bytes + ":" + typeof globalThis.deka.json
      + ":" + typeof globalThis[Symbol.for("deka.host.internal")]
      + ":" + Object.isFrozen(globalThis[Symbol.for("deka.host.internal")])
  }
  return match (probe) {
    Ok(s) => s,
    Err(e) => e,
  }
}
"#,
    );
    let entry = project.path().join("main.ds");

    let pool = catalog_pool();
    let response = pool
        .execute(
            HandlerKey::new("catalog_no_global_publication"),
            module_request(&entry, project.path()),
        )
        .await
        .expect("pool execution");
    assert_eq!(body_of(&response), "undefined:undefined:object:true");
}

/// Unknown kind, unknown method, wrong arity, throwing helpers under `safe`,
/// and bare catalog calls are source diagnostics with file:line:col — the
/// project refuses to load before any user code runs.
#[tokio::test]
async fn closed_catalog_rejects_unknown_helpers_and_wrong_arity() {
    if !ensure_dsc() {
        println!("SKIP closed_catalog_rejects_unknown_helpers_and_wrong_arity: dsc unavailable");
        return;
    }
    for (name, source, needle) in [
        (
            "unknown_kind",
            "export fn app(req: string) string {\n  const n: number = safe { deka.bogus.len(req) }\n  return string(n)\n}\n",
            "unknown `deka` kind `bogus`",
        ),
        (
            "unknown_method",
            "export fn app(req: string) string {\n  const n: number = safe { deka.bytes.bogus(req) }\n  return string(n)\n}\n",
            "unknown `deka.bytes` helper `bogus`",
        ),
        (
            "wrong_arity",
            "export fn app(req: string) string {\n  const n: number = safe { deka.bytes.get(req) }\n  return string(n)\n}\n",
            "expected 2 argument(s), found 1",
        ),
        (
            "throwing_under_safe",
            "export fn app(req: string) string {\n  const s: string = safe { deka.bytes.to_string(req) }\n  return s\n}\n",
            "may throw",
        ),
        (
            "bare_call",
            "export fn app(req: string) string {\n  const n: number = deka.bytes.len(req)\n  return string(n)\n}\n",
            "must appear as the body of `safe { }` or `unsafe { }`",
        ),
    ] {
        let project = tempfile::tempdir().expect("tempdir");
        write(project.path(), "deka.json", r#"{"name":"@deka/catalogneg"}"#);
        write(project.path(), "deka.lock", EMPTY_DEKA_LOCK);
        write(project.path(), "main.ds", source);
        let entry = project.path().join("main.ds");

        let pool = catalog_pool();
        let response = pool
            .execute(
                HandlerKey::new(format!("catalog_neg_{name}")),
                module_request(&entry, project.path()),
            )
            .await
            .expect("pool execution");
        let error = error_of(&response);
        assert!(error.contains(needle), "{name}: expected {needle:?} in {error}");
        assert!(
            error.contains("main.ds:2:"),
            "{name}: diagnostic should carry a source location: {error}"
        );
    }
}

/// `safe` / `unsafe` catalog calls are stdlib-only. An application project
/// root may not use them even though dsc itself would accept the ambient
/// `deka` global — the loader's scan rejects it with a source diagnostic.
#[tokio::test]
async fn user_package_catalog_use_is_a_source_diagnostic() {
    if !ensure_dsc() {
        println!("SKIP user_package_catalog_use_is_a_source_diagnostic: dsc unavailable");
        return;
    }
    for (name, source) in [
        (
            "user_safe",
            "export fn app(req: string) string {\n  const n: number = safe { deka.bytes.len(req) }\n  return string(n)\n}\n",
        ),
        (
            "user_unsafe",
            "export fn app(req: string) string {\n  const r: Result<unknown, string> = unsafe { deka.json.parse(req) }\n  return \"x\"\n}\n",
        ),
        (
            "user_bare",
            "export fn app(req: string) string {\n  const n: number = deka.bytes.len(req)\n  return string(n)\n}\n",
        ),
    ] {
        let project = tempfile::tempdir().expect("tempdir");
        write(project.path(), "deka.json", r#"{"name":"my-app"}"#);
        write(project.path(), "deka.lock", EMPTY_DEKA_LOCK);
        write(project.path(), "main.ds", source);
        let entry = project.path().join("main.ds");

        let pool = catalog_pool();
        let response = pool
            .execute(
                HandlerKey::new(format!("catalog_user_{name}")),
                module_request(&entry, project.path()),
            )
            .await
            .expect("pool execution");
        let error = error_of(&response);
        assert!(
            error.contains("stdlib-only"),
            "{name}: expected stdlib-only diagnostic in {error}"
        );
    }
}

/// An unofficial dependency (`@acme/evil`, not `@deka/*`) that calls the
/// catalog is refused the same way — officialness is decided by the
/// registry identity, not by where the package sits.
#[tokio::test]
async fn unofficial_dependency_catalog_use_is_rejected() {
    if !ensure_dsc() {
        println!("SKIP unofficial_dependency_catalog_use_is_rejected: dsc unavailable");
        return;
    }
    let project = tempfile::tempdir().expect("tempdir");
    write(
        project.path(),
        "deka.json",
        r#"{"name":"my-app","dependencies":{"@acme/evil":"1.0.0"}}"#,
    );
    let dep = project.path().join("ds_modules/@acme/evil");
    write(&dep, "deka.json", r#"{"name":"@acme/evil","version":"1.0.0"}"#);
    write(
        &dep,
        "index.ds",
        "export fn steal() string {\n  const n: number = safe { deka.bytes.len(deka.bytes.from_string(\"x\")) }\n  return string(n)\n}\n",
    );
    pin_lock(project.path(), &dep, "@acme/evil");
    write(
        project.path(),
        "main.ds",
        "import { steal } from \"@acme/evil\"\nexport fn app(req: string) string {\n  return steal()\n}\n",
    );
    let entry = project.path().join("main.ds");

    let pool = catalog_pool();
    let response = pool
        .execute(
            HandlerKey::new("catalog_unofficial_dep"),
            module_request(&entry, project.path()),
        )
        .await
        .expect("pool execution");
    let error = error_of(&response);
    assert!(
        error.contains("stdlib-only"),
        "expected stdlib-only diagnostic in {error}"
    );
    assert!(
        error.contains("index.ds"),
        "diagnostic should name the dependency source: {error}"
    );
}

/// A stdlib dependency (`ds_modules/@deka/bytestest`) imported by an
/// application: its `safe` calls compile through the staged lowering, and
/// the app observes the package's returned values. Catalog authority comes
/// from the `@deka/*` identity — no grant table entry is involved.
#[tokio::test]
async fn stdlib_dependency_safe_calls_serve_app_imports() {
    if !ensure_dsc() {
        println!("SKIP stdlib_dependency_safe_calls_serve_app_imports: dsc unavailable");
        return;
    }
    let project = tempfile::tempdir().expect("tempdir");
    write(
        project.path(),
        "deka.json",
        r#"{"name":"my-app","dependencies":{"@deka/bytestest":"1.0.0"}}"#,
    );
    let dep = project.path().join("ds_modules/@deka/bytestest");
    write(&dep, "deka.json", r#"{"name":"@deka/bytestest","version":"1.0.0"}"#);
    write(
        &dep,
        "index.ds",
        r#"fn hexOf(b: bytes) string {
  return safe { deka.bytes.to_hex(b) }
}

export fn describe(s: string) string {
  const b: bytes = safe { deka.bytes.from_string(s) }
  const n: number = safe { deka.bytes.len(b) }
  return hexOf(b) + ":" + string(n)
}
"#,
    );
    pin_lock(project.path(), &dep, "@deka/bytestest");
    write(
        project.path(),
        "main.ds",
        "import { describe } from \"@deka/bytestest\"\nexport fn app(req: string) string {\n  return describe(\"deka\")\n}\n",
    );
    let entry = project.path().join("main.ds");

    let pool = catalog_pool();
    let response = pool
        .execute(
            HandlerKey::new("catalog_stdlib_dep"),
            module_request(&entry, project.path()),
        )
        .await
        .expect("pool execution");
    assert_eq!(body_of(&response), "64656b61:4");
}

/// The safe classification is load-bearing: a `safe` helper that threw would
/// escape as a raw exception (no try/catch is emitted), so the catalog pins
/// every safe entry total over its declared argument types. Property-style
/// sweep of adversarial-but-typed inputs through the real pipeline.
#[tokio::test]
async fn safe_helpers_do_not_throw_on_typed_edge_inputs() {
    if !ensure_dsc() {
        println!("SKIP safe_helpers_do_not_throw_on_typed_edge_inputs: dsc unavailable");
        return;
    }
    let project = tempfile::tempdir().expect("tempdir");
    write(project.path(), "deka.json", r#"{"name":"@deka/catalogedge"}"#);
    write(project.path(), "deka.lock", EMPTY_DEKA_LOCK);
    write(
        project.path(),
        "main.ds",
        r#"fn lenOf(b: bytes) number {
  return safe { deka.bytes.len(b) }
}

export fn app(req: string) string {
  const empty: bytes = safe { deka.bytes.from_string("") }
  const emptyLen: number = safe { deka.bytes.len(empty) }
  const negOpt: Option<number> = safe { deka.bytes.get(empty, -3) }
  const neg = match (negOpt) {
    Some(_) => "some",
    None => "none",
  }
  const emptyHex: string = safe { deka.bytes.to_hex(empty) }
  const emptyB64: string = safe { deka.bytes.to_base64(empty) }
  const emptySlice: bytes = safe { deka.bytes.slice(empty, 0) }
  const emptySliceHex: string = safe { deka.bytes.to_hex(emptySlice) }
  const emptyFromHex: Option<bytes> = safe { deka.bytes.from_hex("") }
  const emptyFromHexLen = match (emptyFromHex) {
    Some(v) => string(lenOf(v)),
    None => "none",
  }
  const emptyFromB64: Option<bytes> = safe { deka.bytes.from_base64("") }
  const emptyFromB64Len = match (emptyFromB64) {
    Some(v) => string(lenOf(v)),
    None => "none",
  }
  const pad: Option<bytes> = safe { deka.bytes.from_base64("TQ==") }
  const padHex = match (pad) {
    Some(v) => safe { deka.bytes.to_hex(v) },
    None => "none",
  }
  const concatBytes: bytes = safe { deka.bytes.concat(empty, empty) }
  const concat: string = safe { deka.bytes.to_hex(concatBytes) }
  return string(emptyLen) + ":" + neg + ":" + emptyHex + ":" + emptyB64 + ":" + emptySliceHex + ":" + emptyFromHexLen + ":" + emptyFromB64Len + ":" + padHex + ":" + concat
}
"#,
    );
    let entry = project.path().join("main.ds");

    let pool = catalog_pool();
    let response = pool
        .execute(
            HandlerKey::new("catalog_safe_edges"),
            module_request(&entry, project.path()),
        )
        .await
        .expect("pool execution");
    assert_eq!(body_of(&response), "0:none::::0:0:4d:");
}
