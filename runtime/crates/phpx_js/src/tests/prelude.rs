use super::*;

// ---- 9. Prelude uses ??= pattern ----

#[test]
fn prelude_uses_nullish_assignment() {
    // #47: the prelude is demand-driven, so `$x = 1;` alone (referencing no
    // mission/runtime helper) now emits ZERO globalThis lines — see
    // `prelude_no_helpers_emits_no_globalthis_prelude`. To check the ??=
    // pattern itself, use a source that actually triggers a polyfill.
    let js = phpx_to_js("panic(\"boom\");").expect("should compile");
    // The prelude should use ??= for global setup
    assert!(js.contains("??="), "expected ??= in prelude, got:\n{}", js);
    // Should NOT use the anti-pattern `if (!globalThis.X) { globalThis.X = ... }`
    // for polyfill definitions (those should use ??=)
    let lines: Vec<&str> = js
        .lines()
        .filter(|l| {
            l.contains("if (!globalThis.") && l.contains("globalThis.") && !l.contains("__deka")
        })
        .collect();
    // Allow __dekaGlobalsInstalled guard but polyfills should use ??=
    for line in &lines {
        assert!(
            !line.contains("function"),
            "function polyfills should use ??= not if-guard: {}",
            line
        );
    }
}

// ---- Additional coverage ----
#[test]
fn prelude_no_removed_polyfills() {
    let js = phpx_to_js("$x = 1;").expect("should compile");
    // Verify that removed class (a) polyfills are NOT in the prelude
    assert!(
        !js.contains("globalThis.count ??="),
        "globalThis.count polyfill should be removed"
    );
    assert!(
        !js.contains("globalThis.strlen ??="),
        "globalThis.strlen polyfill should be removed"
    );
    assert!(
        !js.contains("globalThis.substr ??="),
        "globalThis.substr polyfill should be removed"
    );
    assert!(
        !js.contains("globalThis.trim ??="),
        "globalThis.trim polyfill should be removed"
    );
    assert!(
        !js.contains("globalThis.ltrim ??="),
        "globalThis.ltrim polyfill should be removed"
    );
    assert!(
        !js.contains("globalThis.rtrim ??="),
        "globalThis.rtrim polyfill should be removed"
    );
    assert!(
        !js.contains("globalThis.chr ??="),
        "globalThis.chr polyfill should be removed"
    );
    assert!(
        !js.contains("globalThis.ord ??="),
        "globalThis.ord polyfill should be removed"
    );
    assert!(
        !js.contains("globalThis.time ??="),
        "globalThis.time polyfill should be removed"
    );
    assert!(
        !js.contains("globalThis.strtolower ??="),
        "globalThis.strtolower polyfill should be removed"
    );
    assert!(
        !js.contains("globalThis.strtoupper ??="),
        "globalThis.strtoupper polyfill should be removed"
    );
    assert!(
        !js.contains("globalThis.strpos ??="),
        "globalThis.strpos polyfill should be removed"
    );
    assert!(
        !js.contains("globalThis.strrpos ??="),
        "globalThis.strrpos polyfill should be removed"
    );
    assert!(
        !js.contains("globalThis.str_starts_with ??="),
        "globalThis.str_starts_with polyfill should be removed"
    );
    assert!(
        !js.contains("globalThis.str_ends_with ??="),
        "globalThis.str_ends_with polyfill should be removed"
    );
    assert!(
        !js.contains("globalThis.str_contains ??="),
        "globalThis.str_contains polyfill should be removed"
    );
    assert!(
        !js.contains("globalThis.array_key_exists ??="),
        "globalThis.array_key_exists polyfill should be removed"
    );
    assert!(
        !js.contains("globalThis.in_array ??="),
        "globalThis.in_array polyfill should be removed"
    );
    assert!(
        !js.contains("globalThis.explode ??="),
        "globalThis.explode polyfill should be removed"
    );
    assert!(
        !js.contains("globalThis.implode ??="),
        "globalThis.implode polyfill should be removed"
    );
    assert!(
        !js.contains("globalThis.is_array ="),
        "globalThis.is_array polyfill should be removed"
    );
    assert!(
        !js.contains("globalThis.array_keys ??="),
        "globalThis.array_keys polyfill should not be added"
    );
    assert!(
        !js.contains("globalThis.array_values ??="),
        "globalThis.array_values polyfill should not be added"
    );
    assert!(
        !js.contains("globalThis.array_map ??="),
        "globalThis.array_map polyfill should be removed"
    );
    assert!(
        !js.contains("globalThis.array_filter ??="),
        "globalThis.array_filter polyfill should be removed"
    );
    // Type predicates are compile-time IIFE rewrites — no globalThis polyfill.
    assert!(
        !js.contains("globalThis.is_int ??="),
        "globalThis.is_int polyfill should be removed"
    );
    assert!(
        !js.contains("globalThis.is_float ??="),
        "globalThis.is_float polyfill should be removed"
    );
    assert!(
        !js.contains("globalThis.is_numeric ??="),
        "globalThis.is_numeric polyfill should be removed"
    );
    assert!(
        !js.contains("globalThis.is_string ??="),
        "globalThis.is_string polyfill should be removed"
    );
    assert!(
        !js.contains("globalThis.is_object ??="),
        "globalThis.is_object polyfill should be removed"
    );
    // Batch-2 functions: all are now compile-time rewrites, none should appear as globalThis polyfills.
    assert!(
        !js.contains("globalThis.urlencode ??="),
        "globalThis.urlencode polyfill should be removed"
    );
    assert!(
        !js.contains("globalThis.urldecode ??="),
        "globalThis.urldecode polyfill should be removed"
    );
    assert!(
        !js.contains("globalThis.gettype ??="),
        "globalThis.gettype polyfill should be removed"
    );
    assert!(
        !js.contains("globalThis.get_object_vars ??="),
        "globalThis.get_object_vars polyfill should be removed"
    );
    assert!(
        !js.contains("globalThis.mt_rand ??="),
        "globalThis.mt_rand polyfill should be removed"
    );
    assert!(
        !js.contains("globalThis.microtime ??="),
        "globalThis.microtime polyfill should be removed"
    );
    assert!(
        !js.contains("globalThis.strtotime ??="),
        "globalThis.strtotime polyfill should be removed"
    );
    assert!(
        !js.contains("globalThis.preg_match ??="),
        "globalThis.preg_match polyfill should be removed"
    );
    assert!(
        !js.contains("globalThis.preg_replace ??="),
        "globalThis.preg_replace polyfill should be removed"
    );
    assert!(
        !js.contains("globalThis.parse_url ??="),
        "globalThis.parse_url polyfill should be removed"
    );
    assert!(
        !js.contains("globalThis.intval ??="),
        "globalThis.intval polyfill should be removed"
    );
    assert!(
        !js.contains("globalThis.floatval ??="),
        "globalThis.floatval polyfill should be removed"
    );
    assert!(
        !js.contains("globalThis.boolval ??="),
        "globalThis.boolval polyfill should be removed"
    );
    assert!(
        !js.contains("globalThis.strval ??="),
        "globalThis.strval polyfill should be removed"
    );
    assert!(
        !js.contains("globalThis.array_slice ??="),
        "globalThis.array_slice polyfill should be removed"
    );
    // Kept entries (panic, defined, etc.) are demand-driven as of #47: they
    // no longer appear unconditionally for a program that never references
    // them. See `prelude_kept_entries_present_when_referenced` below for the
    // positive case, and `prelude_no_helpers_emits_no_globalthis_prelude` for
    // the "referenced nothing -> nothing" case this program falls into.
    assert!(
        !js.contains("globalThis.panic ??="),
        "globalThis.panic should NOT be in the prelude when panic() is never called (#47)"
    );
    assert!(
        !js.contains("globalThis.defined ??="),
        "globalThis.defined should NOT be in the prelude when defined() is never called (#47)"
    );
}

