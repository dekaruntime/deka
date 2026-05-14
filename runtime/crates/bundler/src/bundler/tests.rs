use super::*;

#[test]
fn test_is_valid_identifier() {
    assert!(is_valid_identifier("foo"));
    assert!(is_valid_identifier("_foo"));
    assert!(is_valid_identifier("$foo"));
    assert!(!is_valid_identifier("1foo"));
    assert!(!is_valid_identifier("foo-bar"));
    assert!(!is_valid_identifier(""));
}

#[test]
fn test_to_camel_case() {
    assert_eq!(to_camel_case("foo-bar"), "fooBar");
    assert_eq!(to_camel_case("foo_bar"), "fooBar");
    assert_eq!(to_camel_case("foo bar"), "fooBar");
    assert_eq!(to_camel_case("Foo"), "Foo");
    assert_eq!(to_camel_case("foo--bar"), "fooBar");
}

#[test]
fn test_append_named_exports() {
    let mut lines = Vec::new();
    append_named_exports(&mut lines, "foo").unwrap();
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("export const foo"));

    let mut lines = Vec::new();
    append_named_exports(&mut lines, "foo-bar").unwrap();
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("export const fooBar"));

    let mut lines = Vec::new();
    append_named_exports(&mut lines, "1bad").unwrap();
    assert!(lines.is_empty());
}

struct SimpleVirtualSource {
    entry: PathBuf,
    code: String,
}

impl VirtualSource for SimpleVirtualSource {
    fn load_virtual(&self, path: &Path) -> Result<Option<String>, String> {
        if path == self.entry {
            Ok(Some(self.code.clone()))
        } else {
            Ok(None)
        }
    }
}

fn make_tmp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("deka_bundler_test_{}", name));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create test dir");
    dir
}

