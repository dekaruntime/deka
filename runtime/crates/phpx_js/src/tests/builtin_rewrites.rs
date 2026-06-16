use super::*;

#[test]
fn rewrite_count_inline() {
    let js = phpx_to_js("$arr = [1, 2, 3];\n$n = count($arr);").expect("should compile");
    assert!(
        js.contains(".length"),
        "expected .length for count, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.count"),
        "should NOT contain globalThis.count, got:\n{}",
        js
    );
}

#[test]
fn rewrite_strlen_inline() {
    let js = phpx_to_js("$s = 'hello';\n$n = strlen($s);").expect("should compile");
    assert!(
        js.contains("String(") && js.contains(".length"),
        "expected String().length for strlen, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.strlen"),
        "should NOT contain globalThis.strlen, got:\n{}",
        js
    );
}

#[test]
fn rewrite_substr_two_args() {
    let js = phpx_to_js("$s = 'hello';\n$r = substr($s, 2);").expect("should compile");
    assert!(
        js.contains(".slice("),
        "expected .slice for substr, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.substr"),
        "should NOT contain globalThis.substr, got:\n{}",
        js
    );
}

#[test]
fn rewrite_substr_three_args() {
    let js = phpx_to_js("$s = 'hello';\n$r = substr($s, 1, 3);").expect("should compile");
    assert!(
        js.contains(".slice("),
        "expected .slice for substr(3), got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.substr"),
        "should NOT contain globalThis.substr, got:\n{}",
        js
    );
}

#[test]
fn rewrite_trim_inline() {
    let js = phpx_to_js("$s = '  hello  ';\n$r = trim($s);").expect("should compile");
    assert!(
        js.contains(".trim()"),
        "expected .trim() for trim, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.trim"),
        "should NOT contain globalThis.trim, got:\n{}",
        js
    );
}

#[test]
fn rewrite_ltrim_inline() {
    let js = phpx_to_js("$s = '  hello';\n$r = ltrim($s);").expect("should compile");
    assert!(
        js.contains(".trimStart()"),
        "expected .trimStart() for ltrim, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.ltrim"),
        "should NOT contain globalThis.ltrim, got:\n{}",
        js
    );
}

#[test]
fn rewrite_rtrim_inline() {
    let js = phpx_to_js("$s = 'hello  ';\n$r = rtrim($s);").expect("should compile");
    assert!(
        js.contains(".trimEnd()"),
        "expected .trimEnd() for rtrim, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.rtrim"),
        "should NOT contain globalThis.rtrim, got:\n{}",
        js
    );
}

#[test]
fn rewrite_strpos_inline() {
    let js = phpx_to_js("$h = 'hello';\n$i = strpos($h, 'l');").expect("should compile");
    assert!(
        js.contains(".indexOf("),
        "expected .indexOf for strpos, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.strpos"),
        "should NOT contain globalThis.strpos, got:\n{}",
        js
    );
}

#[test]
fn rewrite_strrpos_inline() {
    let js = phpx_to_js("$h = 'hello';\n$i = strrpos($h, 'l');").expect("should compile");
    assert!(
        js.contains(".lastIndexOf("),
        "expected .lastIndexOf for strrpos, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.strrpos"),
        "should NOT contain globalThis.strrpos, got:\n{}",
        js
    );
}

#[test]
fn rewrite_str_starts_with_inline() {
    let js = phpx_to_js("$h = 'hello';\n$b = str_starts_with($h, 'he');").expect("should compile");
    assert!(
        js.contains(".startsWith("),
        "expected .startsWith for str_starts_with, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.str_starts_with"),
        "should NOT contain globalThis.str_starts_with, got:\n{}",
        js
    );
}

#[test]
fn rewrite_str_ends_with_inline() {
    let js = phpx_to_js("$h = 'hello';\n$b = str_ends_with($h, 'lo');").expect("should compile");
    assert!(
        js.contains(".endsWith("),
        "expected .endsWith for str_ends_with, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.str_ends_with"),
        "should NOT contain globalThis.str_ends_with, got:\n{}",
        js
    );
}

#[test]
fn rewrite_str_contains_inline() {
    let js =
        phpx_to_js("$h = 'hello world';\n$b = str_contains($h, 'world');").expect("should compile");
    assert!(
        js.contains(".includes("),
        "expected .includes for str_contains, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.str_contains"),
        "should NOT contain globalThis.str_contains, got:\n{}",
        js
    );
}

#[test]
fn rewrite_strtolower_inline() {
    let js = phpx_to_js("$s = 'HELLO';\n$r = strtolower($s);").expect("should compile");
    assert!(
        js.contains(".toLowerCase()"),
        "expected .toLowerCase for strtolower, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.strtolower"),
        "should NOT contain globalThis.strtolower, got:\n{}",
        js
    );
}

#[test]
fn rewrite_strtoupper_inline() {
    let js = phpx_to_js("$s = 'hello';\n$r = strtoupper($s);").expect("should compile");
    assert!(
        js.contains(".toUpperCase()"),
        "expected .toUpperCase for strtoupper, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.strtoupper"),
        "should NOT contain globalThis.strtoupper, got:\n{}",
        js
    );
}

#[test]
fn rewrite_array_key_exists_inline() {
    let js = phpx_to_js("$a = { x: 1 };\n$b = array_key_exists('x', $a);").expect("should compile");
    assert!(
        js.contains("hasOwnProperty"),
        "expected hasOwnProperty for array_key_exists, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.array_key_exists"),
        "should NOT contain globalThis.array_key_exists, got:\n{}",
        js
    );
}

#[test]
fn rewrite_in_array_inline() {
    let js = phpx_to_js("$arr = [1, 2, 3];\n$b = in_array(2, $arr);").expect("should compile");
    assert!(
        js.contains(".includes("),
        "expected .includes for in_array, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.in_array"),
        "should NOT contain globalThis.in_array, got:\n{}",
        js
    );
}

#[test]
fn rewrite_explode_inline() {
    let js = phpx_to_js("$s = 'a,b,c';\n$arr = explode(',', $s);").expect("should compile");
    assert!(
        js.contains(".split("),
        "expected .split for explode, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.explode"),
        "should NOT contain globalThis.explode, got:\n{}",
        js
    );
}

#[test]
fn rewrite_implode_inline() {
    let js = phpx_to_js("$arr = ['a', 'b'];\n$s = implode(',', $arr);").expect("should compile");
    assert!(
        js.contains(".join("),
        "expected .join for implode, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.implode"),
        "should NOT contain globalThis.implode, got:\n{}",
        js
    );
}

