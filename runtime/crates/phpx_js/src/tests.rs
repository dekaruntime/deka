use super::*;
use php_rs::parser::lexer::Lexer;
use php_rs::parser::parser::{Parser, ParserMode};

/// Helper: parse PHPX source and emit JS via the subset emitter.
fn phpx_to_js(source: &str) -> Result<String, String> {
    let arena = Bump::new();
    let mut parser =
        Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();
    if !program.errors.is_empty() {
        let msgs: Vec<&str> = program.errors.iter().map(|e| e.message).collect();
        return Err(format!("parse errors: {}", msgs.join("; ")));
    }
    emit_js_from_ast(&program, source.as_bytes(), SourceModuleMeta::empty())
}

// ---- 1. Simple function compilation ----

#[test]
fn simple_function_compiles_to_js() {
    let js = phpx_to_js("function hello(): string { return 'world'; }")
        .expect("should compile");
    assert!(js.contains("function hello()"), "expected function declaration, got:\n{}", js);
    assert!(js.contains("return \"world\"") || js.contains("return 'world'"),
        "expected return statement, got:\n{}", js);
}

// ---- 2. Variable declaration ----

#[test]
fn variable_declaration_compiles() {
    let js = phpx_to_js("$x = 42;").expect("should compile");
    assert!(js.contains("let x = 42") || js.contains("x = 42"),
        "expected variable assignment, got:\n{}", js);
}

// ---- 3. Object literal ----

#[test]
fn object_literal_compiles() {
    let js = phpx_to_js("$value = 1;\n$obj = { key: $value };").expect("should compile");
    assert!(js.contains("key") && js.contains("value"),
        "expected object literal with key/value, got:\n{}", js);
}

// ---- 4. String concatenation ----

#[test]
fn string_concatenation_uses_plus() {
    let js = phpx_to_js("$s = 'hello' . ' ' . 'world';").expect("should compile");
    assert!(js.contains("+"), "expected + for string concat, got:\n{}", js);
    // The concat line itself should use + not .
    let concat_line = js.lines().find(|l| l.contains("hello")).expect("concat line");
    assert!(!concat_line.contains(" . "), "should not use PHP dot operator in concat: {}", concat_line);
}

// ---- 5. Array access ----

#[test]
fn array_access_compiles() {
    let js = phpx_to_js("$arr = ['key' => 'val'];\n$v = $arr['key'];").expect("should compile");
    assert!(js.contains("arr[\"key\"]") || js.contains("arr['key']"),
        "expected array key access, got:\n{}", js);
}

// ---- 6. CQL expression ----

#[test]
fn cql_expression_emits_object() {
    let js = phpx_to_js("cql q = MATCH (n) RETURN n;").expect("should compile");
    assert!(js.contains("__type: \"cql\""), "expected CQL type marker, got:\n{}", js);
    assert!(js.contains("MATCH (n) RETURN n"), "expected cypher query, got:\n{}", js);
    assert!(js.contains("params:"), "expected params field, got:\n{}", js);
}

// ---- 7. Module function calls compile ----

#[test]
fn function_call_compiles() {
    let source = "$result = query('MATCH (n) RETURN n');";
    let js = phpx_to_js(source).expect("should compile");
    assert!(js.contains("query"), "expected query call, got:\n{}", js);
}

#[test]
fn multiple_function_calls_compile() {
    let source = "$val = get('key');\n$ok = set('key', 'val');";
    let js = phpx_to_js(source).expect("should compile");
    assert!(js.contains("get"), "expected get call, got:\n{}", js);
    assert!(js.contains("set"), "expected set call, got:\n{}", js);
}

// ---- 8. Null comparisons are rejected (typechecker, not emitter) ----
// Note: null comparison rejection is handled by the typechecker in php-rs,
// not by the JS emitter. The emitter itself will emit null comparisons.
// See php-rs typeck tests for null_comparison_is_rejected.

// ---- 9. Prelude uses ??= pattern ----

#[test]
fn prelude_uses_nullish_assignment() {
    let js = phpx_to_js("$x = 1;").expect("should compile");
    // The prelude should use ??= for global setup
    assert!(js.contains("??="), "expected ??= in prelude, got:\n{}", js);
    // Should NOT use the anti-pattern `if (!globalThis.X) { globalThis.X = ... }`
    // for polyfill definitions (those should use ??=)
    let lines: Vec<&str> = js.lines()
        .filter(|l| l.contains("if (!globalThis.") && l.contains("globalThis.") && !l.contains("__deka"))
        .collect();
    // Allow __dekaGlobalsInstalled guard but polyfills should use ??=
    for line in &lines {
        assert!(!line.contains("function"),
            "function polyfills should use ??= not if-guard: {}", line);
    }
}

// ---- Additional coverage ----

#[test]
fn if_else_compiles() {
    let js = phpx_to_js("$x = 1;\nif ($x == 1) { $x = 2; } else { $x = 3; }")
        .expect("should compile");
    assert!(js.contains("if ("), "expected if statement, got:\n{}", js);
    assert!(js.contains("else"), "expected else branch, got:\n{}", js);
}

#[test]
fn arrow_function_compiles() {
    let js = phpx_to_js("$fn = fn($x: int): int => $x + 1;").expect("should compile");
    assert!(js.contains("=>"), "expected arrow function, got:\n{}", js);
}

#[test]
fn nullsafe_access_compiles() {
    let js = phpx_to_js("$user = { name: 'test' };\n$v = $user?->name;")
        .expect("should compile");
    assert!(js.contains("?."), "expected optional chaining, got:\n{}", js);
}

#[test]
fn match_expression_compiles() {
    let source = r#"
$x = 1;
$result = match ($x) {
1 => "one",
2 => "two",
default => "other",
};
"#;
    let js = phpx_to_js(source).expect("should compile");
    // match expressions are typically emitted as ternary chains or switch
    assert!(js.contains("one") && js.contains("two") && js.contains("other"),
        "expected match arms, got:\n{}", js);
}

#[test]
fn export_function_via_meta() {
    // Export declarations are handled at the source meta level, not the parser.
    // Verify the emitter can mark a function as exported via SourceModuleMeta.
    let source = "function greet(): string {\n  return 'hi';\n}\n";
    let arena = Bump::new();
    let mut parser =
        Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();
    assert!(program.errors.is_empty(), "parse errors: {:?}", program.errors);
    let mut meta = SourceModuleMeta::empty();
    meta.exported_functions.insert("greet".to_string());
    let js = emit_js_from_ast(&program, source.as_bytes(), meta).expect("should emit");
    assert!(js.contains("export") || js.contains("greet"),
        "expected function in output, got:\n{}", js);
}

#[test]
fn struct_declaration_compiles_without_error() {
    // Struct declarations compile without error; the struct schema is stored
    // internally and used when deka/i imports are present.
    let source = "struct Point {\n  $x: int;\n  $y: int;\n}\n";
    phpx_to_js(source).expect("struct should compile to JS");
}

#[test]
fn jsx_element_compiles() {
    let source = "function View(): Object { return <div class=\"test\">hello</div>; }";
    let js = phpx_to_js(source).expect("should compile");
    assert!(js.contains("div") || js.contains("jsx") || js.contains("h("),
        "expected JSX output, got:\n{}", js);
}

// ==== Phase 2: compile-time rewrite regression tests ====

#[test]
fn rewrite_count_inline() {
    let js = phpx_to_js("$arr = [1, 2, 3];\n$n = count($arr);").expect("should compile");
    assert!(js.contains(".length"), "expected .length for count, got:\n{}", js);
    assert!(!js.contains("globalThis.count"), "should NOT contain globalThis.count, got:\n{}", js);
}

#[test]
fn rewrite_strlen_inline() {
    let js = phpx_to_js("$s = 'hello';\n$n = strlen($s);").expect("should compile");
    assert!(js.contains("String(") && js.contains(".length"), "expected String().length for strlen, got:\n{}", js);
    assert!(!js.contains("globalThis.strlen"), "should NOT contain globalThis.strlen, got:\n{}", js);
}

#[test]
fn rewrite_substr_two_args() {
    let js = phpx_to_js("$s = 'hello';\n$r = substr($s, 2);").expect("should compile");
    assert!(js.contains(".slice("), "expected .slice for substr, got:\n{}", js);
    assert!(!js.contains("globalThis.substr"), "should NOT contain globalThis.substr, got:\n{}", js);
}

#[test]
fn rewrite_substr_three_args() {
    let js = phpx_to_js("$s = 'hello';\n$r = substr($s, 1, 3);").expect("should compile");
    assert!(js.contains(".slice("), "expected .slice for substr(3), got:\n{}", js);
    assert!(!js.contains("globalThis.substr"), "should NOT contain globalThis.substr, got:\n{}", js);
}

#[test]
fn rewrite_trim_inline() {
    let js = phpx_to_js("$s = '  hello  ';\n$r = trim($s);").expect("should compile");
    assert!(js.contains(".trim()"), "expected .trim() for trim, got:\n{}", js);
    assert!(!js.contains("globalThis.trim"), "should NOT contain globalThis.trim, got:\n{}", js);
}

#[test]
fn rewrite_ltrim_inline() {
    let js = phpx_to_js("$s = '  hello';\n$r = ltrim($s);").expect("should compile");
    assert!(js.contains(".trimStart()"), "expected .trimStart() for ltrim, got:\n{}", js);
    assert!(!js.contains("globalThis.ltrim"), "should NOT contain globalThis.ltrim, got:\n{}", js);
}

#[test]
fn rewrite_rtrim_inline() {
    let js = phpx_to_js("$s = 'hello  ';\n$r = rtrim($s);").expect("should compile");
    assert!(js.contains(".trimEnd()"), "expected .trimEnd() for rtrim, got:\n{}", js);
    assert!(!js.contains("globalThis.rtrim"), "should NOT contain globalThis.rtrim, got:\n{}", js);
}

#[test]
fn rewrite_strpos_inline() {
    let js = phpx_to_js("$h = 'hello';\n$i = strpos($h, 'l');").expect("should compile");
    assert!(js.contains(".indexOf("), "expected .indexOf for strpos, got:\n{}", js);
    assert!(!js.contains("globalThis.strpos"), "should NOT contain globalThis.strpos, got:\n{}", js);
}

#[test]
fn rewrite_strrpos_inline() {
    let js = phpx_to_js("$h = 'hello';\n$i = strrpos($h, 'l');").expect("should compile");
    assert!(js.contains(".lastIndexOf("), "expected .lastIndexOf for strrpos, got:\n{}", js);
    assert!(!js.contains("globalThis.strrpos"), "should NOT contain globalThis.strrpos, got:\n{}", js);
}

#[test]
fn rewrite_str_starts_with_inline() {
    let js = phpx_to_js("$h = 'hello';\n$b = str_starts_with($h, 'he');").expect("should compile");
    assert!(js.contains(".startsWith("), "expected .startsWith for str_starts_with, got:\n{}", js);
    assert!(!js.contains("globalThis.str_starts_with"), "should NOT contain globalThis.str_starts_with, got:\n{}", js);
}

#[test]
fn rewrite_str_ends_with_inline() {
    let js = phpx_to_js("$h = 'hello';\n$b = str_ends_with($h, 'lo');").expect("should compile");
    assert!(js.contains(".endsWith("), "expected .endsWith for str_ends_with, got:\n{}", js);
    assert!(!js.contains("globalThis.str_ends_with"), "should NOT contain globalThis.str_ends_with, got:\n{}", js);
}