// ---- Phase 2 (easy polyfills) rewrite tests ----

#[test]
fn prelude_batch3_no_removed_polyfills() {
    // Batch 3 polyfills must be gone — neither the old un-mangled form nor the
    // old globalThis.__phpx_X ??= form should appear in output.
    let js = phpx_to_js("$x = 1;").expect("should compile");
    // Old un-mangled polyfills (batch 3 targets) must be absent.
    assert!(
        !js.contains("globalThis.base64_encode ??="),
        "globalThis.base64_encode polyfill must be gone"
    );
    assert!(
        !js.contains("globalThis.base64_decode ??="),
        "globalThis.base64_decode polyfill must be gone"
    );
    assert!(
        !js.contains("globalThis.hash ??="),
        "globalThis.hash polyfill must be gone"
    );
    assert!(
        !js.contains("globalThis.hash_hmac ??="),
        "globalThis.hash_hmac polyfill must be gone"
    );
    assert!(
        !js.contains("globalThis.hash_equals ??="),
        "globalThis.hash_equals polyfill must be gone"
    );
    assert!(
        !js.contains("globalThis.date ??="),
        "globalThis.date polyfill must be gone"
    );
    assert!(
        !js.contains("globalThis.gmdate ??="),
        "globalThis.gmdate polyfill must be gone"
    );
    assert!(
        !js.contains("globalThis.pack ??="),
        "globalThis.pack polyfill must be gone"
    );
    assert!(
        !js.contains("globalThis.function_exists ??="),
        "globalThis.function_exists polyfill must be gone"
    );
    assert!(
        !js.contains("globalThis.class_exists ??="),
        "globalThis.class_exists polyfill must be gone"
    );
    // Tier B helpers must NOT appear in output for a file that does not use them.
    // They are module-scoped and emitted only when referenced (DCE via omission).
    assert!(
        !js.contains("globalThis.__phpx_base64_encode"),
        "globalThis.__phpx_base64_encode must be absent when unused"
    );
    assert!(
        !js.contains("globalThis.__phpx_base64_decode"),
        "globalThis.__phpx_base64_decode must be absent when unused"
    );
    assert!(
        !js.contains("globalThis.__phpx_hash ??="),
        "globalThis.__phpx_hash must be absent when unused"
    );
    assert!(
        !js.contains("globalThis.__phpx_hash_hmac"),
        "globalThis.__phpx_hash_hmac must be absent when unused"
    );
    assert!(
        !js.contains("globalThis.__phpx_date ??="),
        "globalThis.__phpx_date must be absent when unused"
    );
    assert!(
        !js.contains("globalThis.__phpx_gmdate"),
        "globalThis.__phpx_gmdate must be absent when unused"
    );
    assert!(
        !js.contains("globalThis.__phpx_pack"),
        "globalThis.__phpx_pack must be absent when unused"
    );
    assert!(
        !js.contains("function __phpx_base64_encode"),
        "base64_encode helper must be absent when unused"
    );
    assert!(
        !js.contains("function __phpx_hash("),
        "hash helper must be absent when unused"
    );
    assert!(
        !js.contains("function __phpx_pack"),
        "pack helper must be absent when unused"
    );
    assert!(
        !js.contains("function __phpx_date("),
        "date helper must be absent when unused"
    );
}