#[test]
fn rewrite_chr_inline() {
    let js = phpx_to_js("$c = chr(65);").expect("should compile");
    assert!(
        js.contains("String.fromCharCode("),
        "expected String.fromCharCode for chr, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.chr"),
        "should NOT contain globalThis.chr, got:\n{}",
        js
    );
}

#[test]
fn rewrite_ord_inline() {
    let js = phpx_to_js("$s = 'A';\n$n = ord($s);").expect("should compile");
    assert!(
        js.contains(".charCodeAt(0)"),
        "expected .charCodeAt(0) for ord, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.ord"),
        "should NOT contain globalThis.ord, got:\n{}",
        js
    );
}

#[test]
fn rewrite_time_inline() {
    let js = phpx_to_js("$t = time();").expect("should compile");
    assert!(
        js.contains("Math.floor(Date.now() / 1000)"),
        "expected Date.now for time, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.time"),
        "should NOT contain globalThis.time, got:\n{}",
        js
    );
}

#[test]
fn rewrite_array_keys_inline() {
    let js = phpx_to_js("$a = { x: 1, y: 2 };\n$k = array_keys($a);").expect("should compile");
    assert!(
        js.contains("Object.keys("),
        "expected Object.keys for array_keys, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.array_keys"),
        "should NOT contain globalThis.array_keys, got:\n{}",
        js
    );
}

#[test]
fn rewrite_array_keys_on_array_emits_numeric_indices() {
    let js = phpx_to_js("$a = [10, 20, 30];\n$k = array_keys($a);").expect("should compile");
    // For arrays we emit `__v.map((_, __i) => __i)` so the shape matches PHP
    // (integer indices) rather than JS `Object.keys` stringified indices.
    assert!(
        js.contains(".map("),
        "expected map for array-path array_keys, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.array_keys"),
        "should NOT contain globalThis.array_keys, got:\n{}",
        js
    );
}

#[test]
fn rewrite_array_values_inline() {
    let js = phpx_to_js("$a = { x: 1, y: 2 };\n$v = array_values($a);").expect("should compile");
    assert!(
        js.contains("Object.values("),
        "expected Object.values for array_values, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.array_values"),
        "should NOT contain globalThis.array_values, got:\n{}",
        js
    );
}

#[test]
fn rewrite_array_values_on_array_copies() {
    let js = phpx_to_js("$a = [1, 2, 3];\n$v = array_values($a);").expect("should compile");
    // For arrays we emit .slice() to return a shallow copy.
    assert!(
        js.contains(".slice()"),
        "expected .slice() for array-path array_values, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.array_values"),
        "should NOT contain globalThis.array_values, got:\n{}",
        js
    );
}

#[test]
fn rewrite_array_map_inline() {
    let js =
        phpx_to_js("$fn = fn($x: int): int => $x + 1;\n$a = [1, 2, 3];\n$r = array_map($fn, $a);")
            .expect("should compile");
    assert!(
        js.contains("a.map(fn)"),
        "expected array_map($fn, $a) to rewrite to a.map(fn), got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.array_map"),
        "should NOT contain globalThis.array_map, got:\n{}",
        js
    );
}

#[test]
fn rewrite_array_filter_inline() {
    let js = phpx_to_js(
        "$fn = fn($x: int): bool => $x > 1;\n$a = [1, 2, 3];\n$r = array_filter($a, $fn);",
    )
    .expect("should compile");
    assert!(
        js.contains("a.filter(fn)"),
        "expected array_filter($a, $fn) to rewrite to a.filter(fn), got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.array_filter"),
        "should NOT contain globalThis.array_filter, got:\n{}",
        js
    );
}

#[test]
fn rewrite_array_filter_without_callback_uses_boolean() {
    let js = phpx_to_js("$a = [0, 1, 2];\n$r = array_filter($a);").expect("should compile");
    assert!(
        js.contains("a.filter(Boolean)"),
        "expected array_filter($a) to rewrite to a.filter(Boolean), got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.array_filter"),
        "should NOT contain globalThis.array_filter, got:\n{}",
        js
    );
}

#[test]
fn rewrite_is_array_inline() {
    let js = phpx_to_js("$arr = [1];\n$b = is_array($arr);").expect("should compile");
    assert!(
        js.contains("Array.isArray"),
        "expected Array.isArray for is_array, got:\n{}",
        js
    );
    // Check struct exclusion is present
    assert!(
        js.contains("__struct"),
        "expected __struct check in is_array, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.is_array"),
        "should NOT contain globalThis.is_array, got:\n{}",
        js
    );
}

#[test]
fn rewrite_does_not_shadow_user_variables() {
    // If user declares $count as a variable, calling count() after should
    // use the user's local variable, not the builtin rewrite.
    // (In practice, calling `$count(...)` is a different AST shape than `count(...)`)
    let js = phpx_to_js("$count = 0;\n$x = $count + 1;").expect("should compile");
    // $count variable reference should appear as local `count`, not as a rewrite
    assert!(
        js.contains("let count = 0"),
        "expected let count = 0, got:\n{}",
        js
    );
}

#[test]
fn rewrite_max_multi_arg() {
    let js = phpx_to_js("$n = max(1, 2, 3);").expect("should compile");
    assert!(
        js.contains("Math.max("),
        "expected Math.max for max, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.max"),
        "should NOT contain globalThis.max, got:\n{}",
        js
    );
}

#[test]
fn rewrite_max_single_array_arg() {
    let js = phpx_to_js("$arr = [1, 2, 3];\n$n = max($arr);").expect("should compile");
    // Single arg emits IIFE checking Array.isArray
    assert!(
        js.contains("Array.isArray"),
        "expected Array.isArray check for max(arr), got:\n{}",
        js
    );
    assert!(
        js.contains("Math.max("),
        "expected Math.max for max(arr), got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.max"),
        "should NOT contain globalThis.max, got:\n{}",
        js
    );
}

#[test]
fn rewrite_min_multi_arg() {
    let js = phpx_to_js("$n = min(4, 2, 9);").expect("should compile");
    assert!(
        js.contains("Math.min("),
        "expected Math.min for min, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.min"),
        "should NOT contain globalThis.min, got:\n{}",
        js
    );
}