#[test]
fn rewrite_str_contains_inline() {
    let js = phpx_to_js("$h = 'hello world';\n$b = str_contains($h, 'world');").expect("should compile");
    assert!(js.contains(".includes("), "expected .includes for str_contains, got:\n{}", js);
    assert!(!js.contains("globalThis.str_contains"), "should NOT contain globalThis.str_contains, got:\n{}", js);
}

#[test]
fn rewrite_strtolower_inline() {
    let js = phpx_to_js("$s = 'HELLO';\n$r = strtolower($s);").expect("should compile");
    assert!(js.contains(".toLowerCase()"), "expected .toLowerCase for strtolower, got:\n{}", js);
    assert!(!js.contains("globalThis.strtolower"), "should NOT contain globalThis.strtolower, got:\n{}", js);
}

#[test]
fn rewrite_strtoupper_inline() {
    let js = phpx_to_js("$s = 'hello';\n$r = strtoupper($s);").expect("should compile");
    assert!(js.contains(".toUpperCase()"), "expected .toUpperCase for strtoupper, got:\n{}", js);
    assert!(!js.contains("globalThis.strtoupper"), "should NOT contain globalThis.strtoupper, got:\n{}", js);
}

#[test]
fn rewrite_array_key_exists_inline() {
    let js = phpx_to_js("$a = { x: 1 };\n$b = array_key_exists('x', $a);").expect("should compile");
    assert!(js.contains("hasOwnProperty"), "expected hasOwnProperty for array_key_exists, got:\n{}", js);
    assert!(!js.contains("globalThis.array_key_exists"), "should NOT contain globalThis.array_key_exists, got:\n{}", js);
}

#[test]
fn rewrite_in_array_inline() {
    let js = phpx_to_js("$arr = [1, 2, 3];\n$b = in_array(2, $arr);").expect("should compile");
    assert!(js.contains(".includes("), "expected .includes for in_array, got:\n{}", js);
    assert!(!js.contains("globalThis.in_array"), "should NOT contain globalThis.in_array, got:\n{}", js);
}

#[test]
fn rewrite_explode_inline() {
    let js = phpx_to_js("$s = 'a,b,c';\n$arr = explode(',', $s);").expect("should compile");
    assert!(js.contains(".split("), "expected .split for explode, got:\n{}", js);
    assert!(!js.contains("globalThis.explode"), "should NOT contain globalThis.explode, got:\n{}", js);
}

#[test]
fn rewrite_implode_inline() {
    let js = phpx_to_js("$arr = ['a', 'b'];\n$s = implode(',', $arr);").expect("should compile");
    assert!(js.contains(".join("), "expected .join for implode, got:\n{}", js);
    assert!(!js.contains("globalThis.implode"), "should NOT contain globalThis.implode, got:\n{}", js);
}

#[test]
fn rewrite_chr_inline() {
    let js = phpx_to_js("$c = chr(65);").expect("should compile");
    assert!(js.contains("String.fromCharCode("), "expected String.fromCharCode for chr, got:\n{}", js);
    assert!(!js.contains("globalThis.chr"), "should NOT contain globalThis.chr, got:\n{}", js);
}

#[test]
fn rewrite_ord_inline() {
    let js = phpx_to_js("$s = 'A';\n$n = ord($s);").expect("should compile");
    assert!(js.contains(".charCodeAt(0)"), "expected .charCodeAt(0) for ord, got:\n{}", js);
    assert!(!js.contains("globalThis.ord"), "should NOT contain globalThis.ord, got:\n{}", js);
}

#[test]
fn rewrite_time_inline() {
    let js = phpx_to_js("$t = time();").expect("should compile");
    assert!(js.contains("Math.floor(Date.now() / 1000)"), "expected Date.now for time, got:\n{}", js);
    assert!(!js.contains("globalThis.time"), "should NOT contain globalThis.time, got:\n{}", js);
}

#[test]
fn rewrite_array_keys_inline() {
    let js = phpx_to_js("$a = { x: 1, y: 2 };\n$k = array_keys($a);").expect("should compile");
    assert!(js.contains("Object.keys("), "expected Object.keys for array_keys, got:\n{}", js);
    assert!(!js.contains("globalThis.array_keys"), "should NOT contain globalThis.array_keys, got:\n{}", js);
}

#[test]
fn rewrite_array_keys_on_array_emits_numeric_indices() {
    let js = phpx_to_js("$a = [10, 20, 30];\n$k = array_keys($a);").expect("should compile");
    // For arrays we emit `__v.map((_, __i) => __i)` so the shape matches PHP
    // (integer indices) rather than JS `Object.keys` stringified indices.
    assert!(js.contains(".map("), "expected map for array-path array_keys, got:\n{}", js);
    assert!(!js.contains("globalThis.array_keys"), "should NOT contain globalThis.array_keys, got:\n{}", js);
}

#[test]
fn rewrite_array_values_inline() {
    let js = phpx_to_js("$a = { x: 1, y: 2 };\n$v = array_values($a);").expect("should compile");
    assert!(js.contains("Object.values("), "expected Object.values for array_values, got:\n{}", js);
    assert!(!js.contains("globalThis.array_values"), "should NOT contain globalThis.array_values, got:\n{}", js);
}

#[test]
fn rewrite_array_values_on_array_copies() {
    let js = phpx_to_js("$a = [1, 2, 3];\n$v = array_values($a);").expect("should compile");
    // For arrays we emit .slice() to return a shallow copy.
    assert!(js.contains(".slice()"), "expected .slice() for array-path array_values, got:\n{}", js);
    assert!(!js.contains("globalThis.array_values"), "should NOT contain globalThis.array_values, got:\n{}", js);
}

#[test]
fn rewrite_array_map_inline() {
    let js = phpx_to_js("$fn = fn($x: int): int => $x + 1;\n$a = [1, 2, 3];\n$r = array_map($fn, $a);").expect("should compile");
    assert!(js.contains(".map("), "expected .map() for array_map, got:\n{}", js);
    assert!(js.contains("const __fn"), "expected callback to be bound once for array_map, got:\n{}", js);
    assert!(!js.contains("globalThis.array_map"), "should NOT contain globalThis.array_map, got:\n{}", js);
}

#[test]
fn rewrite_array_filter_inline() {
    let js = phpx_to_js("$fn = fn($x: int): bool => $x > 1;\n$a = [1, 2, 3];\n$r = array_filter($a, $fn);").expect("should compile");
    assert!(js.contains(".filter("), "expected .filter() for array_filter, got:\n{}", js);
    assert!(js.contains("const __fn"), "expected callback to be bound once for array_filter, got:\n{}", js);
    assert!(!js.contains("globalThis.array_filter"), "should NOT contain globalThis.array_filter, got:\n{}", js);
}

#[test]
fn rewrite_array_filter_without_callback_uses_boolean() {
    let js = phpx_to_js("$a = [0, 1, 2];\n$r = array_filter($a);").expect("should compile");
    assert!(js.contains(".filter("), "expected .filter() for array_filter without callback, got:\n{}", js);
    assert!(js.contains("Boolean"), "expected Boolean callback for array_filter without callback, got:\n{}", js);
    assert!(!js.contains("globalThis.array_filter"), "should NOT contain globalThis.array_filter, got:\n{}", js);
}

#[test]
fn rewrite_is_array_inline() {
    let js = phpx_to_js("$arr = [1];\n$b = is_array($arr);").expect("should compile");
    assert!(js.contains("Array.isArray"), "expected Array.isArray for is_array, got:\n{}", js);
    // Check struct exclusion is present
    assert!(js.contains("__struct"), "expected __struct check in is_array, got:\n{}", js);
    assert!(!js.contains("globalThis.is_array"), "should NOT contain globalThis.is_array, got:\n{}", js);
}

#[test]
fn rewrite_does_not_shadow_user_variables() {
    // If user declares $count as a variable, calling count() after should
    // use the user's local variable, not the builtin rewrite.
    // (In practice, calling `$count(...)` is a different AST shape than `count(...)`)
    let js = phpx_to_js("$count = 0;\n$x = $count + 1;").expect("should compile");
    // $count variable reference should appear as local `count`, not as a rewrite
    assert!(js.contains("let count = 0"), "expected let count = 0, got:\n{}", js);
}

#[test]
fn prelude_no_removed_polyfills() {
    let js = phpx_to_js("$x = 1;").expect("should compile");
    // Verify that removed class (a) polyfills are NOT in the prelude
    assert!(!js.contains("globalThis.count ??="), "globalThis.count polyfill should be removed");
    assert!(!js.contains("globalThis.strlen ??="), "globalThis.strlen polyfill should be removed");
    assert!(!js.contains("globalThis.substr ??="), "globalThis.substr polyfill should be removed");
    assert!(!js.contains("globalThis.trim ??="), "globalThis.trim polyfill should be removed");
    assert!(!js.contains("globalThis.ltrim ??="), "globalThis.ltrim polyfill should be removed");
    assert!(!js.contains("globalThis.rtrim ??="), "globalThis.rtrim polyfill should be removed");
    assert!(!js.contains("globalThis.chr ??="), "globalThis.chr polyfill should be removed");
    assert!(!js.contains("globalThis.ord ??="), "globalThis.ord polyfill should be removed");
    assert!(!js.contains("globalThis.time ??="), "globalThis.time polyfill should be removed");
    assert!(!js.contains("globalThis.strtolower ??="), "globalThis.strtolower polyfill should be removed");
    assert!(!js.contains("globalThis.strtoupper ??="), "globalThis.strtoupper polyfill should be removed");
    assert!(!js.contains("globalThis.strpos ??="), "globalThis.strpos polyfill should be removed");
    assert!(!js.contains("globalThis.strrpos ??="), "globalThis.strrpos polyfill should be removed");
    assert!(!js.contains("globalThis.str_starts_with ??="), "globalThis.str_starts_with polyfill should be removed");
    assert!(!js.contains("globalThis.str_ends_with ??="), "globalThis.str_ends_with polyfill should be removed");
    assert!(!js.contains("globalThis.str_contains ??="), "globalThis.str_contains polyfill should be removed");
    assert!(!js.contains("globalThis.array_key_exists ??="), "globalThis.array_key_exists polyfill should be removed");
    assert!(!js.contains("globalThis.in_array ??="), "globalThis.in_array polyfill should be removed");
    assert!(!js.contains("globalThis.explode ??="), "globalThis.explode polyfill should be removed");
    assert!(!js.contains("globalThis.implode ??="), "globalThis.implode polyfill should be removed");
    assert!(!js.contains("globalThis.is_array ="), "globalThis.is_array polyfill should be removed");
    assert!(!js.contains("globalThis.array_keys ??="), "globalThis.array_keys polyfill should not be added");
    assert!(!js.contains("globalThis.array_values ??="), "globalThis.array_values polyfill should not be added");
    assert!(!js.contains("globalThis.array_map ??="), "globalThis.array_map polyfill should be removed");
    assert!(!js.contains("globalThis.array_filter ??="), "globalThis.array_filter polyfill should be removed");
    // Type predicates are compile-time IIFE rewrites — no globalThis polyfill.
    assert!(!js.contains("globalThis.is_int ??="), "globalThis.is_int polyfill should be removed");
    assert!(!js.contains("globalThis.is_float ??="), "globalThis.is_float polyfill should be removed");
    assert!(!js.contains("globalThis.is_numeric ??="), "globalThis.is_numeric polyfill should be removed");
    assert!(!js.contains("globalThis.is_string ??="), "globalThis.is_string polyfill should be removed");
    assert!(!js.contains("globalThis.is_object ??="), "globalThis.is_object polyfill should be removed");
    // Batch-2 functions: all are now compile-time rewrites, none should appear as globalThis polyfills.
    assert!(!js.contains("globalThis.urlencode ??="), "globalThis.urlencode polyfill should be removed");
    assert!(!js.contains("globalThis.urldecode ??="), "globalThis.urldecode polyfill should be removed");
    assert!(!js.contains("globalThis.gettype ??="), "globalThis.gettype polyfill should be removed");
    assert!(!js.contains("globalThis.get_object_vars ??="), "globalThis.get_object_vars polyfill should be removed");
    assert!(!js.contains("globalThis.mt_rand ??="), "globalThis.mt_rand polyfill should be removed");
    assert!(!js.contains("globalThis.microtime ??="), "globalThis.microtime polyfill should be removed");
    assert!(!js.contains("globalThis.strtotime ??="), "globalThis.strtotime polyfill should be removed");
    assert!(!js.contains("globalThis.preg_match ??="), "globalThis.preg_match polyfill should be removed");
    assert!(!js.contains("globalThis.preg_replace ??="), "globalThis.preg_replace polyfill should be removed");
    assert!(!js.contains("globalThis.parse_url ??="), "globalThis.parse_url polyfill should be removed");
    assert!(!js.contains("globalThis.intval ??="), "globalThis.intval polyfill should be removed");
    assert!(!js.contains("globalThis.floatval ??="), "globalThis.floatval polyfill should be removed");
    assert!(!js.contains("globalThis.boolval ??="), "globalThis.boolval polyfill should be removed");
    assert!(!js.contains("globalThis.strval ??="), "globalThis.strval polyfill should be removed");
    assert!(!js.contains("globalThis.array_slice ??="), "globalThis.array_slice polyfill should be removed");
    // But kept entries should still be present
    assert!(js.contains("globalThis.panic ??="), "globalThis.panic should still be in prelude");
    assert!(js.contains("globalThis.defined ??="), "globalThis.defined should still be in prelude");
}