#[test]
fn bundle_produces_valid_js() {
    let tmp = make_tmp_dir("valid_js");
    let entry = tmp.join("entry.js");
    std::fs::write(&entry, "export const x = 42;\n").expect("write entry");
    let provider = Arc::new(SimpleVirtualSource {
        entry: entry.clone(),
        code: "export const x = 42;\n".to_string(),
    });
    let result = bundle_virtual_entry(
        &entry,
        BundleOptions {
            project_root: tmp.clone(),
            minify: false,
            iife: false,
            stdlib_path: None,
        },
        provider,
    )
    .expect("bundle should succeed");
    assert!(result.contains("42"), "expected value in bundle: {}", result);
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn bundle_with_iife_wrapping() {
    let tmp = make_tmp_dir("iife");
    let entry = tmp.join("entry.js");
    std::fs::write(&entry, "const msg = 'hello';\n").expect("write entry");
    let provider = Arc::new(SimpleVirtualSource {
        entry: entry.clone(),
        code: "const msg = 'hello';\n".to_string(),
    });
    let result = bundle_virtual_entry(
        &entry,
        BundleOptions {
            project_root: tmp.clone(),
            minify: false,
            iife: true,
            stdlib_path: None,
        },
        provider,
    )
    .expect("bundle should succeed");
    assert!(
        result.contains("function") || result.contains("hello"),
        "expected IIFE or content in bundle: {}",
        result
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn bundle_iife_strips_exports_and_await() {
    let tmp = make_tmp_dir("iife_exports");
    let entry = tmp.join("entry.js");
    let source = r#"
export const phpxBuildMode = "subset-ast";
export const phpxTargetSemantics = "js";
function App(req) { return { status: 200, body: "ok" }; }
const __phpx_main = async () => {
let app = App;
globalThis.app = app;
};
await __phpx_main();
"#;
    std::fs::write(&entry, source).expect("write entry");
    let provider = Arc::new(SimpleVirtualSource {
        entry: entry.clone(),
        code: source.to_string(),
    });
    let result = bundle_virtual_entry(
        &entry,
        BundleOptions {
            project_root: tmp.clone(),
            minify: true,
            iife: true,
            stdlib_path: None,
        },
        provider,
    )
    .expect("bundle should succeed");
    // IIFE mode should NOT contain export statements
    assert!(
        !result.contains("export "),
        "IIFE bundle should not contain export statements: {}",
        result
    );
    // Must start with `(async function` for pre-bundled IIFE detection
    let trimmed = result.trim_start();
    assert!(
        trimmed.starts_with("(async function"),
        "IIFE bundle should start with (async function: starts with {:?}",
        &trimmed[..60.min(trimmed.len())]
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn bundle_minified_output_is_valid() {
    let tmp = make_tmp_dir("minified");
    let entry = tmp.join("entry.js");
    std::fs::write(&entry, "export const greeting = 'hello world';\n").expect("write entry");
    let provider = Arc::new(SimpleVirtualSource {
        entry: entry.clone(),
        code: "export const greeting = 'hello world';\n".to_string(),
    });
    let result = bundle_virtual_entry(
        &entry,
        BundleOptions {
            project_root: tmp.clone(),
            minify: true,
            iife: false,
            stdlib_path: None,
        },
        provider,
    )
    .expect("minified bundle should succeed");
    assert!(
        result.contains("hello world"),
        "expected string in minified bundle: {}",
        result
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

/// Regression: `if (cond) x = y;` must not be rewritten into
/// `cond && x = y` (invalid: assignment LHS not parenthesized).
/// Root cause: `compress_if_stmt_as_expr` in
/// swc_ecma_minifier/src/compress/pure/bools.rs runs whenever
/// `conditionals || bools` is enabled; the bundler disables both.
#[test]
fn bundle_minified_preserves_if_assignment() {
    let tmp = make_tmp_dir("if_assign");
    let entry = tmp.join("entry.js");
    // The minifier used to emit `s < 0 && s = Math.max(...)` here,
    // which Bun/V8 rejects as an invalid LHS in assignment.
    let source = "\
        export function substr(src, start) {\n\
          let s = Number(start) || 0;\n\
          if (s < 0) s = Math.max(src.length + s, 0);\n\
          return src.slice(s);\n\
        }\n\
        export const out = substr('hello', -2);\n";
    std::fs::write(&entry, source).expect("write entry");
    let provider = Arc::new(SimpleVirtualSource {
        entry: entry.clone(),
        code: source.to_string(),
    });
    let result = bundle_virtual_entry(
        &entry,
        BundleOptions {
            project_root: tmp.clone(),
            minify: true,
            iife: false,
            stdlib_path: None,
        },
        provider,
    )
    .expect("minified bundle should succeed");
    assert!(
        !result.contains("&& s ="),
        "minifier broke `if (cond) x = y` into `cond && x = y`: {}",
        result
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

/// Regression: compress must not fold adjacent statements into a
/// `for-of` head. `count = 0; for (let _ of arr) ...` must stay two
/// statements — otherwise we get `for (let _ of count = 0, arr)`,
/// which is a parse error. Disabling `sequences` prevents this.
#[test]
fn bundle_minified_preserves_for_of_head() {
    let tmp = make_tmp_dir("for_of");
    let entry = tmp.join("entry.js");
    let source = "\
        export function __c(value) {\n\
          let count = 0;\n\
          for (const _ of (Array.isArray(value) ? value : [])) {\n\
            count += 1;\n\
          }\n\
          return count;\n\
        }\n\
        export const out = __c([1, 2, 3]);\n";
    std::fs::write(&entry, source).expect("write entry");
    let provider = Arc::new(SimpleVirtualSource {
        entry: entry.clone(),
        code: source.to_string(),
    });
    let result = bundle_virtual_entry(
        &entry,
        BundleOptions {
            project_root: tmp.clone(),
            minify: true,
            iife: false,
            stdlib_path: None,
        },
        provider,
    )
    .expect("minified bundle should succeed");
    assert!(
        !result.contains("of count = 0,"),
        "minifier folded a statement into the for-of head: {}",
        result
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn resolver_only_uses_project_local_php_modules() {
    // There is no stdlib fallback. A stdlib path passed to the resolver
    // is ignored — only project-local php_modules/ is consulted.
    let project = make_tmp_dir("no_stdlib_fallback_project");
    let stdlib = make_tmp_dir("no_stdlib_fallback_stdlib");

    std::fs::create_dir_all(project.join("php_modules")).unwrap();
    let crypto_dir = stdlib.join("crypto");
    std::fs::create_dir_all(&crypto_dir).unwrap();
    std::fs::write(
        crypto_dir.join("index.js"),
        "export function random_hex() { return '0a'; }\n",
    )
    .unwrap();

    let resolver = DekaResolver::new(project.clone(), Some(stdlib.clone())).unwrap();
    assert!(
        resolver.resolve_php_module("crypto").is_none(),
        "stdlib fallback is disabled: resolver must return None for missing packages"
    );

    // If project-local has the module it resolves normally.
    let local_crypto = project.join("php_modules").join("crypto");
    std::fs::create_dir_all(&local_crypto).unwrap();
    std::fs::write(
        local_crypto.join("index.js"),
        "export function random_hex() { return 'local'; }\n",
    )
    .unwrap();

    let resolver2 = DekaResolver::new(project.clone(), Some(stdlib.clone())).unwrap();
    let result2 = resolver2.resolve_php_module("crypto");
    assert!(result2.is_some(), "expected local resolution");
    assert!(
        result2.unwrap().starts_with(&project),
        "local php_modules should resolve"
    );

    let _ = std::fs::remove_dir_all(&project);
    let _ = std::fs::remove_dir_all(&stdlib);
}

#[test]
fn resolver_rejects_path_traversal() {
    let project = make_tmp_dir("path_traversal");
    let modules = project.join("php_modules");
    std::fs::create_dir_all(modules.join("component")).unwrap();
    std::fs::write(
        modules.join("component").join("button.js"),
        "export const Button = 'ok';\n",
    )
    .unwrap();

    // Also create a file outside php_modules to be the traversal target
    std::fs::write(project.join("secret.js"), "export const secret = 'oops';\n").unwrap();

    let resolver = DekaResolver::new(project.clone(), None).unwrap();

    // Normal resolution should work
    let normal = resolver.resolve_php_module("component/button");
    assert!(normal.is_some(), "normal module resolution should work");

    // Path traversal should fail — the specifier escapes php_modules/
    let traversal = resolver.resolve_php_module("component/../../secret");
    assert!(
        traversal.is_none(),
        "path traversal should be rejected, but resolved to: {:?}",
        traversal
    );

    let _ = std::fs::remove_dir_all(&project);
}

// Regression for issue #36: a file in api/ must be able to import from
// the package root via `../helpers`.  The resolved path must stay within
// the project root — this is the security check in guard_path_traversal.
#[test]
fn resolver_allows_parent_relative_import_within_project() {
    let project = make_tmp_dir("parent_relative_import");

    // Create project structure:
    //   helpers.js           <- the shared helper at the project root
    //   api/checkout.js      <- file that imports ../helpers
    let api_dir = project.join("api");
    std::fs::create_dir_all(&api_dir).unwrap();
    std::fs::write(
        project.join("helpers.js"),
        "export function client_ip() { return '127.0.0.1'; }\n",
    )
    .unwrap();
    std::fs::write(
        api_dir.join("checkout.js"),
        "import { client_ip } from '../helpers';\n",
    )
    .unwrap();

    let resolver = DekaResolver::new(project.clone(), None).unwrap();

    // Resolve `../helpers` from `api/checkout.js`
    let base = FileName::Real(api_dir.join("checkout.js"));
    let result = resolver.resolve(&base, "../helpers");
    assert!(
        result.is_ok(),
        "expected ../helpers to resolve from api/checkout.js, got: {:?}",
        result
    );
    let resolved_path = match result.unwrap().filename {
        FileName::Real(p) => p,
        other => panic!("expected FileName::Real, got {other:?}"),
    };
    assert!(
        resolved_path.starts_with(&project),
        "resolved path {:?} must stay within project root {:?}",
        resolved_path,
        project
    );

    let _ = std::fs::remove_dir_all(&project);
}

// Ensure that `../../..` traversal that exits the project root is blocked.
// DekaResolver now calls guard_path_traversal before returning Ok for any
// resolved relative path, so even files that exist outside the root are
// rejected with a path-traversal error (not file-not-found).
#[test]
fn resolver_parent_relative_import_stays_within_project() {
    // Layout:
    //   workspace/
    //     outside.js          <- the target file: exists but outside project
    //     project/
    //       api/
    //         checkout.js     <- base file
    //
    // `../outside` from `workspace/project/api/checkout.js`
    // resolves to `workspace/outside.js` — exists but outside the project root.
    let workspace = make_tmp_dir("parent_relative_escaping_workspace");
    let project = workspace.join("project");
    let api_dir = project.join("api");
    std::fs::create_dir_all(&api_dir).unwrap();
    std::fs::write(
        api_dir.join("checkout.js"),
        "import { x } from '../../outside';\n",
    )
    .unwrap();

    // Create a file that DOES exist but is OUTSIDE the project root.
    // This is the file that the previous implementation silently allowed
    // through (the only reason it returned Err was file-not-found).
    std::fs::write(workspace.join("outside.js"), "export const x = 'leaked';\n").unwrap();

    let resolver = DekaResolver::new(project.clone(), None).unwrap();
    let base = FileName::Real(api_dir.join("checkout.js"));

    // `../../outside` from `project/api/checkout.js` resolves to
    // `workspace/outside.js` — which exists but is outside the project root.
    // guard_path_traversal must reject it.
    let result = resolver.resolve(&base, "../../outside");
    assert!(
        result.is_err(),
        "expected path-traversal Err for import resolving outside project root, got Ok"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Path traversal") || err_msg.contains("traversal"),
        "expected path-traversal message, got: {err_msg}"
    );

    let _ = std::fs::remove_dir_all(&workspace);
}

// Confirm that going all-the-way out (many ../ hops) is also rejected.
#[test]
fn resolver_deep_traversal_to_system_path_is_rejected() {
    let project = make_tmp_dir("deep_traversal");
    let src_dir = project.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(src_dir.join("index.js"), "// entry\n").unwrap();

    let resolver = DekaResolver::new(project.clone(), None).unwrap();
    let base = FileName::Real(src_dir.join("index.js"));

    // This specifier attempts to climb to /etc/passwd (or an analogous
    // path on this OS). It either doesn't exist (Err from file-not-found)
    // or it does exist but must be rejected by the path-traversal guard.
    // Either way it must NOT return Ok with a path outside the project.
    let result = resolver.resolve(&base, "../../../../../../etc/passwd");
    if let Ok(ref res) = result {
        if let FileName::Real(ref p) = res.filename {
            panic!(
                "resolver returned Ok with path outside project root: {:?}",
                p
            );
        }
    }
    // Err is the expected outcome (traversal rejected or file not found).

    let _ = std::fs::remove_dir_all(&project);
}

// Positive test: a within-project cross-package `../` import is allowed.
// (The existing resolver_allows_parent_relative_import_within_project test
//  covers api/ → root helpers. This test covers one level deeper nesting.)
#[test]
fn resolver_allows_parent_relative_import_two_levels_within_project() {
    let project = make_tmp_dir("parent_relative_two_levels");
    let deep_dir = project.join("components").join("ui");
    std::fs::create_dir_all(&deep_dir).unwrap();
    std::fs::write(
        project.join("utils.js"),
        "export function fmt(x) { return String(x); }\n",
    )
    .unwrap();
    std::fs::write(
        deep_dir.join("button.js"),
        "import { fmt } from '../../utils';\n",
    )
    .unwrap();

    let resolver = DekaResolver::new(project.clone(), None).unwrap();
    let base = FileName::Real(deep_dir.join("button.js"));
    let result = resolver.resolve(&base, "../../utils");
    assert!(
        result.is_ok(),
        "expected ../../utils to resolve from components/ui/button.js, got: {:?}",
        result
    );
    let resolved_path = match result.unwrap().filename {
        FileName::Real(p) => p,
        other => panic!("expected FileName::Real, got {other:?}"),
    };
    assert!(
        resolved_path.starts_with(&project),
        "resolved path {:?} must stay within project root {:?}",
        resolved_path,
        project
    );

    let _ = std::fs::remove_dir_all(&project);
}