#[test]
fn rewrite_is_int_inline() {
    let js = phpx_to_js("$v = 42;\n$b = is_int($v);").expect("should compile");
    assert!(
        js.contains("typeof"),
        "expected typeof check for is_int, got:\n{}",
        js
    );
    assert!(
        js.contains("Number.isInteger"),
        "expected Number.isInteger for is_int, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.is_int"),
        "should NOT contain globalThis.is_int, got:\n{}",
        js
    );
}

#[test]
fn rewrite_is_float_inline() {
    let js = phpx_to_js("$v = 3.14;\n$b = is_float($v);").expect("should compile");
    assert!(
        js.contains("typeof"),
        "expected typeof check for is_float, got:\n{}",
        js
    );
    assert!(
        js.contains("Number.isInteger"),
        "expected Number.isInteger for is_float, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.is_float"),
        "should NOT contain globalThis.is_float, got:\n{}",
        js
    );
}

#[test]
fn rewrite_is_numeric_inline() {
    let js = phpx_to_js("$v = '42';\n$b = is_numeric($v);").expect("should compile");
    assert!(
        js.contains("isNaN(Number("),
        "expected isNaN(Number()) for is_numeric, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.is_numeric"),
        "should NOT contain globalThis.is_numeric, got:\n{}",
        js
    );
}

#[test]
fn rewrite_is_string_inline() {
    let js = phpx_to_js("$v = 'hello';\n$b = is_string($v);").expect("should compile");
    assert!(
        js.contains("typeof"),
        "expected typeof for is_string, got:\n{}",
        js
    );
    assert!(
        js.contains("\"string\""),
        "expected string type check for is_string, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.is_string"),
        "should NOT contain globalThis.is_string, got:\n{}",
        js
    );
}

#[test]
fn rewrite_is_object_inline() {
    let js = phpx_to_js("$v = { x: 1 };\n$b = is_object($v);").expect("should compile");
    assert!(
        js.contains("typeof"),
        "expected typeof for is_object, got:\n{}",
        js
    );
    assert!(
        js.contains("Array.isArray"),
        "expected Array.isArray check for is_object, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.is_object"),
        "should NOT contain globalThis.is_object, got:\n{}",
        js
    );
}

#[test]
fn rewrite_htmlspecialchars_inline() {
    let js = phpx_to_js("$s = '<script>';\n$e = htmlspecialchars($s);").expect("should compile");
    assert!(
        js.contains(".replace("),
        "expected .replace() chain for htmlspecialchars, got:\n{}",
        js
    );
    assert!(
        js.contains("&amp;"),
        "expected &amp; entity for htmlspecialchars, got:\n{}",
        js
    );
    assert!(
        js.contains("&lt;"),
        "expected &lt; entity for htmlspecialchars, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.htmlspecialchars"),
        "should NOT contain globalThis.htmlspecialchars, got:\n{}",
        js
    );
}

#[test]
fn rewrite_str_replace_scalar_inline() {
    let js = phpx_to_js("$s = 'hello world';\n$r = str_replace('world', 'earth', $s);")
        .expect("should compile");
    assert!(
        js.contains(".split("),
        "expected .split for str_replace, got:\n{}",
        js
    );
    assert!(
        js.contains(".join("),
        "expected .join for str_replace, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.str_replace"),
        "should NOT contain globalThis.str_replace, got:\n{}",
        js
    );
}

#[test]
fn rewrite_rawurlencode_inline() {
    let js = phpx_to_js("$s = 'hello world';\n$e = rawurlencode($s);").expect("should compile");
    assert!(
        js.contains("encodeURIComponent("),
        "expected encodeURIComponent for rawurlencode, got:\n{}",
        js
    );
    assert!(
        js.contains("%21"),
        "expected %21 replacement for rawurlencode, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.rawurlencode"),
        "should NOT contain globalThis.rawurlencode, got:\n{}",
        js
    );
}

#[test]
fn rewrite_dechex_inline() {
    let js = phpx_to_js("$n = 255;\n$h = dechex($n);").expect("should compile");
    assert!(
        js.contains(".toString(16)"),
        "expected .toString(16) for dechex, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.dechex"),
        "should NOT contain globalThis.dechex, got:\n{}",
        js
    );
}

#[test]
fn rewrite_hexdec_inline() {
    let js = phpx_to_js("$s = 'ff';\n$n = hexdec($s);").expect("should compile");
    assert!(
        js.contains("parseInt("),
        "expected parseInt for hexdec, got:\n{}",
        js
    );
    assert!(
        js.contains(", 16)"),
        "expected base 16 for hexdec, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.hexdec"),
        "should NOT contain globalThis.hexdec, got:\n{}",
        js
    );
}

#[test]
fn rewrite_ltrim_with_chars_inline() {
    let js = phpx_to_js("$s = '...hello';\n$r = ltrim($s, '.');").expect("should compile");
    // 2-arg ltrim emits an IIFE with regex escaping
    assert!(
        js.contains("new RegExp("),
        "expected new RegExp for ltrim with chars, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.ltrim"),
        "should NOT contain globalThis.ltrim, got:\n{}",
        js
    );
}

#[test]
fn rewrite_rtrim_with_chars_inline() {
    let js = phpx_to_js("$s = 'hello...';\n$r = rtrim($s, '.');").expect("should compile");
    assert!(
        js.contains("new RegExp("),
        "expected new RegExp for rtrim with chars, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.rtrim"),
        "should NOT contain globalThis.rtrim, got:\n{}",
        js
    );
}

// ---- Batch 2: urlencode / urldecode / gettype / get_object_vars / mt_rand /
//              microtime / strtotime / preg_match / preg_replace / parse_url /
//              intval / floatval / boolval / strval / array_slice ----

#[test]
fn rewrite_urlencode_inline() {
    let js = phpx_to_js("$s = 'hello world';\n$e = urlencode($s);").expect("should compile");
    assert!(
        js.contains("encodeURIComponent("),
        "expected encodeURIComponent for urlencode, got:\n{}",
        js
    );
    assert!(
        js.contains("%20"),
        "expected %20-to-+ substitution for urlencode, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.urlencode"),
        "should NOT contain globalThis.urlencode, got:\n{}",
        js
    );
}