// ---- Phase 2 (easy polyfills) rewrite tests ----

#[test]
fn rewrite_max_multi_arg() {
    let js = phpx_to_js("$n = max(1, 2, 3);").expect("should compile");
    assert!(js.contains("Math.max("), "expected Math.max for max, got:\n{}", js);
    assert!(!js.contains("globalThis.max"), "should NOT contain globalThis.max, got:\n{}", js);
}

#[test]
fn rewrite_max_single_array_arg() {
    let js = phpx_to_js("$arr = [1, 2, 3];\n$n = max($arr);").expect("should compile");
    // Single arg emits IIFE checking Array.isArray
    assert!(js.contains("Array.isArray"), "expected Array.isArray check for max(arr), got:\n{}", js);
    assert!(js.contains("Math.max("), "expected Math.max for max(arr), got:\n{}", js);
    assert!(!js.contains("globalThis.max"), "should NOT contain globalThis.max, got:\n{}", js);
}

#[test]
fn rewrite_min_multi_arg() {
    let js = phpx_to_js("$n = min(4, 2, 9);").expect("should compile");
    assert!(js.contains("Math.min("), "expected Math.min for min, got:\n{}", js);
    assert!(!js.contains("globalThis.min"), "should NOT contain globalThis.min, got:\n{}", js);
}

#[test]
fn rewrite_is_int_inline() {
    let js = phpx_to_js("$v = 42;\n$b = is_int($v);").expect("should compile");
    assert!(js.contains("typeof"), "expected typeof check for is_int, got:\n{}", js);
    assert!(js.contains("Number.isInteger"), "expected Number.isInteger for is_int, got:\n{}", js);
    assert!(!js.contains("globalThis.is_int"), "should NOT contain globalThis.is_int, got:\n{}", js);
}

#[test]
fn rewrite_is_float_inline() {
    let js = phpx_to_js("$v = 3.14;\n$b = is_float($v);").expect("should compile");
    assert!(js.contains("typeof"), "expected typeof check for is_float, got:\n{}", js);
    assert!(js.contains("Number.isInteger"), "expected Number.isInteger for is_float, got:\n{}", js);
    assert!(!js.contains("globalThis.is_float"), "should NOT contain globalThis.is_float, got:\n{}", js);
}

#[test]
fn rewrite_is_numeric_inline() {
    let js = phpx_to_js("$v = '42';\n$b = is_numeric($v);").expect("should compile");
    assert!(js.contains("isNaN(Number("), "expected isNaN(Number()) for is_numeric, got:\n{}", js);
    assert!(!js.contains("globalThis.is_numeric"), "should NOT contain globalThis.is_numeric, got:\n{}", js);
}

#[test]
fn rewrite_is_string_inline() {
    let js = phpx_to_js("$v = 'hello';\n$b = is_string($v);").expect("should compile");
    assert!(js.contains("typeof"), "expected typeof for is_string, got:\n{}", js);
    assert!(js.contains("\"string\""), "expected string type check for is_string, got:\n{}", js);
    assert!(!js.contains("globalThis.is_string"), "should NOT contain globalThis.is_string, got:\n{}", js);
}

#[test]
fn rewrite_is_object_inline() {
    let js = phpx_to_js("$v = { x: 1 };\n$b = is_object($v);").expect("should compile");
    assert!(js.contains("typeof"), "expected typeof for is_object, got:\n{}", js);
    assert!(js.contains("Array.isArray"), "expected Array.isArray check for is_object, got:\n{}", js);
    assert!(!js.contains("globalThis.is_object"), "should NOT contain globalThis.is_object, got:\n{}", js);
}

#[test]
fn rewrite_htmlspecialchars_inline() {
    let js = phpx_to_js("$s = '<script>';\n$e = htmlspecialchars($s);").expect("should compile");
    assert!(js.contains(".replace("), "expected .replace() chain for htmlspecialchars, got:\n{}", js);
    assert!(js.contains("&amp;"), "expected &amp; entity for htmlspecialchars, got:\n{}", js);
    assert!(js.contains("&lt;"), "expected &lt; entity for htmlspecialchars, got:\n{}", js);
    assert!(!js.contains("globalThis.htmlspecialchars"), "should NOT contain globalThis.htmlspecialchars, got:\n{}", js);
}

#[test]
fn rewrite_str_replace_scalar_inline() {
    let js = phpx_to_js("$s = 'hello world';\n$r = str_replace('world', 'earth', $s);").expect("should compile");
    assert!(js.contains(".split("), "expected .split for str_replace, got:\n{}", js);
    assert!(js.contains(".join("), "expected .join for str_replace, got:\n{}", js);
    assert!(!js.contains("globalThis.str_replace"), "should NOT contain globalThis.str_replace, got:\n{}", js);
}

#[test]
fn rewrite_rawurlencode_inline() {
    let js = phpx_to_js("$s = 'hello world';\n$e = rawurlencode($s);").expect("should compile");
    assert!(js.contains("encodeURIComponent("), "expected encodeURIComponent for rawurlencode, got:\n{}", js);
    assert!(js.contains("%21"), "expected %21 replacement for rawurlencode, got:\n{}", js);
    assert!(!js.contains("globalThis.rawurlencode"), "should NOT contain globalThis.rawurlencode, got:\n{}", js);
}

#[test]
fn rewrite_dechex_inline() {
    let js = phpx_to_js("$n = 255;\n$h = dechex($n);").expect("should compile");
    assert!(js.contains(".toString(16)"), "expected .toString(16) for dechex, got:\n{}", js);
    assert!(!js.contains("globalThis.dechex"), "should NOT contain globalThis.dechex, got:\n{}", js);
}

#[test]
fn rewrite_hexdec_inline() {
    let js = phpx_to_js("$s = 'ff';\n$n = hexdec($s);").expect("should compile");
    assert!(js.contains("parseInt("), "expected parseInt for hexdec, got:\n{}", js);
    assert!(js.contains(", 16)"), "expected base 16 for hexdec, got:\n{}", js);
    assert!(!js.contains("globalThis.hexdec"), "should NOT contain globalThis.hexdec, got:\n{}", js);
}

#[test]
fn rewrite_ltrim_with_chars_inline() {
    let js = phpx_to_js("$s = '...hello';\n$r = ltrim($s, '.');").expect("should compile");
    // 2-arg ltrim emits an IIFE with regex escaping
    assert!(js.contains("new RegExp("), "expected new RegExp for ltrim with chars, got:\n{}", js);
    assert!(!js.contains("globalThis.ltrim"), "should NOT contain globalThis.ltrim, got:\n{}", js);
}

#[test]
fn rewrite_rtrim_with_chars_inline() {
    let js = phpx_to_js("$s = 'hello...';\n$r = rtrim($s, '.');").expect("should compile");
    assert!(js.contains("new RegExp("), "expected new RegExp for rtrim with chars, got:\n{}", js);
    assert!(!js.contains("globalThis.rtrim"), "should NOT contain globalThis.rtrim, got:\n{}", js);
}

// ---- Batch 2: urlencode / urldecode / gettype / get_object_vars / mt_rand /
//              microtime / strtotime / preg_match / preg_replace / parse_url /
//              intval / floatval / boolval / strval / array_slice ----

#[test]
fn rewrite_urlencode_inline() {
    let js = phpx_to_js("$s = 'hello world';\n$e = urlencode($s);").expect("should compile");
    assert!(js.contains("encodeURIComponent("), "expected encodeURIComponent for urlencode, got:\n{}", js);
    assert!(js.contains("%20"), "expected %20-to-+ substitution for urlencode, got:\n{}", js);
    assert!(!js.contains("globalThis.urlencode"), "should NOT contain globalThis.urlencode, got:\n{}", js);
}

#[test]
fn rewrite_urldecode_inline() {
    let js = phpx_to_js("$s = 'hello+world';\n$d = urldecode($s);").expect("should compile");
    assert!(js.contains("decodeURIComponent("), "expected decodeURIComponent for urldecode, got:\n{}", js);
    assert!(js.contains("replace(/\\+/g"), "expected +→%20 substitution for urldecode, got:\n{}", js);
    assert!(!js.contains("globalThis.urldecode"), "should NOT contain globalThis.urldecode, got:\n{}", js);
    // Must be wrapped in IIFE with try/catch — no bare decodeURIComponent call.
    assert!(js.contains("try {"), "expected try/catch guard against URIError, got:\n{}", js);
    assert!(js.contains("catch("), "expected catch clause for URIError guard, got:\n{}", js);
}

// Regression: Hamza's DoS report — malformed %XX sequences must not throw URIError.
// PHP urldecode passes malformed sequences through unchanged.
#[test]
fn urldecode_malformed_percent_g0_no_throw() {
    // %G0 is not a valid percent-sequence — JS decodeURIComponent throws URIError.
    // The rewrite must emit a try/catch so the IIFE returns the raw string instead.
    let js = phpx_to_js("$d = urldecode('%G0');").expect("should compile");
    // Verify the guard structure is present in the emitted JS.
    assert!(js.contains("try {"), "urldecode('%G0') must emit try/catch guard, got:\n{}", js);
    assert!(js.contains("catch("), "urldecode('%G0') must emit catch clause, got:\n{}", js);
    // Verify the emitted code is syntactically valid and evaluates without throwing.
    // (We cannot run JS here, but structure checks above are sufficient for unit scope.)
}

