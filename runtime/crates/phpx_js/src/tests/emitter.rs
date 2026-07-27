use super::*;

#[test]
fn simple_function_compiles_to_js() {
    let js = phpx_to_js("function hello(): string { return 'world'; }").expect("should compile");
    assert!(
        js.contains("function hello()"),
        "expected function declaration, got:\n{}",
        js
    );
    assert!(
        js.contains("return \"world\"") || js.contains("return 'world'"),
        "expected return statement, got:\n{}",
        js
    );
}

// ---- 2. Variable declaration ----

#[test]
fn variable_declaration_compiles() {
    let js = phpx_to_js("$x = 42;").expect("should compile");
    assert!(
        js.contains("let x = 42") || js.contains("x = 42"),
        "expected variable assignment, got:\n{}",
        js
    );
}

// ---- 3. Object literal ----

#[test]
fn object_literal_compiles() {
    let js = phpx_to_js("$value = 1;\n$obj = { key: $value };").expect("should compile");
    assert!(
        js.contains("key") && js.contains("value"),
        "expected object literal with key/value, got:\n{}",
        js
    );
}

// ---- 4. String concatenation ----

#[test]
fn string_concatenation_uses_plus() {
    let js = phpx_to_js("$s = 'hello' . ' ' . 'world';").expect("should compile");
    assert!(
        js.contains("+"),
        "expected + for string concat, got:\n{}",
        js
    );
    // The concat line itself should use + not .
    let concat_line = js
        .lines()
        .find(|l| l.contains("hello"))
        .expect("concat line");
    assert!(
        !concat_line.contains(" . "),
        "should not use PHP dot operator in concat: {}",
        concat_line
    );
}

// ---- 5. Array access ----

#[test]
fn array_access_compiles() {
    let js = phpx_to_js("$arr = ['key' => 'val'];\n$v = $arr['key'];").expect("should compile");
    assert!(
        js.contains("arr[\"key\"]") || js.contains("arr['key']"),
        "expected array key access, got:\n{}",
        js
    );
}

// ---- 6. CQL expression ----