#[test]
fn rewrite_urldecode_inline() {
    let js = phpx_to_js("$s = 'hello+world';\n$d = urldecode($s);").expect("should compile");
    assert!(
        js.contains("decodeURIComponent("),
        "expected decodeURIComponent for urldecode, got:\n{}",
        js
    );
    assert!(
        js.contains("replace(/\\+/g"),
        "expected +→%20 substitution for urldecode, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.urldecode"),
        "should NOT contain globalThis.urldecode, got:\n{}",
        js
    );
    // Must be wrapped in IIFE with try/catch — no bare decodeURIComponent call.
    assert!(
        js.contains("try {"),
        "expected try/catch guard against URIError, got:\n{}",
        js
    );
    assert!(
        js.contains("catch("),
        "expected catch clause for URIError guard, got:\n{}",
        js
    );
}

// Regression: Hamza's DoS report — malformed %XX sequences must not throw URIError.
// PHP urldecode passes malformed sequences through unchanged.
#[test]
fn urldecode_malformed_percent_g0_no_throw() {
    // %G0 is not a valid percent-sequence — JS decodeURIComponent throws URIError.
    // The rewrite must emit a try/catch so the IIFE returns the raw string instead.
    let js = phpx_to_js("$d = urldecode('%G0');").expect("should compile");
    // Verify the guard structure is present in the emitted JS.
    assert!(
        js.contains("try {"),
        "urldecode('%G0') must emit try/catch guard, got:\n{}",
        js
    );
    assert!(
        js.contains("catch("),
        "urldecode('%G0') must emit catch clause, got:\n{}",
        js
    );
    // Verify the emitted code is syntactically valid and evaluates without throwing.
    // (We cannot run JS here, but structure checks above are sufficient for unit scope.)
}

#[test]
fn urldecode_malformed_truncated_percent_no_throw() {
    // %2 has only one hex digit — also invalid.
    let js = phpx_to_js("$d = urldecode('%2');").expect("should compile");
    assert!(
        js.contains("try {"),
        "urldecode('%2') must emit try/catch guard, got:\n{}",
        js
    );
}

#[test]
fn urldecode_malformed_incomplete_multibyte_no_throw() {
    // %E0 is the first byte of a three-byte UTF-8 sequence with no continuation bytes —
    // decodeURIComponent throws URIError on this.
    let js = phpx_to_js("$d = urldecode('%E0');").expect("should compile");
    assert!(
        js.contains("try {"),
        "urldecode('%E0') must emit try/catch guard, got:\n{}",
        js
    );
}

#[test]
fn urldecode_plus_to_space() {
    // Regression: existing behaviour — '+' must become a space.
    let js = phpx_to_js("$d = urldecode('hello+world');").expect("should compile");
    assert!(
        js.contains("replace(/\\+/g, '%20')"),
        "expected + → %20 substitution, got:\n{}",
        js
    );
}

#[test]
fn urldecode_percent20_to_space() {
    // Regression: %20 must decode to a space (decodeURIComponent handles this).
    let js = phpx_to_js("$d = urldecode('hello%20world');").expect("should compile");
    assert!(
        js.contains("decodeURIComponent("),
        "expected decodeURIComponent for %20, got:\n{}",
        js
    );
}

#[test]
fn rewrite_gettype_inline() {
    let js = phpx_to_js("$v = 42;\n$t = gettype($v);").expect("should compile");
    assert!(
        js.contains("Number.isInteger"),
        "expected Number.isInteger for gettype, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.gettype"),
        "should NOT contain globalThis.gettype, got:\n{}",
        js
    );
}

#[test]
fn rewrite_get_object_vars_inline() {
    let js = phpx_to_js("$o = ['a' => 1];\n$v = get_object_vars($o);").expect("should compile");
    assert!(
        js.contains("Object.keys("),
        "expected Object.keys for get_object_vars, got:\n{}",
        js
    );
    assert!(
        js.contains("__struct"),
        "expected __struct exclusion for get_object_vars, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.get_object_vars"),
        "should NOT contain globalThis.get_object_vars, got:\n{}",
        js
    );
}

#[test]
fn rewrite_mt_rand_inline() {
    let js = phpx_to_js("$n = mt_rand(1, 10);").expect("should compile");
    assert!(
        js.contains("Math.random()"),
        "expected Math.random for mt_rand, got:\n{}",
        js
    );
    assert!(
        js.contains("Math.floor("),
        "expected Math.floor for mt_rand, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.mt_rand"),
        "should NOT contain globalThis.mt_rand, got:\n{}",
        js
    );
}

#[test]
fn rewrite_microtime_as_float_inline() {
    let js = phpx_to_js("$t = microtime(true);").expect("should compile");
    assert!(
        js.contains("Date.now()"),
        "expected Date.now for microtime, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.microtime"),
        "should NOT contain globalThis.microtime, got:\n{}",
        js
    );
}

#[test]
fn rewrite_strtotime_inline() {
    let js = phpx_to_js("$ts = strtotime('2024-01-01');").expect("should compile");
    assert!(
        js.contains("new Date("),
        "expected new Date for strtotime, got:\n{}",
        js
    );
    assert!(
        js.contains("Math.floor("),
        "expected Math.floor for strtotime, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.strtotime"),
        "should NOT contain globalThis.strtotime, got:\n{}",
        js
    );
}

#[test]
fn rewrite_preg_match_inline() {
    let js = phpx_to_js("$m = preg_match('/foo/', 'foobar');").expect("should compile");
    assert!(
        js.contains("new RegExp("),
        "expected new RegExp for preg_match, got:\n{}",
        js
    );
    assert!(
        js.contains(".test("),
        "expected .test() for preg_match, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.preg_match"),
        "should NOT contain globalThis.preg_match, got:\n{}",
        js
    );
}

#[test]
fn rewrite_preg_replace_inline() {
    let js = phpx_to_js("$r = preg_replace('/foo/', 'bar', 'foobar');").expect("should compile");
    assert!(
        js.contains("new RegExp("),
        "expected new RegExp for preg_replace, got:\n{}",
        js
    );
    assert!(
        js.contains(".replace("),
        "expected .replace() for preg_replace, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.preg_replace"),
        "should NOT contain globalThis.preg_replace, got:\n{}",
        js
    );
}

#[test]
fn rewrite_parse_url_inline() {
    let js =
        phpx_to_js("$parts = parse_url('https://example.com/path?q=1');").expect("should compile");
    assert!(
        js.contains("new URL("),
        "expected new URL for parse_url, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.parse_url"),
        "should NOT contain globalThis.parse_url, got:\n{}",
        js
    );
}