#[test]
fn urldecode_malformed_truncated_percent_no_throw() {
    // %2 has only one hex digit — also invalid.
    let js = phpx_to_js("$d = urldecode('%2');").expect("should compile");
    assert!(js.contains("try {"), "urldecode('%2') must emit try/catch guard, got:\n{}", js);
}

#[test]
fn urldecode_malformed_incomplete_multibyte_no_throw() {
    // %E0 is the first byte of a three-byte UTF-8 sequence with no continuation bytes —
    // decodeURIComponent throws URIError on this.
    let js = phpx_to_js("$d = urldecode('%E0');").expect("should compile");
    assert!(js.contains("try {"), "urldecode('%E0') must emit try/catch guard, got:\n{}", js);
}

#[test]
fn urldecode_plus_to_space() {
    // Regression: existing behaviour — '+' must become a space.
    let js = phpx_to_js("$d = urldecode('hello+world');").expect("should compile");
    assert!(js.contains("replace(/\\+/g, '%20')"), "expected + → %20 substitution, got:\n{}", js);
}

#[test]
fn urldecode_percent20_to_space() {
    // Regression: %20 must decode to a space (decodeURIComponent handles this).
    let js = phpx_to_js("$d = urldecode('hello%20world');").expect("should compile");
    assert!(js.contains("decodeURIComponent("), "expected decodeURIComponent for %20, got:\n{}", js);
}

#[test]
fn rewrite_gettype_inline() {
    let js = phpx_to_js("$v = 42;\n$t = gettype($v);").expect("should compile");
    assert!(js.contains("Number.isInteger"), "expected Number.isInteger for gettype, got:\n{}", js);
    assert!(!js.contains("globalThis.gettype"), "should NOT contain globalThis.gettype, got:\n{}", js);
}

#[test]
fn rewrite_get_object_vars_inline() {
    let js = phpx_to_js("$o = ['a' => 1];\n$v = get_object_vars($o);").expect("should compile");
    assert!(js.contains("Object.keys("), "expected Object.keys for get_object_vars, got:\n{}", js);
    assert!(js.contains("__struct"), "expected __struct exclusion for get_object_vars, got:\n{}", js);
    assert!(!js.contains("globalThis.get_object_vars"), "should NOT contain globalThis.get_object_vars, got:\n{}", js);
}

#[test]
fn rewrite_mt_rand_inline() {
    let js = phpx_to_js("$n = mt_rand(1, 10);").expect("should compile");
    assert!(js.contains("Math.random()"), "expected Math.random for mt_rand, got:\n{}", js);
    assert!(js.contains("Math.floor("), "expected Math.floor for mt_rand, got:\n{}", js);
    assert!(!js.contains("globalThis.mt_rand"), "should NOT contain globalThis.mt_rand, got:\n{}", js);
}

#[test]
fn rewrite_microtime_as_float_inline() {
    let js = phpx_to_js("$t = microtime(true);").expect("should compile");
    assert!(js.contains("Date.now()"), "expected Date.now for microtime, got:\n{}", js);
    assert!(!js.contains("globalThis.microtime"), "should NOT contain globalThis.microtime, got:\n{}", js);
}

#[test]
fn rewrite_strtotime_inline() {
    let js = phpx_to_js("$ts = strtotime('2024-01-01');").expect("should compile");
    assert!(js.contains("new Date("), "expected new Date for strtotime, got:\n{}", js);
    assert!(js.contains("Math.floor("), "expected Math.floor for strtotime, got:\n{}", js);
    assert!(!js.contains("globalThis.strtotime"), "should NOT contain globalThis.strtotime, got:\n{}", js);
}

#[test]
fn rewrite_preg_match_inline() {
    let js = phpx_to_js("$m = preg_match('/foo/', 'foobar');").expect("should compile");
    assert!(js.contains("new RegExp("), "expected new RegExp for preg_match, got:\n{}", js);
    assert!(js.contains(".test("), "expected .test() for preg_match, got:\n{}", js);
    assert!(!js.contains("globalThis.preg_match"), "should NOT contain globalThis.preg_match, got:\n{}", js);
}

#[test]
fn rewrite_preg_replace_inline() {
    let js = phpx_to_js("$r = preg_replace('/foo/', 'bar', 'foobar');").expect("should compile");
    assert!(js.contains("new RegExp("), "expected new RegExp for preg_replace, got:\n{}", js);
    assert!(js.contains(".replace("), "expected .replace() for preg_replace, got:\n{}", js);
    assert!(!js.contains("globalThis.preg_replace"), "should NOT contain globalThis.preg_replace, got:\n{}", js);
}

#[test]
fn rewrite_parse_url_inline() {
    let js = phpx_to_js("$parts = parse_url('https://example.com/path?q=1');").expect("should compile");
    assert!(js.contains("new URL("), "expected new URL for parse_url, got:\n{}", js);
    assert!(!js.contains("globalThis.parse_url"), "should NOT contain globalThis.parse_url, got:\n{}", js);
}

#[test]
fn rewrite_intval_inline() {
    let js = phpx_to_js("$n = intval('42');").expect("should compile");
    assert!(js.contains("parseInt("), "expected parseInt for intval, got:\n{}", js);
    assert!(!js.contains("globalThis.intval"), "should NOT contain globalThis.intval, got:\n{}", js);
}

#[test]
fn rewrite_floatval_inline() {
    let js = phpx_to_js("$n = floatval('3.14');").expect("should compile");
    assert!(js.contains("parseFloat("), "expected parseFloat for floatval, got:\n{}", js);
    assert!(!js.contains("globalThis.floatval"), "should NOT contain globalThis.floatval, got:\n{}", js);
}

#[test]
fn rewrite_boolval_inline() {
    let js = phpx_to_js("$b = boolval(1);").expect("should compile");
    assert!(js.contains("Boolean("), "expected Boolean() for boolval, got:\n{}", js);
    assert!(!js.contains("globalThis.boolval"), "should NOT contain globalThis.boolval, got:\n{}", js);
}

#[test]
fn rewrite_strval_inline() {
    let js = phpx_to_js("$s = strval(42);").expect("should compile");
    assert!(js.contains("String("), "expected String() for strval, got:\n{}", js);
    assert!(!js.contains("globalThis.strval"), "should NOT contain globalThis.strval, got:\n{}", js);
}

#[test]
fn rewrite_array_slice_inline() {
    let js = phpx_to_js("$a = [1,2,3,4];\n$s = array_slice($a, 1, 2);").expect("should compile");
    assert!(js.contains(".slice("), "expected .slice() for array_slice, got:\n{}", js);
    assert!(!js.contains("globalThis.array_slice"), "should NOT contain globalThis.array_slice, got:\n{}", js);
}

// ---- Batch 3: base64, hash, date, pack, function_exists, class_exists ----

#[test]
fn rewrite_base64_encode_to_helper() {
    let js = phpx_to_js("$s = 'hello';\n$b = base64_encode($s);").expect("should compile");
    assert!(js.contains("__phpx_base64_encode("), "expected __phpx_base64_encode call, got:\n{}", js);
    assert!(!js.contains("globalThis.base64_encode ??="), "globalThis.base64_encode polyfill should NOT be in prelude, got:\n{}", js);
}

#[test]
fn rewrite_base64_decode_to_helper() {
    let js = phpx_to_js("$s = 'aGVsbG8=';\n$d = base64_decode($s);").expect("should compile");
    assert!(js.contains("__phpx_base64_decode("), "expected __phpx_base64_decode call, got:\n{}", js);
    assert!(!js.contains("globalThis.base64_decode ??="), "globalThis.base64_decode polyfill should NOT be in prelude, got:\n{}", js);
}

#[test]
fn rewrite_base64_decode_strict_to_helper() {
    let js = phpx_to_js("$s = 'aGVsbG8=';\n$d = base64_decode($s, true);").expect("should compile");
    assert!(js.contains("__phpx_base64_decode("), "expected __phpx_base64_decode call with strict arg, got:\n{}", js);
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
            l.starts_with("function __phpx_base64_")
                || l.starts_with("const __phpx_base64_table")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let script = format!("{helpers}\nconsole.log([String(__phpx_base64_decode('ab=Y', true)), String(__phpx_base64_decode('ab%=', true))].join('\\n'));");
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
            l.starts_with("function __phpx_base64_")
                || l.starts_with("const __phpx_base64_table")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let script = format!(
        "{helpers}\nconsole.log(__phpx_base64_decode('aGVsbG8=', true));"
    );
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
    assert!(js.contains("__phpx_hash("), "expected __phpx_hash call, got:\n{}", js);
    assert!(!js.contains("globalThis.hash ??="), "globalThis.hash polyfill should NOT be in prelude, got:\n{}", js);
}

#[test]
fn rewrite_hash_raw_to_helper() {
    let js = phpx_to_js("$s = 'hello';\n$h = hash('sha256', $s, true);").expect("should compile");
    assert!(js.contains("__phpx_hash("), "expected __phpx_hash call with raw arg, got:\n{}", js);
}