// #47: the mission/runtime globalThis prelude is demand-driven — an entry
// is only emitted when the compiled program's own body actually references
// it. `$x = 1;` references none of it, so these two tests (which previously
// asserted __phpx_is_struct / panic were ALWAYS present) now use fixtures
// that actually trigger each helper, and a new pair of tests directly below
// covers the "referenced nothing" / "referenced exactly one thing" cases.

#[test]
fn prelude_contains_phpx_is_struct_when_instanceof_struct_used() {
    let source = r#"
struct Point {
$x: int;
$y: int;
}
function f(): bool {
$p = { x: 1, y: 2, __struct: 'Point' };
return $p instanceof Point;
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(
        js.contains("__phpx_is_struct"),
        "expected __phpx_is_struct in prelude when `instanceof` on a struct is used, got:\n{}",
        js
    );
}

#[test]
fn prelude_contains_panic_when_panic_called() {
    let js = phpx_to_js("panic(\"boom\");").expect("should compile");
    assert!(
        js.contains("globalThis.panic ??="),
        "expected panic in prelude when panic() is called, got first 500 chars:\n{}",
        &js[..std::cmp::min(500, js.len())]
    );
}

#[test]
fn prelude_no_helpers_emits_no_globalthis_prelude() {
    // #47 acceptance: a program referencing no mission/runtime helper emits
    // no `globalThis.<name> ??= ...` prelude lines at all — not "a smaller
    // one", none.
    let js = phpx_to_js("$x = 1;").expect("should compile");
    let assignment_lines: Vec<&str> = js
        .lines()
        .filter(|line| line.starts_with("globalThis.") && line.contains("??="))
        .collect();
    assert!(
        assignment_lines.is_empty(),
        "expected zero globalThis.* ??= prelude lines for a program that references none \
         of them, got:\n{}",
        assignment_lines.join("\n")
    );
}

#[test]
fn prelude_exactly_one_helper_emits_only_that_helper() {
    // #47 acceptance: a program referencing exactly one helper (getenv)
    // emits that helper and none of the other ~40 mission/runtime entries.
    let js = phpx_to_js("$v = getenv(\"HOME\");").expect("should compile");
    assert!(
        js.contains("globalThis.getenv ??="),
        "expected globalThis.getenv in prelude when getenv() is called, got:\n{}",
        js
    );
    let other_entries = [
        "globalThis.panic ??=",
        "globalThis.class_alias ??=",
        "globalThis.defined ??=",
        "globalThis.__phpx_is_struct ??=",
        "globalThis.is_promise ??=",
        "globalThis.GLOBALS ??=",
        "globalThis.JSON_ERROR_NONE ??=",
        "globalThis.__deka_chr ??=",
        "globalThis.__deka_ord ??=",
        "globalThis.__deka_object_set ??=",
        "globalThis.__phpx_symbol_table ??=",
        "globalThis.__phpx_array_cursor ??=",
        "globalThis.__phpx_stat ??=",
        "globalThis.is_file ??=",
        "globalThis.is_dir ??=",
        "globalThis.mkdir ??=",
        "globalThis.file ??=",
        "globalThis.error_log ??=",
        "globalThis.error_get_last ??=",
        "globalThis.set_error_handler ??=",
        "globalThis.register_shutdown_function ??=",
        "globalThis.__phpx_serve_php ??=",
        "globalThis.__phpxCurrentResponse ??=",
        "globalThis.header ??=",
        "globalThis.phpxStartBuffer ??=",
        "globalThis.phpxEndBuffer ??=",
        "globalThis.phpxWrapHandler ??=",
        "globalThis.jsx ??=",
        "globalThis.jsxs ??=",
        "globalThis.__phpxStructMethods ??=",
    ];
    for entry in other_entries {
        assert!(
            !js.contains(entry),
            "expected `{}` to be absent when the program only calls getenv(), got:\n{}",
            entry,
            js
        );
    }
}

#[test]
fn prelude_kept_entries_present_when_referenced() {
    // Companion to `prelude_no_removed_polyfills`'s negative assertions:
    // panic/defined ARE still real prelude entries, just demand-driven now.
    let js = phpx_to_js("panic(\"boom\"); $seen = defined(\"X\");").expect("should compile");
    assert!(
        js.contains("globalThis.panic ??="),
        "globalThis.panic should be present when panic() is called: {}",
        js
    );
    assert!(
        js.contains("globalThis.defined ??="),
        "globalThis.defined should be present when defined() is called: {}",
        js
    );
}

// ---- Top-level vs function scope: globalThis mirroring ----