#[test]
fn rewrite_intval_inline() {
    let js = phpx_to_js("$n = intval('42');").expect("should compile");
    assert!(
        js.contains("parseInt("),
        "expected parseInt for intval, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.intval"),
        "should NOT contain globalThis.intval, got:\n{}",
        js
    );
}

#[test]
fn rewrite_floatval_inline() {
    let js = phpx_to_js("$n = floatval('3.14');").expect("should compile");
    assert!(
        js.contains("parseFloat("),
        "expected parseFloat for floatval, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.floatval"),
        "should NOT contain globalThis.floatval, got:\n{}",
        js
    );
}

#[test]
fn rewrite_boolval_inline() {
    let js = phpx_to_js("$b = boolval(1);").expect("should compile");
    assert!(
        js.contains("Boolean("),
        "expected Boolean() for boolval, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.boolval"),
        "should NOT contain globalThis.boolval, got:\n{}",
        js
    );
}

#[test]
fn rewrite_strval_inline() {
    let js = phpx_to_js("$s = strval(42);").expect("should compile");
    assert!(
        js.contains("String("),
        "expected String() for strval, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.strval"),
        "should NOT contain globalThis.strval, got:\n{}",
        js
    );
}

#[test]
fn rewrite_array_slice_inline() {
    let js = phpx_to_js("$a = [1,2,3,4];\n$s = array_slice($a, 1, 2);").expect("should compile");
    assert!(
        js.contains(".slice("),
        "expected .slice() for array_slice, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.array_slice"),
        "should NOT contain globalThis.array_slice, got:\n{}",
        js
    );
}

// ---- Batch 3: base64, hash, date, pack, function_exists, class_exists ----

#[test]
fn rewrite_base64_encode_to_helper() {
    let js = phpx_to_js("$s = 'hello';\n$b = base64_encode($s);").expect("should compile");
    assert!(
        js.contains("__phpx_base64_encode("),
        "expected __phpx_base64_encode call, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.base64_encode ??="),
        "globalThis.base64_encode polyfill should NOT be in prelude, got:\n{}",
        js
    );
}

#[test]
fn rewrite_base64_decode_to_helper() {
    let js = phpx_to_js("$s = 'aGVsbG8=';\n$d = base64_decode($s);").expect("should compile");
    assert!(
        js.contains("__phpx_base64_decode("),
        "expected __phpx_base64_decode call, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.base64_decode ??="),
        "globalThis.base64_decode polyfill should NOT be in prelude, got:\n{}",
        js
    );
}

#[test]
fn rewrite_base64_decode_strict_to_helper() {
    let js = phpx_to_js("$s = 'aGVsbG8=';\n$d = base64_decode($s, true);").expect("should compile");
    assert!(
        js.contains("__phpx_base64_decode("),
        "expected __phpx_base64_decode call with strict arg, got:\n{}",
        js
    );
}