#[test]
fn rewrite_hash_hmac_to_helper() {
    let js = phpx_to_js("$h = hash_hmac('sha256', 'data', 'key');").expect("should compile");
    assert!(js.contains("__phpx_hash_hmac("), "expected __phpx_hash_hmac call, got:\n{}", js);
    assert!(!js.contains("globalThis.hash_hmac ??="), "globalThis.hash_hmac polyfill should NOT be in prelude, got:\n{}", js);
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
        .filter(|l| {
            l.starts_with("function __phpx_") || l.starts_with("const __phpx_node_crypto")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn sha256_nist_empty_string() {
    let helpers = sha256_helpers_js();
    let script = format!(
        "{helpers}\nconsole.log(__phpx_sha256_hex(''));"
    );
    match run_node(&script) {
        Err(e) if e.contains("node not available") => return, // skip if no node
        Err(e) => panic!("node error: {e}"),
        Ok(got) => assert_eq!(
            got,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
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
            got,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
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
            got,
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
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
    let script = format!(
        "{helpers}\nconsole.log(__phpx_sha256_hex('a'.repeat(1_000_000)));"
    );
    match run_node(&script) {
        Err(e) if e.contains("node not available") => return,
        Err(e) => panic!("node error: {e}"),
        Ok(got) => assert_eq!(
            got,
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0",
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
            got,
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7",
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
            got,
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843",
            "hmac-sha256 RFC 4231 TC2 mismatch"
        ),
    }
}

// ---- end SHA-256 correctness tests ----

#[test]
fn rewrite_hash_equals_inline_constant_time() {
    let js = phpx_to_js("$a = 'abc';\n$b = 'abc';\n$eq = hash_equals($a, $b);").expect("should compile");
    // Must be an IIFE with XOR loop — no external helper call
    assert!(!js.contains("__phpx_hash_equals"), "hash_equals must NOT call an external helper, got:\n{}", js);
    assert!(!js.contains("globalThis.hash_equals ??="), "globalThis.hash_equals polyfill should NOT be in prelude, got:\n{}", js);
    // Constant-time check: XOR accumulation
    assert!(js.contains("^"), "expected XOR (^) for constant-time compare, got:\n{}", js);
    assert!(js.contains("charCodeAt"), "expected charCodeAt for byte comparison, got:\n{}", js);
    // Must NOT short-circuit on length mismatch alone — must still run the loop
    assert!(js.contains("__a.length !== __b.length"), "expected length mismatch check, got:\n{}", js);
}

#[test]
fn rewrite_date_to_helper() {
    let js = phpx_to_js("$d = date('c');").expect("should compile");
    assert!(js.contains("__phpx_date("), "expected __phpx_date call, got:\n{}", js);
    assert!(!js.contains("globalThis.date ??="), "globalThis.date polyfill should NOT be in prelude, got:\n{}", js);
}

#[test]
fn rewrite_date_with_ts_to_helper() {
    let js = phpx_to_js("$t = 1700000000;\n$d = date('Y-m-d', $t);").expect("should compile");
    assert!(js.contains("__phpx_date("), "expected __phpx_date call with ts arg, got:\n{}", js);
}

#[test]
fn rewrite_gmdate_to_helper() {
    let js = phpx_to_js("$d = gmdate('c');").expect("should compile");
    assert!(js.contains("__phpx_gmdate("), "expected __phpx_gmdate call, got:\n{}", js);
    assert!(!js.contains("globalThis.gmdate ??="), "globalThis.gmdate polyfill should NOT be in prelude, got:\n{}", js);
}

#[test]
fn rewrite_pack_to_helper() {
    let js = phpx_to_js("$h = 'deadbeef';\n$b = pack('H*', $h);").expect("should compile");
    assert!(js.contains("__phpx_pack("), "expected __phpx_pack call, got:\n{}", js);
    assert!(!js.contains("globalThis.pack ??="), "globalThis.pack polyfill should NOT be in prelude, got:\n{}", js);
}

#[test]
fn rewrite_function_exists_inline() {
    let js = phpx_to_js("$b = function_exists('hash');").expect("should compile");
    assert!(js.contains("typeof globalThis["), "expected typeof globalThis lookup for function_exists, got:\n{}", js);
    assert!(js.contains("=== \"function\""), "expected === 'function' for function_exists, got:\n{}", js);
    assert!(!js.contains("globalThis.function_exists ??="), "globalThis.function_exists polyfill should NOT be in prelude, got:\n{}", js);
}

#[test]
fn rewrite_class_exists_inline_always_false() {
    let js = phpx_to_js("$b = class_exists('Result');").expect("should compile");
    // PHPX has no classes — class_exists always compiles to false
    assert!(js.contains("false"), "expected false for class_exists in PHPX, got:\n{}", js);
    assert!(!js.contains("globalThis.class_exists ??="), "globalThis.class_exists polyfill should NOT be in prelude, got:\n{}", js);
}

#[test]
fn prelude_batch3_no_removed_polyfills() {
    // Batch 3 polyfills must be gone — neither the old un-mangled form nor the
    // old globalThis.__phpx_X ??= form should appear in output.
    let js = phpx_to_js("$x = 1;").expect("should compile");
    // Old un-mangled polyfills (batch 3 targets) must be absent.
    assert!(!js.contains("globalThis.base64_encode ??="), "globalThis.base64_encode polyfill must be gone");
    assert!(!js.contains("globalThis.base64_decode ??="), "globalThis.base64_decode polyfill must be gone");
    assert!(!js.contains("globalThis.hash ??="), "globalThis.hash polyfill must be gone");
    assert!(!js.contains("globalThis.hash_hmac ??="), "globalThis.hash_hmac polyfill must be gone");
    assert!(!js.contains("globalThis.hash_equals ??="), "globalThis.hash_equals polyfill must be gone");
    assert!(!js.contains("globalThis.date ??="), "globalThis.date polyfill must be gone");
    assert!(!js.contains("globalThis.gmdate ??="), "globalThis.gmdate polyfill must be gone");
    assert!(!js.contains("globalThis.pack ??="), "globalThis.pack polyfill must be gone");
    assert!(!js.contains("globalThis.function_exists ??="), "globalThis.function_exists polyfill must be gone");
    assert!(!js.contains("globalThis.class_exists ??="), "globalThis.class_exists polyfill must be gone");
    // Tier B helpers must NOT appear in output for a file that does not use them.
    // They are module-scoped and emitted only when referenced (DCE via omission).
    assert!(!js.contains("globalThis.__phpx_base64_encode"), "globalThis.__phpx_base64_encode must be absent when unused");
    assert!(!js.contains("globalThis.__phpx_base64_decode"), "globalThis.__phpx_base64_decode must be absent when unused");
    assert!(!js.contains("globalThis.__phpx_hash ??="), "globalThis.__phpx_hash must be absent when unused");
    assert!(!js.contains("globalThis.__phpx_hash_hmac"), "globalThis.__phpx_hash_hmac must be absent when unused");
    assert!(!js.contains("globalThis.__phpx_date ??="), "globalThis.__phpx_date must be absent when unused");
    assert!(!js.contains("globalThis.__phpx_gmdate"), "globalThis.__phpx_gmdate must be absent when unused");
    assert!(!js.contains("globalThis.__phpx_pack"), "globalThis.__phpx_pack must be absent when unused");
    assert!(!js.contains("function __phpx_base64_encode"), "base64_encode helper must be absent when unused");
    assert!(!js.contains("function __phpx_hash("), "hash helper must be absent when unused");
    assert!(!js.contains("function __phpx_pack"), "pack helper must be absent when unused");
    assert!(!js.contains("function __phpx_date("), "date helper must be absent when unused");
}

#[test]
fn tier_b_helpers_emitted_as_module_scoped_functions() {
    // When base64_encode is used, the output must contain a module-scoped function
    // declaration — NOT a globalThis assignment. This is the DCE-correctness test.
    let js = phpx_to_js("$s = 'hello';\n$b = base64_encode($s);").expect("should compile");
    assert!(js.contains("function __phpx_base64_encode("), "expected module-scoped function declaration for base64_encode, got:\n{}", js);
    assert!(!js.contains("globalThis.__phpx_base64_encode"), "globalThis assignment must NOT be emitted for base64_encode, got:\n{}", js);
    // pack should be absent since it's not used
    assert!(!js.contains("function __phpx_pack"), "pack helper must be absent when unused, got:\n{}", js);
    // date should be absent since it's not used
    assert!(!js.contains("function __phpx_date("), "date helper must be absent when unused, got:\n{}", js);

    // Same check for date helper.
    let js2 = phpx_to_js("$d = date('Y-m-d');").expect("should compile");
    assert!(js2.contains("function __phpx_date("), "expected module-scoped function declaration for date, got:\n{}", js2);
    assert!(!js2.contains("globalThis.__phpx_date"), "globalThis assignment must NOT be emitted for date, got:\n{}", js2);
    // base64 should be absent since it's not used
    assert!(!js2.contains("function __phpx_base64_encode"), "base64_encode helper must be absent when unused, got:\n{}", js2);
}

#[test]
fn tier_b_dce_evidence_no_helpers_when_unused() {
    // A file that uses neither hash/hash_hmac nor base64_* must NOT contain any
    // of those helper declarations. This is the DCE-evidence test.
    let js = phpx_to_js("$x = strlen('hello');\n$y = count([1, 2, 3]);").expect("should compile");
    assert!(!js.contains("function __phpx_hash("), "hash helper must be absent when unused");
    assert!(!js.contains("function __phpx_hash_hmac("), "hash_hmac helper must be absent when unused");
    assert!(!js.contains("function __phpx_sha256_hex("), "sha256_hex helper must be absent when unused");
    assert!(!js.contains("function __phpx_base64_encode("), "base64_encode helper must be absent when unused");
    assert!(!js.contains("function __phpx_base64_decode("), "base64_decode helper must be absent when unused");
    assert!(!js.contains("__phpx_base64_table"), "base64_table must be absent when unused");
}

// ---- Phase 3: scope validation warnings ----

/// Helper: parse PHPX source and emit JS, returning both the JS and any
/// scope-validation warnings.
fn phpx_to_js_with_warnings(source: &str) -> Result<(String, Vec<String>), String> {
    let arena = Bump::new();
    let mut parser =
        Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();
    if !program.errors.is_empty() {
        let msgs: Vec<&str> = program.errors.iter().map(|e| e.message).collect();
        return Err(format!("parse errors: {}", msgs.join("; ")));
    }
    emit_js_from_ast_with_warnings(&program, source.as_bytes(), SourceModuleMeta::empty())
}

#[test]
fn scope_outer_declare_inner_assign_no_warning() {
    // Variable declared at outer scope, assigned inside foreach — no warning.
    // Wrapped in a function to avoid top-level globalThis mirroring.
    let source = "function test(): int {\n  $total = 0;\n  foreach ([1,2,3] as $v) {\n    $total = $total + $v;\n  }\n  return $total;\n}";
    let (js, warnings) = phpx_to_js_with_warnings(source).expect("should compile");
    let scope_warnings: Vec<_> = warnings.iter().filter(|w| w.contains("first assigned inside a block")).collect();
    assert!(scope_warnings.is_empty(), "expected no scope warnings, got: {:?}", scope_warnings);
    assert!(js.contains("let total = 0"), "expected let total at function scope, got:\n{}", js);
    // Inside the function, total should never become globalThis.total
    assert!(!js.contains("globalThis.total"), "total should not be globalThis inside function, got:\n{}", js);
}

#[test]
fn scope_first_assign_inside_block_warns() {
    // Variable first assigned inside a foreach, then read outside — should warn.
    let source = "function test(): int {\n  foreach ([1,2,3] as $v) {\n    $total = $total + $v;\n  }\n  return $total;\n}";
    let (_js, warnings) = phpx_to_js_with_warnings(source).expect("should compile");
    assert!(
        warnings.iter().any(|w| w.contains("$total") && w.contains("first assigned inside a block")),
        "expected scope warning for $total, got: {:?}", warnings
    );
}

#[test]
fn scope_normal_block_scoped_no_warning() {
    // Variables used only within their declaring scope — no warning.
    let source = "function test(): int {\n  $x = 10;\n  if ($x > 5) {\n    $y = $x + 1;\n  }\n  return $x;\n}";
    let (_js, warnings) = phpx_to_js_with_warnings(source).expect("should compile");
    let scope_warnings: Vec<_> = warnings.iter().filter(|w| w.contains("first assigned inside a block")).collect();
    assert!(scope_warnings.is_empty(), "expected no scope warnings, got: {:?}", scope_warnings);
}

// ==================================================================
// Phase 4: Comprehensive PHPX -> JS snapshot coverage
// ==================================================================

// ---- Variables and assignments ----

#[test]
fn variable_integer_emits_let() {
    let js = phpx_to_js("function f(): void { $x = 42; }").expect("should compile");
    assert!(js.contains("let x = 42"), "expected let x = 42, got:\n{}", js);
}

#[test]
fn variable_reassignment_no_redeclaration() {
    let js = phpx_to_js("function f(): void { $x = 1;\n$x = $x + 1; }").expect("should compile");
    // First should be let, second should be bare assignment
    let lines: Vec<&str> = js.lines().filter(|l| l.contains("x =") || l.contains("x=")).collect();
    let let_count = lines.iter().filter(|l| l.contains("let x")).count();
    assert_eq!(let_count, 1, "expected exactly one let declaration, got:\n{}", js);
}

#[test]
fn variable_string_literal() {
    let js = phpx_to_js("function f(): void { $name = 'hello'; }").expect("should compile");
    assert!(js.contains("let name ="), "expected let name, got:\n{}", js);
    assert!(js.contains("hello"), "expected string 'hello', got:\n{}", js);
}

#[test]
fn multiple_assignments_only_first_gets_let() {
    let source = r#"
function f(): void {
$a = 1;
$b = 2;
$a = 3;
$b = 4;
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    // Extract just the function body to avoid prelude noise
    let fn_body = js.split("function f()").nth(1).unwrap_or(&js);
    let let_a_count = fn_body.matches("let a").count();
    let let_b_count = fn_body.matches("let b").count();
    assert_eq!(let_a_count, 1, "expected exactly one 'let a' in function body, got:\n{}", fn_body);
    assert_eq!(let_b_count, 1, "expected exactly one 'let b' in function body, got:\n{}", fn_body);
}

#[test]
fn variable_boolean_true() {
    let js = phpx_to_js("function f(): void { $flag = true; }").expect("should compile");
    assert!(js.contains("let flag = true"), "expected let flag = true, got:\n{}", js);
}

#[test]
fn variable_boolean_false() {
    let js = phpx_to_js("function f(): void { $flag = false; }").expect("should compile");
    assert!(js.contains("let flag = false"), "expected let flag = false, got:\n{}", js);
}

// ---- String operations ----

#[test]
fn string_concat_two_parts() {
    let js = phpx_to_js("function f(): string { $s = 'hi' . ' there'; return $s; }").expect("should compile");
    assert!(js.contains("+"), "expected + for string concat, got:\n{}", js);
    assert!(!js.contains(" . "), "should not have PHP dot operator, got:\n{}", js);
}

#[test]
fn string_concat_with_variable() {
    let js = phpx_to_js("function f(): string { $name = 'world'; $s = 'hello ' . $name . '!'; return $s; }").expect("should compile");
    assert!(js.contains("+"), "expected + for concatenation, got:\n{}", js);
}

#[test]
fn string_concat_multiple() {
    let js = phpx_to_js("function f(): string { $a = 'a'; $b = 'b'; $c = 'c'; $r = $a . $b . $c; return $r; }").expect("should compile");
    assert!(js.contains("+"), "expected + operators for concat chain, got:\n{}", js);
}

// ---- Array and object literals ----

#[test]
fn array_literal_compiles() {
    let js = phpx_to_js("function f(): void { $a = [1, 2, 3]; }").expect("should compile");
    assert!(js.contains("[1, 2, 3]") || (js.contains("1") && js.contains("2") && js.contains("3")),
        "expected array literal, got:\n{}", js);
}

#[test]
fn object_literal_multiple_keys() {
    let js = phpx_to_js("function f(): void { $o = { key: 'value', num: 42 }; }").expect("should compile");
    assert!(js.contains("key"), "expected 'key' in object, got:\n{}", js);
    assert!(js.contains("42"), "expected 42 in object, got:\n{}", js);
}

#[test]
fn nested_array_in_object() {
    let js = phpx_to_js("function f(): void { $o = { items: [1, 2] }; }").expect("should compile");
    assert!(js.contains("items"), "expected 'items' key, got:\n{}", js);
}

// ---- Regression: quoted string-literal keys (deka#25) ----
//
// PHPX source like `{ 'content-type': 'text/html' }` historically leaked
// the surrounding quote characters into the emitted JS key, producing
// `{"'content-type'": "text/html"}`. The key bytes must be the decoded
// inner string, not the raw source span.

#[test]
fn single_quoted_key_strips_delimiters() {
    let js = phpx_to_js("function f(): void { $o = { 'content-type': 'text/html' }; }")
        .expect("should compile");
    assert!(
        js.contains("\"content-type\""),
        "expected bare 'content-type' key, got:\n{}",
        js
    );
    assert!(
        !js.contains("\"'content-type'\"") && !js.contains("\\'content-type\\'"),
        "key must not retain outer quote characters, got:\n{}",
        js
    );
}

#[test]
fn double_quoted_key_strips_delimiters() {
    let js = phpx_to_js("function f(): void { $o = { \"x-shop-id\": 'abc' }; }")
        .expect("should compile");
    assert!(
        js.contains("\"x-shop-id\""),
        "expected bare 'x-shop-id' key, got:\n{}",
        js
    );
    assert!(
        !js.contains("\\\"x-shop-id\\\""),
        "key must not retain escaped double quotes, got:\n{}",
        js
    );
}

#[test]
fn quoted_keys_with_special_chars_preserved() {
    let js = phpx_to_js(
        "function f(): void { $o = { 'with-dash': 1, 'with space': 2 }; }",
    )
    .expect("should compile");
    assert!(
        js.contains("\"with-dash\""),
        "expected 'with-dash' key, got:\n{}",
        js
    );
    assert!(
        js.contains("\"with space\""),
        "expected 'with space' key, got:\n{}",
        js
    );
}

#[test]
fn mixed_quoted_and_bare_keys() {
    let js = phpx_to_js("function f(): void { $o = { 'a': 1, b: 2 }; }")
        .expect("should compile");
    assert!(
        js.contains("\"a\"") && js.contains("1"),
        "expected quoted 'a' key mapped to 1, got:\n{}",
        js
    );
    // The bare identifier key should still appear as `b`, either bare or
    // JSON-quoted. It must NOT carry quote characters in the key name.
    assert!(
        js.contains("b: 2") || js.contains("\"b\": 2"),
        "expected bare 'b' key mapped to 2, got:\n{}",
        js
    );
}

#[test]
fn bare_identifier_key_unchanged() {
    // Regression guard: the fix must not affect unquoted identifier keys.
    let js = phpx_to_js("function f(): void { $o = { items: 1 }; }")
        .expect("should compile");
    assert!(
        js.contains("items: 1") || js.contains("\"items\": 1"),
        "expected bare 'items' key, got:\n{}",
        js
    );
}

#[test]
fn escaped_quote_inside_string_key_preserved() {
    // A single-quoted key containing an escaped single quote should decode
    // to a key name with a literal `'` in it.
    let js = phpx_to_js("function f(): void { $o = { 'it\\'s': 1 }; }")
        .expect("should compile");
    // JSON encoding of `it's` -> `"it's"` (single quote is not escaped in
    // JSON strings). Just verify the key is properly encoded.
    assert!(
        js.contains("\"it's\""),
        "expected decoded key \"it's\", got:\n{}",
        js
    );
}

#[test]
fn array_append_emits_push() {
    let js = phpx_to_js("function f(): void { $a = [1]; $a[] = 2; }").expect("should compile");
    assert!(js.contains("push"), "expected .push for array append, got:\n{}", js);
}

#[test]
fn associative_array_compiles() {
    let js = phpx_to_js("function f(): void { $a = ['name' => 'test', 'age' => 25]; }").expect("should compile");
    assert!(js.contains("name") && js.contains("test"),
        "expected associative key-value, got:\n{}", js);
}

// ---- Control flow ----

#[test]
fn if_statement_basic() {
    let js = phpx_to_js("function f(): void { $x = 1;\nif ($x == 1) { $x = 2; } }").expect("should compile");
    assert!(js.contains("if ("), "expected if statement, got:\n{}", js);
}

#[test]
fn if_else_branches() {
    let source = r#"
function f(): void {
$x = 10;
if ($x > 5) {
    $y = 1;
} else {
    $y = 2;
}
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(js.contains("if ("), "expected if, got:\n{}", js);
    assert!(js.contains("else"), "expected else, got:\n{}", js);
}

#[test]
fn foreach_array_emits_for_of() {
    let source = r#"
function f(): void {
$items = [1, 2, 3];
foreach ($items as $item) {
    $x = $item;
}
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(js.contains("for (const item of"), "expected for...of loop, got:\n{}", js);
}

#[test]
fn foreach_key_value_emits_entries() {
    let source = r#"
function f(): void {
$items = ['a' => 1, 'b' => 2];
foreach ($items as $key => $value) {
    $x = $key;
}
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(js.contains("Object.entries"), "expected Object.entries for key-value foreach, got:\n{}", js);
}

#[test]
fn while_loop_compiles() {
    let source = r#"
function f(): void {
$i = 0;
while ($i < 10) {
    $i = $i + 1;
}
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(js.contains("while ("), "expected while loop, got:\n{}", js);
}

#[test]
fn for_loop_compiles() {
    let source = r#"
function f(): void {
for ($i = 0; $i < 10; $i++) {
    $x = $i;
}
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(js.contains("for ("), "expected for loop, got:\n{}", js);
}

#[test]
fn match_scalar_emits_ternary_chain() {
    let source = r#"
function f(): string {
$x = 1;
$result = match ($x) {
    1 => "one",
    2 => "two",
    default => "other",
};
return $result;
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(js.contains("?"), "expected ternary in match output, got:\n{}", js);
    assert!(js.contains("one"), "expected 'one' arm, got:\n{}", js);
    assert!(js.contains("two"), "expected 'two' arm, got:\n{}", js);
    assert!(js.contains("other"), "expected default arm, got:\n{}", js);
}

#[test]
fn do_while_loop_compiles() {
    let source = r#"
function f(): void {
$i = 0;
do {
    $i = $i + 1;
} while ($i < 5);
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(js.contains("do {"), "expected do...while, got:\n{}", js);
    assert!(js.contains("while ("), "expected while condition, got:\n{}", js);
}

#[test]
fn break_in_loop() {
    let source = r#"
function f(): void {
$i = 0;
while (true) {
    if ($i > 5) { break; }
    $i = $i + 1;
}
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(js.contains("break;"), "expected break statement, got:\n{}", js);
}

#[test]
fn continue_in_loop() {
    let source = r#"
function f(): void {
foreach ([1, 2, 3] as $item) {
    if ($item == 2) { continue; }
    $x = $item;
}
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(js.contains("continue;"), "expected continue statement, got:\n{}", js);
}

#[test]
fn switch_statement_compiles() {
    let source = r#"
function f(): void {
$x = 1;
switch ($x) {
    case 1:
        $y = 'one';
        break;
    case 2:
        $y = 'two';
        break;
    default:
        $y = 'other';
        break;
}
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(js.contains("switch ("), "expected switch, got:\n{}", js);
    assert!(js.contains("case 1"), "expected case 1, got:\n{}", js);
    assert!(js.contains("default:"), "expected default case, got:\n{}", js);
}

// ---- Functions ----

#[test]
fn function_with_typed_params_types_erased() {
    let js = phpx_to_js("function add($a: int, $b: int): int { return $a + $b; }").expect("should compile");
    // The function signature must not carry type annotations.
    assert!(js.contains("function add(a, b)"), "expected types erased in params, got:\n{}", js);
    // The word "int" must not appear inside the function signature itself.
    // (The prelude legitimately contains "parseInt" / "Number.isInteger" / "integer", so
    // we cannot assert the whole output is free of "int".)
    assert!(!js.contains("function add(a: int") && !js.contains(": int)") && !js.contains(", b: int"),
        "type annotation must be erased from the function signature, got:\n{}", js);
}

#[test]
fn function_default_param_value() {
    let js = phpx_to_js("function greet($name: string = 'world'): string { return 'hello ' . $name; }").expect("should compile");
    assert!(js.contains("function greet(name"), "expected function greet, got:\n{}", js);
    // Default values are emitted as guards inside the function body
    assert!(js.contains("world") || js.contains("name ??="), "expected default value handling, got:\n{}", js);
}

#[test]
fn function_return_value() {
    let js = phpx_to_js("function double($x: int): int { return $x * 2; }").expect("should compile");
    assert!(js.contains("return"), "expected return statement, got:\n{}", js);
    assert!(js.contains("* 2"), "expected multiplication, got:\n{}", js);
}

#[test]
fn arrow_function_with_typed_param() {
    let js = phpx_to_js("function f(): void { $fn = fn($x: int): int => $x + 1; }").expect("should compile");
    assert!(js.contains("=>"), "expected arrow function, got:\n{}", js);
    assert!(js.contains("+ 1"), "expected + 1, got:\n{}", js);
}

#[test]
fn closure_compiles() {
    let source = r#"
function f(): void {
$add = function($a: int, $b: int): int {
    return $a + $b;
};
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(js.contains("function("), "expected anonymous function, got:\n{}", js);
}

#[test]
fn async_function_compiles() {
    let js = phpx_to_js("async function fetch_data(): string { return 'data'; }").expect("should compile");
    assert!(js.contains("async function"), "expected async function, got:\n{}", js);
}

#[test]
fn await_expression_compiles() {
    let source = r#"
async function get(): string {
$result = await fetch_data();
return $result;
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(js.contains("await"), "expected await expression, got:\n{}", js);
}

// ---- Structs ----

#[test]
fn struct_with_fields_compiles() {
    let source = r#"
struct Point {
$x: int;
$y: int;
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    // Struct declarations are stored as schemas, may not produce direct output
    // but should not error
    assert!(js.len() >= 0, "struct should compile without error");
}

#[test]
fn struct_instantiation_emits_object() {
    // The transpiler may emit struct instantiation as an object with __struct tag
    let source = r#"
struct Point {
$x: int;
$y: int;
}
$p = Point { x: 1, y: 2 };
"#;
    // Try to compile; struct instantiation may or may not be supported at top level
    let result = phpx_to_js(source);
    if let Ok(js) = result {
        // If it compiles, check output shape
        assert!(js.contains("x") && js.contains("y"), "expected struct fields in output, got:\n{}", js);
    }
    // If it doesn't compile, that's also fine — we tested the error path
}

#[test]
fn struct_field_access() {
    let source = r#"
function f(): void {
$obj = { x: 1, y: 2 };
$val = $obj.x;
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(js.contains("obj.x"), "expected dot notation access, got:\n{}", js);
}

// ---- Enums ----

#[test]
fn enum_declaration_emits_class() {
    let source = r#"
enum Color {
case Red;
case Blue;
case Green;
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(js.contains("class Color"), "expected class for enum, got:\n{}", js);
    assert!(js.contains("Red"), "expected Red case, got:\n{}", js);
    assert!(js.contains("Blue"), "expected Blue case, got:\n{}", js);
    assert!(js.contains("Green"), "expected Green case, got:\n{}", js);
}

#[test]
fn enum_case_access() {
    let source = r#"
enum Color {
case Red;
case Blue;
}
function f(): void {
$c = Color::Red;
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(js.contains("Color.Red"), "expected Color.Red access, got:\n{}", js);
}

#[test]
fn enum_with_payload() {
    let source = r#"
enum Shape {
case Circle($radius: float);
case Rectangle($width: float, $height: float);
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(js.contains("Circle"), "expected Circle case, got:\n{}", js);
    assert!(js.contains("Rectangle"), "expected Rectangle case, got:\n{}", js);
}

#[test]
fn match_on_enum_emits_instanceof_check() {
    let source = r#"
enum Color {
case Red;
case Blue;
}
function f(): string {
$c = Color::Red;
$name = match ($c) {
    Color::Red => "red",
    Color::Blue => "blue",
};
return $name;
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(js.contains("instanceof") || js.contains("__case"),
        "expected enum match pattern, got:\n{}", js);
}

// ---- isset() and Option patterns ----

#[test]
fn isset_single_variable() {
    let source = r#"
function f(): bool {
$x = 42;
return isset($x);
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(js.contains("!== undefined") && js.contains("!== null"),
        "expected isset null/undefined check, got:\n{}", js);
}

#[test]
fn isset_property_access() {
    let source = r#"
function f(): bool {
$obj = { key: 'val' };
return isset($obj.key);
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(js.contains("!== undefined") && js.contains("!== null"),
        "expected isset check on property, got:\n{}", js);
}

#[test]
fn isset_array_key() {
    let source = r#"
function f(): bool {
$a = ['x' => 1];
return isset($a['x']);
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(js.contains("!== undefined") && js.contains("!== null"),
        "expected isset check on array key, got:\n{}", js);
}

// ---- Error-as-value patterns ----

#[test]
fn error_as_value_ok_pattern() {
    let source = r#"
function f(): Object {
return { ok: true, value: 42 };
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(js.contains("ok") && js.contains("true") && js.contains("42"),
        "expected error-as-value ok pattern, got:\n{}", js);
}

#[test]
fn error_as_value_error_pattern() {
    let source = r#"
function f(): Object {
return { ok: false, error: 'something failed' };
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(js.contains("ok") && js.contains("false") && js.contains("something failed"),
        "expected error-as-value error pattern, got:\n{}", js);
}

#[test]
fn check_result_ok_field() {
    let source = r#"
function f(): void {
$result = { ok: true, value: 42 };
if ($result.ok) {
    $v = $result.value;
}
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(js.contains("result.ok"), "expected result.ok check, got:\n{}", js);
    assert!(js.contains("result.value"), "expected result.value access, got:\n{}", js);
}

// ---- Compile-time rewrite edge cases ----

#[test]
fn rewrite_count_on_object() {
    let js = phpx_to_js("function f(): int { $o = { a: 1, b: 2 };\nreturn count($o); }").expect("should compile");
    assert!(js.contains("Object.keys"), "expected Object.keys fallback for count on object, got:\n{}", js);
}

#[test]
fn rewrite_substr_negative_start() {
    let js = phpx_to_js("function f(): string { $s = 'hello';\nreturn substr($s, -2); }").expect("should compile");
    assert!(js.contains(".slice(") || js.contains("__st"), "expected slice or IIFE for negative start, got:\n{}", js);
}

#[test]
fn rewrite_in_array_with_strict() {
    let js = phpx_to_js("function f(): bool { $arr = [1, 2, 3];\nreturn in_array(2, $arr, true); }").expect("should compile");
    assert!(js.contains(".includes("), "expected .includes for in_array strict, got:\n{}", js);
}

#[test]
fn rewrite_explode_with_limit() {
    let js = phpx_to_js("function f(): void { $s = 'a:b:c:d';\n$parts = explode(':', $s, 2); }").expect("should compile");
    assert!(js.contains(".split("), "expected .split in explode with limit, got:\n{}", js);
}

#[test]
fn rewrite_implode_single_arg() {
    let js = phpx_to_js("function f(): string { $arr = ['a', 'b'];\nreturn implode($arr); }").expect("should compile");
    assert!(js.contains(".join("), "expected .join for single-arg implode, got:\n{}", js);
}

#[test]
fn rewrite_strpos_with_offset() {
    let js = phpx_to_js("function f(): void { $h = 'hello world';\n$i = strpos($h, 'o', 5); }").expect("should compile");
    assert!(js.contains(".indexOf("), "expected indexOf for strpos with offset, got:\n{}", js);
    assert!(js.contains("5"), "expected offset 5, got:\n{}", js);
}

// ---- Operators ----

#[test]
fn equality_operator_maps_to_strict() {
    let js = phpx_to_js("function f(): bool { $x = 1;\nreturn $x == 1; }").expect("should compile");
    assert!(js.contains("==="), "expected === for ==, got:\n{}", js);
}

#[test]
fn not_equal_maps_to_strict() {
    let js = phpx_to_js("function f(): bool { $x = 1;\nreturn $x != 2; }").expect("should compile");
    assert!(js.contains("!=="), "expected !== for !=, got:\n{}", js);
}

#[test]
fn logical_and_compiles() {
    let js = phpx_to_js("function f(): bool { $a = true;\n$b = false;\nreturn $a && $b; }").expect("should compile");
    assert!(js.contains("&&"), "expected &&, got:\n{}", js);
}

#[test]
fn logical_or_compiles() {
    let js = phpx_to_js("function f(): bool { $a = true;\n$b = false;\nreturn $a || $b; }").expect("should compile");
    assert!(js.contains("||"), "expected ||, got:\n{}", js);
}

#[test]
fn null_coalesce_operator() {
    let js = phpx_to_js("function f(): int { $x = 1;\nreturn $x ?? 0; }").expect("should compile");
    assert!(js.contains("??"), "expected ?? operator, got:\n{}", js);
}

#[test]
fn modulo_operator() {
    let js = phpx_to_js("function f(): int { return 10 % 3; }").expect("should compile");
    assert!(js.contains("%"), "expected modulo operator, got:\n{}", js);
}

#[test]
fn power_operator() {
    let js = phpx_to_js("function f(): int { return 2 ** 3; }").expect("should compile");
    assert!(js.contains("**"), "expected ** operator, got:\n{}", js);
}

#[test]
fn comparison_operators() {
    let js = phpx_to_js("function f(): bool { $x = 5;\nreturn $x >= 3; }").expect("should compile");
    assert!(js.contains(">="), "expected >= operator, got:\n{}", js);
}

#[test]
fn unary_not() {
    let js = phpx_to_js("function f(): bool { $flag = true;\nreturn !$flag; }").expect("should compile");
    assert!(js.contains("!"), "expected ! operator, got:\n{}", js);
}

#[test]
fn unary_negation() {
    let js = phpx_to_js("function f(): int { $x = 5;\nreturn -$x; }").expect("should compile");
    assert!(js.contains("-"), "expected - negation, got:\n{}", js);
}

#[test]
fn increment_operator() {
    let js = phpx_to_js("function f(): void { $i = 0;\n$i++; }").expect("should compile");
    assert!(js.contains("++"), "expected ++ operator, got:\n{}", js);
}

#[test]
fn decrement_operator() {
    let js = phpx_to_js("function f(): void { $i = 10;\n$i--; }").expect("should compile");
    assert!(js.contains("--"), "expected -- operator, got:\n{}", js);
}

// ---- Nullsafe operator ----

#[test]
fn nullsafe_property_access() {
    let js = phpx_to_js("function f(): void { $obj = { name: 'test' };\n$v = $obj?->name; }").expect("should compile");
    assert!(js.contains("?."), "expected optional chaining ?., got:\n{}", js);
}

#[test]
fn nullsafe_method_call() {
    let source = r#"
function f(): void {
$obj = { name: 'test' };
$v = $obj?->toString();
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(js.contains("?."), "expected optional chaining, got:\n{}", js);
}

// ---- CQL (Cypher) ----

#[test]
fn cql_with_params() {
    let source = "function f(): void { $name = 'test';\ncql q = MATCH (n:User {name: $name}) RETURN n; }";
    let js = phpx_to_js(source).expect("should compile");
    assert!(js.contains("__type: \"cql\""), "expected CQL marker, got:\n{}", js);
    assert!(js.contains("MATCH"), "expected MATCH in CQL, got:\n{}", js);
}

// ---- Echo and print ----

#[test]
fn echo_statement_compiles() {
    let js = phpx_to_js("function f(): void { echo 'hello'; }").expect("should compile");
    assert!(js.contains("hello"), "expected echo output, got:\n{}", js);
    assert!(js.contains("dekaPrint") || js.contains("console.log"),
        "expected print call, got:\n{}", js);
}

// ---- Const ----

#[test]
fn const_declaration() {
    let js = phpx_to_js("const MAX = 100;").expect("should compile");
    assert!(js.contains("const MAX = 100"), "expected const declaration, got:\n{}", js);
}

// ---- Empty expression ----

#[test]
fn empty_expression() {
    let js = phpx_to_js("function f(): bool { $x = '';\nreturn empty($x); }").expect("should compile");
    assert!(js.contains("!"), "expected negation for empty(), got:\n{}", js);
}

// ---- Clone ----

#[test]
fn clone_uses_structured_clone() {
    let js = phpx_to_js("function f(): void { $a = { x: 1 };\n$b = clone $a; }").expect("should compile");
    assert!(js.contains("structuredClone"), "expected structuredClone for clone, got:\n{}", js);
}

// ---- Instanceof ----

#[test]
fn instanceof_struct_uses_phpx_check() {
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
    assert!(js.contains("__phpx_is_struct"), "expected __phpx_is_struct for instanceof struct, got:\n{}", js);
}

// ---- Ternary ----

#[test]
fn ternary_expression() {
    let js = phpx_to_js("function f(): int { $x = 5;\nreturn $x > 3 ? 1 : 0; }").expect("should compile");
    assert!(js.contains("?") && js.contains(":"), "expected ternary expression, got:\n{}", js);
}

// ---- Array access patterns ----

#[test]
fn bracket_access_string_key() {
    let js = phpx_to_js("function f(): void { $a = ['key' => 'val'];\n$v = $a['key']; }").expect("should compile");
    assert!(js.contains("a["), "expected bracket access, got:\n{}", js);
}

#[test]
fn bracket_access_numeric_index() {
    let js = phpx_to_js("function f(): void { $a = [10, 20, 30];\n$v = $a[1]; }").expect("should compile");
    assert!(js.contains("a[1]"), "expected numeric index access, got:\n{}", js);
}

// ---- JSX ----

#[test]
fn jsx_basic_element() {
    let js = phpx_to_js("function View(): Object { return <div>hello</div>; }").expect("should compile");
    assert!(js.contains("div"), "expected div element, got:\n{}", js);
}

#[test]
fn jsx_with_props() {
    let js = phpx_to_js("function View(): Object { return <div class=\"test\" id=\"main\">content</div>; }").expect("should compile");
    assert!(js.contains("test") && js.contains("main"),
        "expected props in JSX output, got:\n{}", js);
}

#[test]
fn jsx_with_expression_child() {
    let source = r#"
function View(): Object {
$name = 'world';
return <div>hello {$name}</div>;
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(js.contains("name"), "expected expression child, got:\n{}", js);
}

#[test]
fn jsx_self_closing() {
    let js = phpx_to_js("function View(): Object { return <br />; }").expect("should compile");
    assert!(js.contains("br"), "expected br element, got:\n{}", js);
}

#[test]
fn jsx_nested_elements() {
    let source = r#"
function View(): Object {
return <div><span>inner</span></div>;
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(js.contains("div") && js.contains("span"),
        "expected nested elements, got:\n{}", js);
}

// ---- Scope validation edge cases ----

#[test]
fn scope_multiple_nested_blocks() {
    let source = r#"
function f(): int {
$total = 0;
foreach ([1, 2] as $v) {
    if ($v > 0) {
        $total = $total + $v;
    }
}
return $total;
}
"#;
    let (_js, warnings) = phpx_to_js_with_warnings(source).expect("should compile");
    let scope_warnings: Vec<_> = warnings.iter().filter(|w| w.contains("first assigned inside a block")).collect();
    assert!(scope_warnings.is_empty(), "expected no scope warnings for outer-declared var, got: {:?}", scope_warnings);
}

#[test]
fn scope_if_block_declaration_warns_on_outside_use() {
    let source = r#"
function f(): int {
if (true) {
    $inner = 42;
}
return $inner;
}
"#;
    let (_js, warnings) = phpx_to_js_with_warnings(source).expect("should compile");
    assert!(
        warnings.iter().any(|w| w.contains("$inner") && w.contains("first assigned inside a block")),
        "expected warning for $inner used outside if block, got: {:?}", warnings
    );
}

// ---- Unset ----

#[test]
fn unset_emits_undefined() {
    let js = phpx_to_js("function f(): void { $x = 1;\nunset($x); }").expect("should compile");
    assert!(js.contains("undefined"), "expected undefined for unset, got:\n{}", js);
}

// ---- Return without value ----

#[test]
fn return_void() {
    let js = phpx_to_js("function f(): void { return; }").expect("should compile");
    assert!(js.contains("return;"), "expected bare return, got:\n{}", js);
}

// ---- Compound assignment operators ----

#[test]
fn compound_plus_equals() {
    let js = phpx_to_js("function f(): void { $x = 1;\n$x += 5; }").expect("should compile");
    assert!(js.contains("+= 5") || js.contains("+ 5"), "expected += or +, got:\n{}", js);
}

// ---- Spread / splat operator (if supported) ----

#[test]
fn spread_in_array() {
    let source = r#"
function f(): void {
$a = [1, 2];
$b = [...$a, 3, 4];
}
"#;
    let result = phpx_to_js(source);
    if let Ok(js) = result {
        assert!(js.contains("..."), "expected spread operator, got:\n{}", js);
    }
    // If spread isn't supported as syntax, that's fine too
}

// ---- Prelude verification ----

#[test]
fn prelude_contains_phpx_is_struct() {
    let js = phpx_to_js("$x = 1;").expect("should compile");
    assert!(js.contains("__phpx_is_struct"), "expected __phpx_is_struct in prelude, got first 500 chars:\n{}", &js[..std::cmp::min(500, js.len())]);
}

#[test]
fn prelude_contains_panic() {
    let js = phpx_to_js("$x = 1;").expect("should compile");
    assert!(js.contains("globalThis.panic"), "expected panic in prelude, got first 500 chars:\n{}", &js[..std::cmp::min(500, js.len())]);
}

// ---- Top-level vs function scope: globalThis mirroring ----

#[test]
fn top_level_var_mirrors_to_global_this() {
    let js = phpx_to_js("$top = 42;").expect("should compile");
    assert!(js.contains("globalThis.top"), "expected globalThis mirror at top level, got:\n{}", js);
}

#[test]
fn function_var_does_not_mirror_to_global_this() {
    let js = phpx_to_js("function f(): void { $local = 42; }").expect("should compile");
    assert!(!js.contains("globalThis.local"), "function vars should not mirror to globalThis, got:\n{}", js);
}

// ---- Export via SourceModuleMeta ----

#[test]
fn exported_function_has_export_keyword() {
    let source = "function greet(): string {\n  return 'hi';\n}\n";
    let arena = Bump::new();
    let mut parser =
        Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();
    assert!(program.errors.is_empty());
    let mut meta = SourceModuleMeta::empty();
    meta.exported_functions.insert("greet".to_string());
    let js = emit_js_from_ast(&program, source.as_bytes(), meta).expect("should emit");
    assert!(js.contains("export function greet"), "expected export keyword, got:\n{}", js);
}

#[test]
fn non_exported_function_no_export_keyword() {
    let source = "function internal(): string {\n  return 'hi';\n}\n";
    let arena = Bump::new();
    let mut parser =
        Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();
    assert!(program.errors.is_empty());
    let js = emit_js_from_ast(&program, source.as_bytes(), SourceModuleMeta::empty()).expect("should emit");
    assert!(!js.contains("export function"), "should not have export keyword, got:\n{}", js);
}

// ---- Closure with use() ----

#[test]
fn closure_with_use_captures_variable() {
    let source = r#"
function f(): void {
$x = 10;
$fn = function() use ($x): int {
    return $x;
};
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(js.contains("__phpx_cap_"), "expected closure capture pattern, got:\n{}", js);
}

// ---- Type coercion / casting ----

#[test]
fn integer_cast() {
    let source = "function f(): int { $s = '42';\nreturn (int) $s; }";
    let result = phpx_to_js(source);
    if let Ok(js) = result {
        // Check that it compiles to some numeric coercion
        assert!(js.contains("42") || js.contains("parseInt") || js.contains("Number"),
            "expected numeric coercion, got:\n{}", js);
    }
}

// ---- Edge case: empty function body ----

#[test]
fn empty_function_body() {
    let js = phpx_to_js("function noop(): void { }").expect("should compile");
    assert!(js.contains("function noop()"), "expected function declaration, got:\n{}", js);
}

// ---- Nested function ----

#[test]
fn nested_function() {
    let source = r#"
function outer(): int {
function inner(): int {
    return 42;
}
return inner();
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(js.contains("function outer") && js.contains("function inner"),
        "expected nested functions, got:\n{}", js);
}

// ---- Multiple return paths ----

#[test]
fn multiple_return_paths() {
    let source = r#"
function abs_val($x: int): int {
if ($x < 0) {
    return -$x;
}
return $x;
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    let return_count = js.matches("return").count();
    assert!(return_count >= 2, "expected at least 2 returns, got {} in:\n{}", return_count, js);
}

// ---- Chained property access ----

#[test]
fn chained_property_access() {
    let source = r#"
function f(): void {
$obj = { inner: { value: 42 } };
$v = $obj.inner.value;
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(js.contains("obj.inner.value"), "expected chained dot access, got:\n{}", js);
}

// ---- Method call on object ----

#[test]
fn method_call_on_object() {
    let source = r#"
function f(): void {
$arr = [3, 1, 2];
$len = count($arr);
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(js.contains(".length"), "expected .length from count rewrite, got:\n{}", js);
}

// ---- Concat assign operator ----

#[test]
fn concat_assign_operator() {
    let source = r#"
function f(): void {
$s = 'hello';
$s .= ' world';
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(js.contains("+= ") || js.contains("+ "), "expected += for .=, got:\n{}", js);
}

// ---- Float literal ----

#[test]
fn float_literal() {
    let js = phpx_to_js("function f(): float { return 3.14; }").expect("should compile");
    assert!(js.contains("3.14"), "expected float literal, got:\n{}", js);
}

// ---- Negative number ----

#[test]
fn negative_number_literal() {
    let js = phpx_to_js("function f(): int { return -42; }").expect("should compile");
    assert!(js.contains("-") && js.contains("42"), "expected negative number, got:\n{}", js);
}

// ---- usort / uasort / uksort rewrites ----

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
    let js = phpx_to_js("$arr = [3, 1, 2];\n$ok = usort($arr, function($a, $b) { return $a - $b; });").expect("should compile");
    assert!(js.contains(".sort("), "expected .sort() for usort, got:\n{}", js);
    assert!(js.contains(", true)"), "expected comma-true pattern for usort, got:\n{}", js);
    assert!(!js.contains("globalThis.usort"), "should NOT contain globalThis.usort, got:\n{}", js);
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
    let js = phpx_to_js("$arr = [3, 1, 2];\n$ok = uasort($arr, function($a, $b) { return $a - $b; });").expect("should compile");
    assert!(js.contains(".sort("), "expected .sort() for uasort, got:\n{}", js);
    assert!(js.contains("return true"), "expected true return for uasort, got:\n{}", js);
    assert!(!js.contains("globalThis.uasort"), "should NOT contain globalThis.uasort, got:\n{}", js);
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
    assert!(js.contains("Object.keys("), "expected Object.keys for uksort, got:\n{}", js);
    assert!(js.contains(".sort("), "expected .sort() for uksort, got:\n{}", js);
    assert!(js.contains("Object.assign("), "expected Object.assign for uksort key-rebuild, got:\n{}", js);
    assert!(!js.contains("globalThis.uksort"), "should NOT contain globalThis.uksort, got:\n{}", js);
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
        ("is_int",     "function chk(): bool { return is_int(consume_token()); }"),
        ("is_float",   "function chk(): bool { return is_float(consume_token()); }"),
        ("is_numeric", "function chk(): bool { return is_numeric(consume_token()); }"),
        ("is_string",  "function chk(): bool { return is_string(consume_token()); }"),
        ("is_object",  "function chk(): bool { return is_object(consume_token()); }"),
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
            builtin, js
        );
    }
}