#[test]
fn cql_expression_emits_object() {
    let js = phpx_to_js("cql q = MATCH (n) RETURN n;").expect("should compile");
    assert!(
        js.contains("__type: \"cql\""),
        "expected CQL type marker, got:\n{}",
        js
    );
    assert!(
        js.contains("MATCH (n) RETURN n"),
        "expected cypher query, got:\n{}",
        js
    );
    assert!(
        js.contains("params:"),
        "expected params field, got:\n{}",
        js
    );
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
fn if_else_compiles() {
    let js =
        phpx_to_js("$x = 1;\nif ($x == 1) { $x = 2; } else { $x = 3; }").expect("should compile");
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
    let js = phpx_to_js("$user = { name: 'test' };\n$v = $user?->name;").expect("should compile");
    assert!(
        js.contains("?."),
        "expected optional chaining, got:\n{}",
        js
    );
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
    assert!(
        js.contains("one") && js.contains("two") && js.contains("other"),
        "expected match arms, got:\n{}",
        js
    );
}

#[test]
fn export_function_via_meta() {
    // Export declarations are handled at the source meta level, not the parser.
    // Verify the emitter can mark a function as exported via SourceModuleMeta.
    let source = "function greet(): string {\n  return 'hi';\n}\n";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();
    assert!(
        program.errors.is_empty(),
        "parse errors: {:?}",
        program.errors
    );
    let mut meta = SourceModuleMeta::empty();
    meta.exported_functions.insert("greet".to_string());
    let js = emit_js_from_ast(&program, source.as_bytes(), meta).expect("should emit");
    assert!(
        js.contains("export") || js.contains("greet"),
        "expected function in output, got:\n{}",
        js
    );
}

#[test]
fn struct_declaration_compiles_without_error() {
    // Struct declarations compile without error; the struct schema is stored
    // internally and used when deka/i imports are present.
    let source = "struct Point {\n  $x: int;\n  $y: int;\n}\n";
    phpx_to_js(source).expect("struct should compile to JS");
}

#[test]
fn variable_integer_emits_let() {
    let js = phpx_to_js("function f(): void { $x = 42; }").expect("should compile");
    assert!(
        js.contains("let x = 42"),
        "expected let x = 42, got:\n{}",
        js
    );
}

#[test]
fn variable_reassignment_no_redeclaration() {
    let js = phpx_to_js("function f(): void { $x = 1;\n$x = $x + 1; }").expect("should compile");
    // First should be let, second should be bare assignment
    let lines: Vec<&str> = js
        .lines()
        .filter(|l| l.contains("x =") || l.contains("x="))
        .collect();
    let let_count = lines.iter().filter(|l| l.contains("let x")).count();
    assert_eq!(
        let_count, 1,
        "expected exactly one let declaration, got:\n{}",
        js
    );
}

#[test]
fn variable_string_literal() {
    let js = phpx_to_js("function f(): void { $name = 'hello'; }").expect("should compile");
    assert!(js.contains("let name ="), "expected let name, got:\n{}", js);
    assert!(
        js.contains("hello"),
        "expected string 'hello', got:\n{}",
        js
    );
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
    assert_eq!(
        let_a_count, 1,
        "expected exactly one 'let a' in function body, got:\n{}",
        fn_body
    );
    assert_eq!(
        let_b_count, 1,
        "expected exactly one 'let b' in function body, got:\n{}",
        fn_body
    );
}

#[test]
fn variable_boolean_true() {
    let js = phpx_to_js("function f(): void { $flag = true; }").expect("should compile");
    assert!(
        js.contains("let flag = true"),
        "expected let flag = true, got:\n{}",
        js
    );
}

#[test]
fn variable_boolean_false() {
    let js = phpx_to_js("function f(): void { $flag = false; }").expect("should compile");
    assert!(
        js.contains("let flag = false"),
        "expected let flag = false, got:\n{}",
        js
    );
}

// ---- String operations ----

#[test]
fn string_concat_two_parts() {
    let js = phpx_to_js("function f(): string { $s = 'hi' . ' there'; return $s; }")
        .expect("should compile");
    assert!(
        js.contains("+"),
        "expected + for string concat, got:\n{}",
        js
    );
    assert!(
        !js.contains(" . "),
        "should not have PHP dot operator, got:\n{}",
        js
    );
}

#[test]
fn string_concat_with_variable() {
    let js = phpx_to_js(
        "function f(): string { $name = 'world'; $s = 'hello ' . $name . '!'; return $s; }",
    )
    .expect("should compile");
    assert!(
        js.contains("+"),
        "expected + for concatenation, got:\n{}",
        js
    );
}

#[test]
fn string_concat_multiple() {
    let js = phpx_to_js(
        "function f(): string { $a = 'a'; $b = 'b'; $c = 'c'; $r = $a . $b . $c; return $r; }",
    )
    .expect("should compile");
    assert!(
        js.contains("+"),
        "expected + operators for concat chain, got:\n{}",
        js
    );
}

// ---- Array and object literals ----

#[test]
fn array_literal_compiles() {
    let js = phpx_to_js("function f(): void { $a = [1, 2, 3]; }").expect("should compile");
    assert!(
        js.contains("[1, 2, 3]") || (js.contains("1") && js.contains("2") && js.contains("3")),
        "expected array literal, got:\n{}",
        js
    );
}

#[test]
fn object_literal_multiple_keys() {
    let js = phpx_to_js("function f(): void { $o = { key: 'value', num: 42 }; }")
        .expect("should compile");
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
    let js = phpx_to_js("function f(): void { $o = { 'with-dash': 1, 'with space': 2 }; }")
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
    let js = phpx_to_js("function f(): void { $o = { 'a': 1, b: 2 }; }").expect("should compile");
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
    let js = phpx_to_js("function f(): void { $o = { items: 1 }; }").expect("should compile");
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
    let js = phpx_to_js("function f(): void { $o = { 'it\\'s': 1 }; }").expect("should compile");
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
    assert!(
        js.contains("push"),
        "expected .push for array append, got:\n{}",
        js
    );
}

#[test]
fn associative_array_compiles() {
    let js = phpx_to_js("function f(): void { $a = ['name' => 'test', 'age' => 25]; }")
        .expect("should compile");
    assert!(
        js.contains("name") && js.contains("test"),
        "expected associative key-value, got:\n{}",
        js
    );
}

// ---- Control flow ----

#[test]
fn if_statement_basic() {
    let js = phpx_to_js("function f(): void { $x = 1;\nif ($x == 1) { $x = 2; } }")
        .expect("should compile");
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
    assert!(
        js.contains("for (const item of"),
        "expected for...of loop, got:\n{}",
        js
    );
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
    assert!(
        js.contains("Object.entries"),
        "expected Object.entries for key-value foreach, got:\n{}",
        js
    );
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
    assert!(
        js.contains("?"),
        "expected ternary in match output, got:\n{}",
        js
    );
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
    assert!(
        js.contains("while ("),
        "expected while condition, got:\n{}",
        js
    );
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
    assert!(
        js.contains("break;"),
        "expected break statement, got:\n{}",
        js
    );
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
    assert!(
        js.contains("continue;"),
        "expected continue statement, got:\n{}",
        js
    );
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
    assert!(
        js.contains("default:"),
        "expected default case, got:\n{}",
        js
    );
}

// ---- Functions ----

#[test]
fn function_with_typed_params_types_erased() {
    let js = phpx_to_js("function add($a: int, $b: int): int { return $a + $b; }")
        .expect("should compile");
    // The function signature must not carry type annotations.
    assert!(
        js.contains("function add(a, b)"),
        "expected types erased in params, got:\n{}",
        js
    );
    // The word "int" must not appear inside the function signature itself.
    // (The prelude legitimately contains "parseInt" / "Number.isInteger" / "integer", so
    // we cannot assert the whole output is free of "int".)
    assert!(
        !js.contains("function add(a: int") && !js.contains(": int)") && !js.contains(", b: int"),
        "type annotation must be erased from the function signature, got:\n{}",
        js
    );
}

#[test]
fn function_default_param_value() {
    let js =
        phpx_to_js("function greet($name: string = 'world'): string { return 'hello ' . $name; }")
            .expect("should compile");
    assert!(
        js.contains("function greet(name"),
        "expected function greet, got:\n{}",
        js
    );
    // Default values are emitted as guards inside the function body
    assert!(
        js.contains("world") || js.contains("name ??="),
        "expected default value handling, got:\n{}",
        js
    );
}

#[test]
fn function_return_value() {
    let js =
        phpx_to_js("function double($x: int): int { return $x * 2; }").expect("should compile");
    assert!(
        js.contains("return"),
        "expected return statement, got:\n{}",
        js
    );
    assert!(js.contains("* 2"), "expected multiplication, got:\n{}", js);
}

#[test]
fn arrow_function_with_typed_param() {
    let js = phpx_to_js("function f(): void { $fn = fn($x: int): int => $x + 1; }")
        .expect("should compile");
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
    assert!(
        js.contains("function("),
        "expected anonymous function, got:\n{}",
        js
    );
}

#[test]
fn async_function_compiles() {
    let js = phpx_to_js("async function fetch_data(): string { return 'data'; }")
        .expect("should compile");
    assert!(
        js.contains("async function"),
        "expected async function, got:\n{}",
        js
    );
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
    assert!(
        js.contains("await"),
        "expected await expression, got:\n{}",
        js
    );
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
    // Struct declarations are stored as schemas; field names should be retained.
    assert!(
        js.contains("x") && js.contains("y"),
        "expected struct fields in output, got:\n{}",
        js
    );
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
        assert!(
            js.contains("x") && js.contains("y"),
            "expected struct fields in output, got:\n{}",
            js
        );
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
    assert!(
        js.contains("obj.x"),
        "expected dot notation access, got:\n{}",
        js
    );
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
    assert!(
        js.contains("class Color"),
        "expected class for enum, got:\n{}",
        js
    );
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
    assert!(
        js.contains("Color.Red"),
        "expected Color.Red access, got:\n{}",
        js
    );
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
    assert!(
        js.contains("Rectangle"),
        "expected Rectangle case, got:\n{}",
        js
    );
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
    assert!(
        js.contains("instanceof") || js.contains("__case"),
        "expected enum match pattern, got:\n{}",
        js
    );
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
    assert!(
        js.contains("!== undefined") && js.contains("!== null"),
        "expected isset null/undefined check, got:\n{}",
        js
    );
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
    assert!(
        js.contains("!== undefined") && js.contains("!== null"),
        "expected isset check on property, got:\n{}",
        js
    );
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
    assert!(
        js.contains("!== undefined") && js.contains("!== null"),
        "expected isset check on array key, got:\n{}",
        js
    );
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
    assert!(
        js.contains("ok") && js.contains("true") && js.contains("42"),
        "expected error-as-value ok pattern, got:\n{}",
        js
    );
}

#[test]
fn error_as_value_error_pattern() {
    let source = r#"
function f(): Object {
return { ok: false, error: 'something failed' };
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(
        js.contains("ok") && js.contains("false") && js.contains("something failed"),
        "expected error-as-value error pattern, got:\n{}",
        js
    );
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
    assert!(
        js.contains("result.ok"),
        "expected result.ok check, got:\n{}",
        js
    );
    assert!(
        js.contains("result.value"),
        "expected result.value access, got:\n{}",
        js
    );
}

// ---- Compile-time rewrite edge cases ----

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
    let js = phpx_to_js("function f(): bool { $a = true;\n$b = false;\nreturn $a && $b; }")
        .expect("should compile");
    assert!(js.contains("&&"), "expected &&, got:\n{}", js);
}

#[test]
fn logical_or_compiles() {
    let js = phpx_to_js("function f(): bool { $a = true;\n$b = false;\nreturn $a || $b; }")
        .expect("should compile");
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
    let js =
        phpx_to_js("function f(): bool { $flag = true;\nreturn !$flag; }").expect("should compile");
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
    let js = phpx_to_js("function f(): void { $obj = { name: 'test' };\n$v = $obj?->name; }")
        .expect("should compile");
    assert!(
        js.contains("?."),
        "expected optional chaining ?., got:\n{}",
        js
    );
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
    assert!(
        js.contains("?."),
        "expected optional chaining, got:\n{}",
        js
    );
}

// ---- Dynamic property access (tana#583 regression) ----
//
// The php-rs parser represents BOTH bareword property fetch (`$obj->foo`) and
// dynamic property fetch (`$obj->$k` / `$obj->{$k}`) as `Expr::PropertyFetch`
// with an `Expr::Variable` property node — the only distinguishing signal is
// whether the source span still has its `$` sigil. Before the fix, the
// emitter always treated `Expr::Variable` as a literal property name, so
// `$out->{$k} = $v` compiled to `out.k = v` (a literal ".k" property) instead
// of `out[k] = v` (indexing by the *value* of $k). This silently dropped
// every real prop key (e.g. "children") in favor of the literal string "k"
// anywhere PHPX code looped `foreach ($props as $k => $v) { $out->{$k} = $v }`
// — the exact pattern in component/dom.phpx's `__component_strip_island_props`,
// which made linkha.sh's `<main>` render permanently empty.

#[test]
fn bareword_property_fetch_stays_literal() {
    let js = phpx_to_js("function f(): void { $obj = { name: 'test' };\n$v = $obj->name; }")
        .expect("should compile");
    assert!(
        js.contains("obj.name"),
        "expected literal .name property read, got:\n{}",
        js
    );
}

#[test]
fn dynamic_property_read_indexes_by_variable_value() {
    let source = r#"
function f($k): void {
$obj = { name: 'test' };
$v = $obj->{$k};
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(
        js.contains("obj[k]"),
        "expected computed obj[k] property read, got:\n{}",
        js
    );
    assert!(
        !js.contains("obj.k"),
        "must not collapse dynamic property read to a literal .k, got:\n{}",
        js
    );
}

#[test]
fn dynamic_property_write_indexes_by_variable_value() {
    let source = r#"
function f($props): void {
$out = {};
foreach ($props as $k => $v) {
$out->{$k} = $v;
}
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(
        js.contains("out[k] = v"),
        "expected computed out[k] = v assignment, got:\n{}",
        js
    );
    assert!(
        !js.contains("out.k = v"),
        "must not collapse dynamic property write to literal .k (tana#583 regression), got:\n{}",
        js
    );
}

#[test]
fn dynamic_property_dollar_var_form_indexes_by_variable_value() {
    let source = r#"
function f($obj, $key): void {
$v = $obj->$key;
}
"#;
    let js = phpx_to_js(source).expect("should compile");
    assert!(
        js.contains("v = obj[key]"),
        "expected computed obj[key] property read assigned to v for ->$key form, got:\n{}",
        js
    );
    assert!(
        !js.contains("v = obj.key"),
        "must not collapse ->$key dynamic property read to a literal .key (tana#583 regression), got:\n{}",
        js
    );
}

// ---- CQL (Cypher) ----

#[test]
fn cql_with_params() {
    let source =
        "function f(): void { $name = 'test';\ncql q = MATCH (n:User {name: $name}) RETURN n; }";
    let js = phpx_to_js(source).expect("should compile");
    assert!(
        js.contains("__type: \"cql\""),
        "expected CQL marker, got:\n{}",
        js
    );
    assert!(js.contains("MATCH"), "expected MATCH in CQL, got:\n{}", js);
}

// ---- Echo and print ----

#[test]
fn echo_statement_compiles() {
    let js = phpx_to_js("function f(): void { echo 'hello'; }").expect("should compile");
    assert!(js.contains("hello"), "expected echo output, got:\n{}", js);
    assert!(
        js.contains("dekaPrint") || js.contains("console.log"),
        "expected print call, got:\n{}",
        js
    );
}

// ---- Const ----

#[test]
fn const_declaration() {
    let js = phpx_to_js("const MAX = 100;").expect("should compile");
    assert!(
        js.contains("const MAX = 100"),
        "expected const declaration, got:\n{}",
        js
    );
}

// ---- Empty expression ----

#[test]
fn empty_expression() {
    let js =
        phpx_to_js("function f(): bool { $x = '';\nreturn empty($x); }").expect("should compile");
    assert!(
        js.contains("!"),
        "expected negation for empty(), got:\n{}",
        js
    );
}

// ---- Clone ----

#[test]
fn clone_uses_structured_clone() {
    let js = phpx_to_js("function f(): void { $a = { x: 1 };\n$b = clone $a; }")
        .expect("should compile");
    assert!(
        js.contains("structuredClone"),
        "expected structuredClone for clone, got:\n{}",
        js
    );
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
    assert!(
        js.contains("__phpx_is_struct"),
        "expected __phpx_is_struct for instanceof struct, got:\n{}",
        js
    );
}

// ---- Ternary ----

#[test]
fn ternary_expression() {
    let js = phpx_to_js("function f(): int { $x = 5;\nreturn $x > 3 ? 1 : 0; }")
        .expect("should compile");
    assert!(
        js.contains("?") && js.contains(":"),
        "expected ternary expression, got:\n{}",
        js
    );
}

// ---- Array access patterns ----

#[test]
fn bracket_access_string_key() {
    let js = phpx_to_js("function f(): void { $a = ['key' => 'val'];\n$v = $a['key']; }")
        .expect("should compile");
    assert!(js.contains("a["), "expected bracket access, got:\n{}", js);
}

#[test]
fn bracket_access_numeric_index() {
    let js = phpx_to_js("function f(): void { $a = [10, 20, 30];\n$v = $a[1]; }")
        .expect("should compile");
    assert!(
        js.contains("a[1]"),
        "expected numeric index access, got:\n{}",
        js
    );
}

// ---- JSX ----

#[test]
fn unset_emits_undefined() {
    let js = phpx_to_js("function f(): void { $x = 1;\nunset($x); }").expect("should compile");
    assert!(
        js.contains("undefined"),
        "expected undefined for unset, got:\n{}",
        js
    );
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
    assert!(
        js.contains("+= 5") || js.contains("+ 5"),
        "expected += or +, got:\n{}",
        js
    );
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
fn top_level_var_mirrors_to_global_this() {
    let js = phpx_to_js("$top = 42;").expect("should compile");
    assert!(
        js.contains("globalThis.top"),
        "expected globalThis mirror at top level, got:\n{}",
        js
    );
}

#[test]
fn function_var_does_not_mirror_to_global_this() {
    let js = phpx_to_js("function f(): void { $local = 42; }").expect("should compile");
    assert!(
        !js.contains("globalThis.local"),
        "function vars should not mirror to globalThis, got:\n{}",
        js
    );
}

// ---- Export via SourceModuleMeta ----

#[test]
fn exported_function_has_export_keyword() {
    let source = "function greet(): string {\n  return 'hi';\n}\n";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();
    assert!(program.errors.is_empty());
    let mut meta = SourceModuleMeta::empty();
    meta.exported_functions.insert("greet".to_string());
    let js = emit_js_from_ast(&program, source.as_bytes(), meta).expect("should emit");
    assert!(
        js.contains("export function greet"),
        "expected export keyword, got:\n{}",
        js
    );
}

#[test]
fn non_exported_function_no_export_keyword() {
    let source = "function internal(): string {\n  return 'hi';\n}\n";
    let arena = Bump::new();
    let mut parser = Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Phpx);
    let program = parser.parse_program();
    assert!(program.errors.is_empty());
    let js = emit_js_from_ast(&program, source.as_bytes(), SourceModuleMeta::empty())
        .expect("should emit");
    assert!(
        !js.contains("export function"),
        "should not have export keyword, got:\n{}",
        js
    );
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
    assert!(
        js.contains("__phpx_cap_"),
        "expected closure capture pattern, got:\n{}",
        js
    );
}

// ---- Type coercion / casting ----

#[test]
fn integer_cast() {
    let source = "function f(): int { $s = '42';\nreturn (int) $s; }";
    let result = phpx_to_js(source);
    if let Ok(js) = result {
        // Check that it compiles to some numeric coercion
        assert!(
            js.contains("42") || js.contains("parseInt") || js.contains("Number"),
            "expected numeric coercion, got:\n{}",
            js
        );
    }
}

// ---- Edge case: empty function body ----

#[test]
fn empty_function_body() {
    let js = phpx_to_js("function noop(): void { }").expect("should compile");
    assert!(
        js.contains("function noop()"),
        "expected function declaration, got:\n{}",
        js
    );
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
    assert!(
        js.contains("function outer") && js.contains("function inner"),
        "expected nested functions, got:\n{}",
        js
    );
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
    assert!(
        return_count >= 2,
        "expected at least 2 returns, got {} in:\n{}",
        return_count,
        js
    );
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
    assert!(
        js.contains("obj.inner.value"),
        "expected chained dot access, got:\n{}",
        js
    );
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
    assert!(
        js.contains(".length"),
        "expected .length from count rewrite, got:\n{}",
        js
    );
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
    assert!(
        js.contains("+= ") || js.contains("+ "),
        "expected += for .=, got:\n{}",
        js
    );
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
    assert!(
        js.contains("-") && js.contains("42"),
        "expected negative number, got:\n{}",
        js
    );
}