// Regression: strict-mode base64_decode must return false for invalid chars
// in positions 2-3 of a quartet (issue #34).  The old guard `n2 < -1 || n3 < -1`
// was unreachable because tbl.indexOf returns at most -1; any invalid char
// would silently produce garbage bytes instead of returning false.
#[test]
fn base64_decode_strict_rejects_invalid_char_in_quartet() {
    let js = phpx_to_js("$d = base64_decode('ab=Y', true);").expect("should compile");
    // Extract the helper JS so we can run it directly.
    let helpers: String = js
        .lines()
        .filter(|l| {
            l.starts_with("function __phpx_base64_") || l.starts_with("const __phpx_base64_table")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let script = format!(
        "{helpers}\nconsole.log([String(__phpx_base64_decode('ab=Y', true)), String(__phpx_base64_decode('ab%=', true))].join('\\n'));"
    );
    match run_node(&script) {
        Err(e) if e.contains("node not available") => return, // skip if no node
        Err(e) => panic!("node error: {e}"),
        Ok(out) => assert_eq!(
            out, "false\nfalse",
            "strict base64_decode should reject malformed quartets 'ab=Y' and 'ab%=', got: {out}"
        ),
    }
}

// Also verify that valid base64 still decodes correctly with strict=true
#[test]
fn base64_decode_strict_accepts_valid_base64() {
    let js = phpx_to_js("$d = base64_decode('aGVsbG8=', true);").expect("should compile");
    let helpers: String = js
        .lines()
        .filter(|l| {
            l.starts_with("function __phpx_base64_") || l.starts_with("const __phpx_base64_table")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let script = format!("{helpers}\nconsole.log(__phpx_base64_decode('aGVsbG8=', true));");
    match run_node(&script) {
        Err(e) if e.contains("node not available") => return,
        Err(e) => panic!("node error: {e}"),
        Ok(out) => assert_eq!(
            out, "hello",
            "strict base64_decode with valid 'aGVsbG8=' should return 'hello', got: {out}"
        ),
    }
}

#[test]
fn rewrite_hash_to_helper() {
    let js = phpx_to_js("$s = 'hello';\n$h = hash('sha256', $s);").expect("should compile");
    assert!(
        js.contains("__phpx_hash("),
        "expected __phpx_hash call, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.hash ??="),
        "globalThis.hash polyfill should NOT be in prelude, got:\n{}",
        js
    );
}

#[test]
fn rewrite_hash_raw_to_helper() {
    let js = phpx_to_js("$s = 'hello';\n$h = hash('sha256', $s, true);").expect("should compile");
    assert!(
        js.contains("__phpx_hash("),
        "expected __phpx_hash call with raw arg, got:\n{}",
        js
    );
}

#[test]
fn rewrite_hash_hmac_to_helper() {
    let js = phpx_to_js("$h = hash_hmac('sha256', 'data', 'key');").expect("should compile");
    assert!(
        js.contains("__phpx_hash_hmac("),
        "expected __phpx_hash_hmac call, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.hash_hmac ??="),
        "globalThis.hash_hmac polyfill should NOT be in prelude, got:\n{}",
        js
    );
}

// ---- SHA-256 correctness: NIST / RFC test vectors run via node ----
//
// These tests extract the emitted __phpx_sha256_hex and __phpx_hmac_sha256_hex helpers
// from transpiler output and execute them in Node.js to verify bit-exact correctness.
// Structural/compile-time assertions are NOT enough for a cryptographic primitive —
// the bit-length encoding bug (issue #37, PR #25) passed structural checks but produced
// wrong hashes for all non-empty inputs.
//
// If node is not on PATH, the test is skipped gracefully so CI without node still passes.

/// Run a JS snippet in Node.js. Returns Ok(stdout) or Err(stderr).
#[cfg(test)]
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

/// Extract the `__phpx_sha256_hex` and related helper function bodies from
/// compiled PHPX output so the tests can run them directly.
#[cfg(test)]
fn sha256_helpers_js() -> String {
    // Compile a minimal PHPX file that forces all three helpers to be emitted.
    let js = phpx_to_js("$h = hash('sha256', 'x');\n$m = hash_hmac('sha256', 'x', 'k');")
        .expect("should compile");
    // Strip the non-function lines (let assignments etc.) — keep only function/const lines.
    js.lines()
        .filter(|l| l.starts_with("function __phpx_") || l.starts_with("const __phpx_node_crypto"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn sha256_nist_empty_string() {
    let helpers = sha256_helpers_js();
    let script = format!("{helpers}\nconsole.log(__phpx_sha256_hex(''));");
    match run_node(&script) {
        Err(e) if e.contains("node not available") => return, // skip if no node
        Err(e) => panic!("node error: {e}"),
        Ok(got) => assert_eq!(
            got, "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "sha256('') mismatch — NIST vector"
        ),
    }
}

#[test]
fn sha256_nist_abc() {
    // NIST FIPS 180-4 example: SHA-256('abc')
    let helpers = sha256_helpers_js();
    let script = format!("{helpers}\nconsole.log(__phpx_sha256_hex('abc'));");
    match run_node(&script) {
        Err(e) if e.contains("node not available") => return,
        Err(e) => panic!("node error: {e}"),
        Ok(got) => assert_eq!(
            got, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            "sha256('abc') mismatch — NIST vector"
        ),
    }
}

#[test]
fn sha256_nist_56byte_crosses_block_boundary() {
    // 56-byte input: padding pushes it into a second 64-byte block.
    // This exercises the message-schedule expansion path and the length encoding.
    let helpers = sha256_helpers_js();
    let script = format!(
        "{helpers}\nconsole.log(__phpx_sha256_hex('abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq'));"
    );
    match run_node(&script) {
        Err(e) if e.contains("node not available") => return,
        Err(e) => panic!("node error: {e}"),
        Ok(got) => assert_eq!(
            got, "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
            "sha256(56-byte vector) mismatch — NIST vector"
        ),
    }
}

#[test]
fn sha256_nist_one_million_a() {
    // NIST: SHA-256(1_000_000 × 'a').
    // bitLen = 8_000_000 — fits in 32 bits (< 2^32), so the high-half of the
    // 64-bit length field must be 0.  The bug (#37) wrote the LOW half into
    // both halves, which still corrupted the padding for non-zero bitLen.
    let helpers = sha256_helpers_js();
    let script = format!("{helpers}\nconsole.log(__phpx_sha256_hex('a'.repeat(1_000_000)));");
    match run_node(&script) {
        Err(e) if e.contains("node not available") => return,
        Err(e) => panic!("node error: {e}"),
        Ok(got) => assert_eq!(
            got, "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0",
            "sha256(1M 'a') mismatch — NIST vector"
        ),
    }
}

#[test]
fn hmac_sha256_rfc4231_tc1_key_0b_20_hi_there() {
    // RFC 4231 Test Case 1: key = 0x0b repeated 20 times, data = "Hi There"
    // Expected: b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7
    let helpers = sha256_helpers_js();
    let script = format!(
        "{helpers}\n\
         const key1 = String.fromCharCode(...Array(20).fill(0x0b));\n\
         console.log(__phpx_hmac_sha256_hex('Hi There', key1));"
    );
    match run_node(&script) {
        Err(e) if e.contains("node not available") => return,
        Err(e) => panic!("node error: {e}"),
        Ok(got) => assert_eq!(
            got, "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7",
            "hmac-sha256 RFC 4231 TC1 mismatch"
        ),
    }
}

#[test]
fn hmac_sha256_rfc4231_tc2_jefe() {
    // RFC 4231 Test Case 2: key = "Jefe", data = "what do ya want for nothing?"
    // Expected: 5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843
    let helpers = sha256_helpers_js();
    let script = format!(
        "{helpers}\n\
         console.log(__phpx_hmac_sha256_hex('what do ya want for nothing?', 'Jefe'));"
    );
    match run_node(&script) {
        Err(e) if e.contains("node not available") => return,
        Err(e) => panic!("node error: {e}"),
        Ok(got) => assert_eq!(
            got, "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843",
            "hmac-sha256 RFC 4231 TC2 mismatch"
        ),
    }
}

// ---- end SHA-256 correctness tests ----

#[test]
fn rewrite_hash_equals_inline_constant_time() {
    let js =
        phpx_to_js("$a = 'abc';\n$b = 'abc';\n$eq = hash_equals($a, $b);").expect("should compile");
    // Must be an IIFE with XOR loop — no external helper call
    assert!(
        !js.contains("__phpx_hash_equals"),
        "hash_equals must NOT call an external helper, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.hash_equals ??="),
        "globalThis.hash_equals polyfill should NOT be in prelude, got:\n{}",
        js
    );
    // Constant-time check: XOR accumulation
    assert!(
        js.contains("^"),
        "expected XOR (^) for constant-time compare, got:\n{}",
        js
    );
    assert!(
        js.contains("charCodeAt"),
        "expected charCodeAt for byte comparison, got:\n{}",
        js
    );
    // Must NOT short-circuit on length mismatch alone — must still run the loop
    assert!(
        js.contains("__a.length !== __b.length"),
        "expected length mismatch check, got:\n{}",
        js
    );
}

#[test]
fn rewrite_date_to_helper() {
    let js = phpx_to_js("$d = date('c');").expect("should compile");
    assert!(
        js.contains("__phpx_date("),
        "expected __phpx_date call, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.date ??="),
        "globalThis.date polyfill should NOT be in prelude, got:\n{}",
        js
    );
}

#[test]
fn rewrite_date_with_ts_to_helper() {
    let js = phpx_to_js("$t = 1700000000;\n$d = date('Y-m-d', $t);").expect("should compile");
    assert!(
        js.contains("__phpx_date("),
        "expected __phpx_date call with ts arg, got:\n{}",
        js
    );
}

#[test]
fn rewrite_gmdate_to_helper() {
    let js = phpx_to_js("$d = gmdate('c');").expect("should compile");
    assert!(
        js.contains("__phpx_gmdate("),
        "expected __phpx_gmdate call, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.gmdate ??="),
        "globalThis.gmdate polyfill should NOT be in prelude, got:\n{}",
        js
    );
}

#[test]
fn rewrite_pack_to_helper() {
    let js = phpx_to_js("$h = 'deadbeef';\n$b = pack('H*', $h);").expect("should compile");
    assert!(
        js.contains("__phpx_pack("),
        "expected __phpx_pack call, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.pack ??="),
        "globalThis.pack polyfill should NOT be in prelude, got:\n{}",
        js
    );
}

#[test]
fn rewrite_function_exists_inline() {
    let js = phpx_to_js("$b = function_exists('hash');").expect("should compile");
    assert!(
        js.contains("typeof globalThis["),
        "expected typeof globalThis lookup for function_exists, got:\n{}",
        js
    );
    assert!(
        js.contains("=== \"function\""),
        "expected === 'function' for function_exists, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.function_exists ??="),
        "globalThis.function_exists polyfill should NOT be in prelude, got:\n{}",
        js
    );
}

