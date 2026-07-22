use super::*;

// ---- 9. Prelude uses ??= pattern ----

#[test]
fn prelude_uses_nullish_assignment() {
    let js = phpx_to_js("$x = 1;").expect("should compile");
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
    // But kept entries should still be present
    assert!(
        js.contains("globalThis.panic ??="),
        "globalThis.panic should still be in prelude"
    );
    assert!(
        js.contains("globalThis.defined ??="),
        "globalThis.defined should still be in prelude"
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

#[test]
fn prelude_contains_phpx_is_struct() {
    let js = phpx_to_js("$x = 1;").expect("should compile");
    assert!(
        js.contains("__phpx_is_struct"),
        "expected __phpx_is_struct in prelude, got first 500 chars:\n{}",
        &js[..std::cmp::min(500, js.len())]
    );
}

#[test]
fn prelude_contains_panic() {
    let js = phpx_to_js("$x = 1;").expect("should compile");
    assert!(
        js.contains("globalThis.panic"),
        "expected panic in prelude, got first 500 chars:\n{}",
        &js[..std::cmp::min(500, js.len())]
    );
}

// ---- Top-level vs function scope: globalThis mirroring ----