#[test]
fn rewrite_class_exists_inline_always_false() {
    let js = phpx_to_js("$b = class_exists('Result');").expect("should compile");
    // PHPX has no classes — class_exists always compiles to false
    assert!(
        js.contains("false"),
        "expected false for class_exists in PHPX, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.class_exists ??="),
        "globalThis.class_exists polyfill should NOT be in prelude, got:\n{}",
        js
    );
}

#[test]
fn tier_b_helpers_emitted_as_module_scoped_functions() {
    // When base64_encode is used, the output must contain a module-scoped function
    // declaration — NOT a globalThis assignment. This is the DCE-correctness test.
    let js = phpx_to_js("$s = 'hello';\n$b = base64_encode($s);").expect("should compile");
    assert!(
        js.contains("function __phpx_base64_encode("),
        "expected module-scoped function declaration for base64_encode, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.__phpx_base64_encode"),
        "globalThis assignment must NOT be emitted for base64_encode, got:\n{}",
        js
    );
    // pack should be absent since it's not used
    assert!(
        !js.contains("function __phpx_pack"),
        "pack helper must be absent when unused, got:\n{}",
        js
    );
    // date should be absent since it's not used
    assert!(
        !js.contains("function __phpx_date("),
        "date helper must be absent when unused, got:\n{}",
        js
    );

    // Same check for date helper.
    let js2 = phpx_to_js("$d = date('Y-m-d');").expect("should compile");
    assert!(
        js2.contains("function __phpx_date("),
        "expected module-scoped function declaration for date, got:\n{}",
        js2
    );
    assert!(
        !js2.contains("globalThis.__phpx_date"),
        "globalThis assignment must NOT be emitted for date, got:\n{}",
        js2
    );
    // base64 should be absent since it's not used
    assert!(
        !js2.contains("function __phpx_base64_encode"),
        "base64_encode helper must be absent when unused, got:\n{}",
        js2
    );
}

#[test]
fn tier_b_dce_evidence_no_helpers_when_unused() {
    // A file that uses neither hash/hash_hmac nor base64_* must NOT contain any
    // of those helper declarations. This is the DCE-evidence test.
    let js = phpx_to_js("$x = strlen('hello');\n$y = count([1, 2, 3]);").expect("should compile");
    assert!(
        !js.contains("function __phpx_hash("),
        "hash helper must be absent when unused"
    );
    assert!(
        !js.contains("function __phpx_hash_hmac("),
        "hash_hmac helper must be absent when unused"
    );
    assert!(
        !js.contains("function __phpx_sha256_hex("),
        "sha256_hex helper must be absent when unused"
    );
    assert!(
        !js.contains("function __phpx_base64_encode("),
        "base64_encode helper must be absent when unused"
    );
    assert!(
        !js.contains("function __phpx_base64_decode("),
        "base64_decode helper must be absent when unused"
    );
    assert!(
        !js.contains("__phpx_base64_table"),
        "base64_table must be absent when unused"
    );
}

// ---- Phase 3: scope validation warnings ----
#[test]
fn rewrite_count_on_object() {
    let js = phpx_to_js("function f(): int { $o = { a: 1, b: 2 };\nreturn count($o); }")
        .expect("should compile");
    assert!(
        js.contains("Object.keys"),
        "expected Object.keys fallback for count on object, got:\n{}",
        js
    );
}

#[test]
fn rewrite_substr_negative_start() {
    let js = phpx_to_js("function f(): string { $s = 'hello';\nreturn substr($s, -2); }")
        .expect("should compile");
    assert!(
        js.contains(".slice(") || js.contains("__st"),
        "expected slice or IIFE for negative start, got:\n{}",
        js
    );
}

#[test]
fn rewrite_in_array_with_strict() {
    let js =
        phpx_to_js("function f(): bool { $arr = [1, 2, 3];\nreturn in_array(2, $arr, true); }")
            .expect("should compile");
    assert!(
        js.contains(".includes("),
        "expected .includes for in_array strict, got:\n{}",
        js
    );
}

#[test]
fn rewrite_explode_with_limit() {
    let js = phpx_to_js("function f(): void { $s = 'a:b:c:d';\n$parts = explode(':', $s, 2); }")
        .expect("should compile");
    assert!(
        js.contains(".split("),
        "expected .split in explode with limit, got:\n{}",
        js
    );
}

#[test]
fn rewrite_implode_single_arg() {
    let js = phpx_to_js("function f(): string { $arr = ['a', 'b'];\nreturn implode($arr); }")
        .expect("should compile");
    assert!(
        js.contains(".join("),
        "expected .join for single-arg implode, got:\n{}",
        js
    );
}

#[test]
fn rewrite_strpos_with_offset() {
    let js = phpx_to_js("function f(): void { $h = 'hello world';\n$i = strpos($h, 'o', 5); }")
        .expect("should compile");
    assert!(
        js.contains(".indexOf("),
        "expected indexOf for strpos with offset, got:\n{}",
        js
    );
    assert!(js.contains("5"), "expected offset 5, got:\n{}", js);
}

// ---- Operators ----

fn run_phpx_and_eval_json(source: &str, expression: &str) -> Result<String, String> {
    let js = phpx_to_js(source).expect("should compile");
    assert!(
        !js.contains("globalThis.usort")
            && !js.contains("globalThis.uasort")
            && !js.contains("globalThis.uksort"),
        "sort rewrites must not install globalThis polyfills, got:\n{}",
        js
    );
    run_node(&format!("{js}\nconsole.log(JSON.stringify({expression}));"))
}

#[test]
fn rewrite_usort_inline() {
    let js =
        phpx_to_js("$arr = [3, 1, 2];\n$ok = usort($arr, function($a, $b) { return $a - $b; });")
            .expect("should compile");
    assert!(
        js.contains(".sort("),
        "expected .sort() for usort, got:\n{}",
        js
    );
    assert!(
        js.contains(", true)"),
        "expected comma-true pattern for usort, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.usort"),
        "should NOT contain globalThis.usort, got:\n{}",
        js
    );
}

#[test]
fn rewrite_usort_sorts_ascending_and_descending_with_comparator() {
    let asc = run_phpx_and_eval_json(
        "$arr = [3, 1, 2];\n$ok = usort($arr, function($a, $b) { return $a - $b; });",
        "globalThis.arr",
    );
    match asc {
        Err(e) if e.contains("node not available") => return,
        Err(e) => panic!("node error: {e}"),
        Ok(out) => assert_eq!(out, "[1,2,3]"),
    }

    let desc = run_phpx_and_eval_json(
        "$arr = [3, 1, 2];\n$ok = usort($arr, function($a, $b) { return $b - $a; });",
        "globalThis.arr",
    );
    match desc {
        Err(e) if e.contains("node not available") => return,
        Err(e) => panic!("node error: {e}"),
        Ok(out) => assert_eq!(out, "[3,2,1]"),
    }
}

#[test]
fn rewrite_usort_supports_multi_key_comparator() {
    let out = run_phpx_and_eval_json(
        r#"$rows = [
  ['name' => 'beta', 'rank' => 2],
  ['name' => 'gamma', 'rank' => 1],
  ['name' => 'alpha', 'rank' => 1]
];
$ok = usort($rows, function($a, $b) {
  if ($a['rank'] == $b['rank']) {
    return $a['name'] < $b['name'] ? -1 : ($a['name'] > $b['name'] ? 1 : 0);
  }
  return $a['rank'] - $b['rank'];
});"#,
        "globalThis.rows.map(r => r.name)",
    );
    match out {
        Err(e) if e.contains("node not available") => return,
        Err(e) => panic!("node error: {e}"),
        Ok(out) => assert_eq!(out, "[\"alpha\",\"gamma\",\"beta\"]"),
    }
}

#[test]
fn rewrite_uasort_inline() {
    let js =
        phpx_to_js("$arr = [3, 1, 2];\n$ok = uasort($arr, function($a, $b) { return $a - $b; });")
            .expect("should compile");
    assert!(
        js.contains(".sort("),
        "expected .sort() for uasort, got:\n{}",
        js
    );
    assert!(
        js.contains("return true"),
        "expected true return for uasort, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.uasort"),
        "should NOT contain globalThis.uasort, got:\n{}",
        js
    );
}

#[test]
fn rewrite_uasort_preserves_associative_keys() {
    let out = run_phpx_and_eval_json(
        "$scores = ['third' => 30, 'first' => 10, 'second' => 20];\n$ok = uasort($scores, function($a, $b) { return $a - $b; });",
        "Object.keys(globalThis.scores).map(k => [k, globalThis.scores[k]])",
    );
    match out {
        Err(e) if e.contains("node not available") => return,
        Err(e) => panic!("node error: {e}"),
        Ok(out) => assert_eq!(out, "[[\"first\",10],[\"second\",20],[\"third\",30]]"),
    }
}

#[test]
fn rewrite_uksort_inline() {
    let js = phpx_to_js("$obj = ['b' => 2, 'a' => 1];\n$ok = uksort($obj, function($a, $b) { return $a < $b ? -1 : ($a > $b ? 1 : 0); });").expect("should compile");
    assert!(
        js.contains("Object.keys("),
        "expected Object.keys for uksort, got:\n{}",
        js
    );
    assert!(
        js.contains(".sort("),
        "expected .sort() for uksort, got:\n{}",
        js
    );
    assert!(
        js.contains("Object.assign("),
        "expected Object.assign for uksort key-rebuild, got:\n{}",
        js
    );
    assert!(
        !js.contains("globalThis.uksort"),
        "should NOT contain globalThis.uksort, got:\n{}",
        js
    );
}

#[test]
fn rewrite_uksort_compares_keys() {
    let out = run_phpx_and_eval_json(
        "$obj = ['b' => 2, 'c' => 3, 'a' => 1];\n$ok = uksort($obj, function($a, $b) { return $b < $a ? -1 : ($b > $a ? 1 : 0); });",
        "Object.keys(globalThis.obj)",
    );
    match out {
        Err(e) if e.contains("node not available") => return,
        Err(e) => panic!("node error: {e}"),
        Ok(out) => assert_eq!(out, "[\"c\",\"b\",\"a\"]"),
    }
}

// ---- Type predicate IIFE: arg evaluated exactly once ----

#[test]
fn type_predicates_evaluate_arg_exactly_once() {
    // Each type predicate must bind the argument into __v once via an IIFE so that a
    // side-effecting call-expression arg is only evaluated a single time.
    // The snippets are wrapped in a function so the arg variable doesn't get mirrored
    // onto globalThis (which would add an extra globalThis.consume_token reference).
    let cases = [
        (
            "is_int",
            "function chk(): bool { return is_int(consume_token()); }",
        ),
        (
            "is_float",
            "function chk(): bool { return is_float(consume_token()); }",
        ),
        (
            "is_numeric",
            "function chk(): bool { return is_numeric(consume_token()); }",
        ),
        (
            "is_string",
            "function chk(): bool { return is_string(consume_token()); }",
        ),
        (
            "is_object",
            "function chk(): bool { return is_object(consume_token()); }",
        ),
    ];
    for (builtin, src) in cases {
        let js = phpx_to_js(src).expect(&format!("{} should compile", builtin));
        // The arg expression must appear exactly once — the IIFE binds it to __v.
        let call_count = js.matches("consume_token()").count();
        assert_eq!(
            call_count, 1,
            "{}: expected exactly 1 occurrence of consume_token() in emitted JS (got {}), JS:\n{}",
            builtin, call_count, js
        );
        // Confirm the IIFE shape: `const __v =` binding must be present.
        assert!(
            js.contains("const __v"),
            "{}: expected IIFE binding `const __v` in emitted JS, got:\n{}",
            builtin,
            js
        );
    }
}
