# Unified Pending Tasks (PHPX)

This file is the single, merged list of all *pending* work items across the
current PHPX effort. It is a union of the outstanding tasks from:

- `tasks/TASKS.md`
- `tasks/VALIDATION-STATUS.md`
- `tasks/PHPX-VALIDATION.md`
- `tasks/DX-TASKS.md`

Notes:
- Items are grouped by area, but all pending tasks are included.
- Where tasks were listed without checkboxes in the source docs (especially
  `tasks/VALIDATION-STATUS.md`), they are listed explicitly here.
- Duplicates are kept only when they add distinct detail.

## Process (applies to every task)
- Always run relevant tests after completing a task before moving to the next.
- Always make a git commit after tests pass and before starting the next task.

## Local build/test loop notes
- Rebuild the CLI in release mode: `cargo build --release -p cli`
- The locally wired `deka` CLI points to the release binary: `target/release/cli`
  (use this for testing after rebuilding).
- Packaging note: ship `phpx_lsp` alongside `deka` in installers/scripts so `deka lsp`
  can exec it (or set `PHPX_LSP_BIN`).

## Runtime + Language Hardening (from `tasks/TASKS.md`)
1. [x] Fix namespaced struct coercion (type metadata should use fully-qualified struct names).
2. [x] Resolve type aliases for applied types (e.g., `type Maybe<T> = Option<T>`).
3. [x] Define `Result` array schema rules (`ok` boolean vs truthy) and align coercion.
4. [x] Decide/document missing struct fields behavior (defaults vs unset).
5. [x] Add match expression inference (union of arm types) so `match` participates in return/assignment typing.
6. [x] Infer type params for `array<T>` from array literals / `Type::Array` actuals.
7. [x] Infer type params for `Option<T>` / `Result<T,E>` when actuals are enum cases (`Option::Some`, `Result::Ok`, `Result::Err`).
8. [x] Add method-call type checking for structs/interfaces (arity + arg types).
9. [x] Add dot-access typing for promoted embedded fields in inference (if any gap remains).
10. [x] Add `unset($obj.field)` support for dot access (ObjectMap + struct).
11. [x] Support `->` property access on ObjectMap for PHP compatibility
12. [x] Decide how ObjectMap crosses PHP boundary: keep ObjectMap or coerce to stdClass.
13. [x] Audit core object helpers (`get_class`, `property_exists`, `method_exists`, `count`) for ObjectMap/Struct semantics and document/implement decisions.
14. [x] Define object-literal equality semantics (`==`/`===`) and implement deep comparison.
15. [x] Add tests for dot-unset + object-literal equality (value semantics).
16. [x] Implement JSX validation pass (syntax/expressions/components) as outlined in `tasks/PHPX-VALIDATION.md`.
17. [x] Enforce JSX expression rules (no statements; object literals require `{{ }}`).
18. [x] Add optional JSX/VNode type inference (e.g., `VNode` return type for components).
19. [x] Decide on JSX whitespace normalization (current renderer trims text nodes).
20. [x] Verify unused-import detection in presence of synthetic JSX/runtime imports (avoid false positives/negatives).
21. [x] Decide whether `import` in `.php` should allow additional PHP statements before it when `<?php` is present (currently must be first non-empty line).
22. [x] Add explicit tests for duplicate imports, duplicate export aliases, and cyclic imports.
23. [x] Clarify behavior of `phpx_import()` when module load fails (panic/trigger_error/echo).
24. [x] Allow unary +/− constant expressions in struct defaults (e.g. `$x: int = -1`).
25. [x] Decide whether object/struct literals should be permitted as struct defaults; if yes, extend constant-expr validation + runtime init.
26. [x] ContextProvider should push/pop context even when JSX passes callable (not string).
27. [x] Decide on `createRoot` `mode` support (implement or remove + document).
28. [x] Implement or remove `Link` prefetch option (currently unused in hydration).
29. [x] Add helper to emit partial JSON responses with proper headers (or document required headers in examples).
30. [x] Clarify layout semantics (where layout id is set and when partial navigation falls back).
31. [x] Fix PHPX eval frame depth so eval'd PHPX can call functions (no stack underflow).
32. [x] Read export signatures from `__PHPX_TYPES` registry (avoid namespace-scoped type vars).
33. [x] Auto-add `core/option` + `core/result` as deps when referenced in PHPX modules.

### Phase 7 Tests/Docs (from `tasks/TASKS.md`, non-checkbox items)
1. [x] Add PHP<->PHPX bridge tests for `Option<T>` (null -> None, Some -> value, return mapping).
2. [x] Add PHP<->PHPX bridge tests for `Result<T,E>` (Ok/Err return mapping; array/stdClass coercions).
3. [x] Add PHP<->PHPX bridge tests for object/object-shape + struct param coercion (extra keys ignored).
4. [x] Add runtime coverage for module isolation + unused import errors.
5. [x] Add tests/docs for frontmatter templates + `<Hydration />`.
6. [x] Deduplicate boundary coercion bullets in `docs/phpx/phpx-dx.md`.

## Validation System (from `tasks/VALIDATION-STATUS.md` + `tasks/PHPX-VALIDATION.md`)

### Validation status gaps (explicit in `tasks/VALIDATION-STATUS.md`)
1. [x] Add `deka-validation` integration for parser errors (use formatter instead of basic text).
2. [x] Extend `ParseError` to include `error_kind` and `help_text`.
3. [x] Add PHPX error kinds (Syntax/Type/Import/Export/Null/OOP/JSX/etc.) for validation output.
4. [x] Add compiler API: `compile_phpx(source, file_path) -> ValidationResult` with structured errors.
5. [x] Return `ValidationResult { errors, warnings, ast }` instead of generic CoreError.
6. [x] Map parser errors to PHPX validation errors.

### Validation plan checklist (from `tasks/PHPX-VALIDATION.md`)
1. [x] PHPX-VALIDATION:  Unclosed braces, brackets, parentheses
2. [x] PHPX-VALIDATION:  Invalid tokens
3. [x] PHPX-VALIDATION:  Unexpected end of file
4. [x] PHPX-VALIDATION:  Malformed expressions
5. [x] PHPX-VALIDATION:  Create `crates/modules_php/src/validation/syntax.rs`
6. [x] PHPX-VALIDATION:  Implement `validate_syntax(source: &str, ast: &Program) -> Vec<ValidationError>`
7. [x] PHPX-VALIDATION:  Use php-rs parser error recovery
8. [x] PHPX-VALIDATION:  Map parser errors to validation errors
9. [x] PHPX-VALIDATION:  Add helpful suggestions for common mistakes
10. [x] PHPX-VALIDATION:  Import at top of file (before other code)
11. [x] PHPX-VALIDATION:  Valid import syntax
12. [x] PHPX-VALIDATION:  Named imports: `import { foo, bar } from 'module'`
13. [x] PHPX-VALIDATION:  WASM imports: `import { fn } from '@user/mod' as wasm`
14. [x] PHPX-VALIDATION:  Module path format (no relative paths with `../`)
15. [x] PHPX-VALIDATION:  Unused imports (warning)
16. [x] PHPX-VALIDATION:  Duplicate imports
17. [x] PHPX-VALIDATION:  Create `crates/modules_php/src/validation/imports.rs`
18. [x] PHPX-VALIDATION:  Implement `validate_imports(source: &str, file_path: &str) -> (Vec<ValidationError>, Vec<ValidationWarning>)`
19. [x] PHPX-VALIDATION:  Check import placement (AST position)
20. [x] PHPX-VALIDATION:  Validate module paths
21. [x] PHPX-VALIDATION:  Track used imports (mark on usage)
22. [x] PHPX-VALIDATION:  Detect duplicates
23. [x] PHPX-VALIDATION:  Export only functions, constants, types
24. [x] PHPX-VALIDATION:  No duplicate exports
25. [x] PHPX-VALIDATION:  Exported names actually exist
26. [x] PHPX-VALIDATION:  Re-export syntax validation
27. [x] PHPX-VALIDATION:  Template files: no explicit exports (auto-exported as Component)
28. [x] PHPX-VALIDATION:  Create `crates/modules_php/src/validation/exports.rs`
29. [x] PHPX-VALIDATION:  Implement `validate_exports(source: &str, file_path: &str, ast: &Program) -> Vec<ValidationError>`
30. [x] PHPX-VALIDATION:  Track exported names
31. [x] PHPX-VALIDATION:  Check for duplicates
32. [x] PHPX-VALIDATION:  Verify definitions exist
33. [x] PHPX-VALIDATION:  Special handling for template files
34. [x] PHPX-VALIDATION:  Valid type names
35. [x] PHPX-VALIDATION:  Generic syntax: `Option<T>`, `Result<T, E>`, `array<T>`
36. [x] PHPX-VALIDATION:  Object shape syntax: `Object<{ field: Type }>`
37. [x] PHPX-VALIDATION:  Type alias syntax: `type Name = ...`
38. [x] PHPX-VALIDATION:  Union types (limited): `int|float`
39. [x] PHPX-VALIDATION:  No nullable types (`?T`, `T|null` are banned)
40. [x] PHPX-VALIDATION:  Create `crates/modules_php/src/validation/type_syntax.rs`
41. [x] PHPX-VALIDATION:  Implement `validate_type_annotations(program: &Program, source: &str) -> Vec<ValidationError>`
42. [x] PHPX-VALIDATION:  Reject `null`, `?T`, `T|null` syntax
43. [x] PHPX-VALIDATION:  Validate generic parameter syntax
44. [x] PHPX-VALIDATION:  Check object shape syntax
45. [x] PHPX-VALIDATION:  Variable assignment type matches
46. [x] PHPX-VALIDATION:  Function parameter types match arguments
47. [x] PHPX-VALIDATION:  Return type matches returned value
48. [x] PHPX-VALIDATION:  Binary operation types compatible
49. [x] PHPX-VALIDATION:  Struct field types match literal values
50. [x] PHPX-VALIDATION:  Safe widening only (int → float allowed, not reverse)
51. [x] PHPX-VALIDATION:  Create `crates/modules_php/src/validation/type_checker.rs`
52. [x] PHPX-VALIDATION:  Implement type inference engine (via php-rs PHPX typeck)
53. [x] PHPX-VALIDATION:  Implement `check_types(program: &Program, source: &str, file_path: Option<&str>) -> Vec<ValidationError>`
54. [x] PHPX-VALIDATION:  Build type environment (symbol table)
55. [x] PHPX-VALIDATION:  Infer types for expressions
56. [x] PHPX-VALIDATION:  Check compatibility at assignments/calls/returns
57. [x] PHPX-VALIDATION:  Track widening rules
58. [x] PHPX-VALIDATION:  Generic parameters are used
59. [x] PHPX-VALIDATION:  Generic constraints are satisfied
60. [x] PHPX-VALIDATION:  Type arguments provided where required
61. [x] PHPX-VALIDATION:  Constraint syntax: `T: Reader`
62. [x] PHPX-VALIDATION:  Create `crates/modules_php/src/validation/generics.rs`
63. [x] PHPX-VALIDATION:  Implement `validate_generics(program: &Program, source: &str) -> (Vec<ValidationError>, Vec<ValidationWarning>)`
64. [x] PHPX-VALIDATION:  Track generic parameters
65. [x] PHPX-VALIDATION:  Check constraints
66. [x] PHPX-VALIDATION:  Infer type arguments
67. [x] PHPX-VALIDATION:  No `null` literals
68. [x] PHPX-VALIDATION:  No `=== null` or `!== null` comparisons
69. [x] PHPX-VALIDATION:  No `is_null()` calls
70. [x] PHPX-VALIDATION:  Suggest `Option<T>` instead
71. [x] PHPX-VALIDATION:  Create `crates/modules_php/src/validation/phpx_rules.rs`
72. [x] PHPX-VALIDATION:  Implement `validate_no_null(program: &Program, source: &str) -> Vec<ValidationError>`
73. [x] PHPX-VALIDATION:  Scan AST for null literals
74. [x] PHPX-VALIDATION:  Scan for null comparisons
75. [x] PHPX-VALIDATION:  Scan for is_null() calls
76. [x] PHPX-VALIDATION:  No `throw` statements
77. [x] PHPX-VALIDATION:  No `try/catch/finally` blocks
78. [x] PHPX-VALIDATION:  Suggest `Result<T, E>` instead
79. [x] PHPX-VALIDATION:  Allow `panic()` for unrecoverable errors
80. [x] PHPX-VALIDATION:  Add to `crates/modules_php/src/validation/phpx_rules.rs`
81. [x] PHPX-VALIDATION:  Implement `validate_no_exceptions(program: &Program, source: &str) -> Vec<ValidationError>`
82. [x] PHPX-VALIDATION:  Scan for throw statements
83. [x] PHPX-VALIDATION:  Scan for try/catch/finally
84. [x] PHPX-VALIDATION:  No `class` declarations
85. [x] PHPX-VALIDATION:  No `trait` declarations
86. [x] PHPX-VALIDATION:  No `extends` keyword
87. [x] PHPX-VALIDATION:  No `implements` keyword
88. [x] PHPX-VALIDATION:  No `new` keyword
89. [x] PHPX-VALIDATION:  No `interface` inheritance (structural interfaces only)
90. [x] PHPX-VALIDATION:  Suggest structs instead
91. [x] PHPX-VALIDATION:  Add to `crates/modules_php/src/validation/phpx_rules.rs`
92. [x] PHPX-VALIDATION:  Implement `validate_no_oop(program: &Program, source: &str) -> Vec<ValidationError>`
93. [x] PHPX-VALIDATION:  Scan for class/trait/interface declarations
94. [x] PHPX-VALIDATION:  Scan for extends/implements
95. [x] PHPX-VALIDATION:  Scan for new keyword
96. [x] PHPX-VALIDATION:  No `namespace` declarations
97. [x] PHPX-VALIDATION:  No top-level `use` statements
98. [x] PHPX-VALIDATION:  Suggest import/export instead
99. [x] PHPX-VALIDATION:  Add to `crates/modules_php/src/validation/phpx_rules.rs`
100. [x] PHPX-VALIDATION:  Implement `validate_no_namespace(program: &Program, source: &str) -> Vec<ValidationError>`
101. [x] PHPX-VALIDATION:  Scan for namespace declarations
102. [x] PHPX-VALIDATION:  Scan for top-level use statements
103. [x] PHPX-VALIDATION:  No `__construct` in PHPX structs
104. [x] PHPX-VALIDATION:  Field defaults are constant expressions
105. [x] PHPX-VALIDATION:  Field type annotations are valid
106. [x] PHPX-VALIDATION:  No duplicate field names
107. [x] PHPX-VALIDATION:  Struct composition (`use A`) is valid
108. [x] PHPX-VALIDATION:  Create `crates/modules_php/src/validation/structs.rs`
109. [x] PHPX-VALIDATION:  Implement `validate_struct_definitions(program: &Program, source: &str) -> Vec<ValidationError>`
110. [x] PHPX-VALIDATION:  Check for __construct
111. [x] PHPX-VALIDATION:  Validate field defaults are constants
112. [x] PHPX-VALIDATION:  Check for duplicate fields
113. [x] PHPX-VALIDATION:  Validate composition
114. [x] PHPX-VALIDATION:  All required fields provided
115. [x] PHPX-VALIDATION:  No extra fields
116. [x] PHPX-VALIDATION:  Field types match values
117. [x] PHPX-VALIDATION:  Shorthand syntax valid
118. [x] PHPX-VALIDATION:  Nested struct literals valid
119. [x] PHPX-VALIDATION:  Add to `crates/modules_php/src/validation/structs.rs`
120. [x] PHPX-VALIDATION:  Implement `validate_struct_literals(program: &Program, source: &str) -> Vec<ValidationError>`
121. [x] PHPX-VALIDATION:  Check required fields
122. [x] PHPX-VALIDATION:  Reject extra fields
123. [x] PHPX-VALIDATION:  Validate field types
124. [x] PHPX-VALIDATION:  Handle shorthand syntax
125. [x] PHPX-VALIDATION:  Valid tag names
126. [x] PHPX-VALIDATION:  Matching opening/closing tags
127. [x] PHPX-VALIDATION:  Valid attribute syntax
128. [x] PHPX-VALIDATION:  Fragment syntax: `<>...</>`
129. [x] PHPX-VALIDATION:  Self-closing tags
130. [x] PHPX-VALIDATION:  Create `crates/modules_php/src/validation/jsx.rs`
131. [x] PHPX-VALIDATION:  Implement `validate_jsx_syntax(program: &Program, source: &str) -> Vec<ValidationError>`
132. [x] PHPX-VALIDATION:  Check tag matching
133. [x] PHPX-VALIDATION:  Validate attribute names
134. [x] PHPX-VALIDATION:  Check self-closing vs paired tags
135. [x] PHPX-VALIDATION:  Expression syntax: `{$var}`, `{$obj.field}`
136. [x] PHPX-VALIDATION:  If blocks: `{if ($cond) { <p>yes</p> }}`
137. [x] PHPX-VALIDATION:  Foreach loops: `{foreach ($items as $item) { <li>{$item}</li> }}`
138. [x] PHPX-VALIDATION:  Object literals require double braces: `{{ key: 'value' }}`
139. [x] PHPX-VALIDATION:  No statements in expressions (only expressions)
140. [x] PHPX-VALIDATION:  Add to `crates/modules_php/src/validation/jsx.rs`
141. [x] PHPX-VALIDATION:  Implement `validate_jsx_expressions(program: &Program, source: &str) -> Vec<ValidationError>`
142. [x] PHPX-VALIDATION:  Check expression syntax
143. [x] PHPX-VALIDATION:  Validate if/foreach blocks
144. [x] PHPX-VALIDATION:  Detect statements in expressions
145. [x] PHPX-VALIDATION:  Check object literal braces
146. [x] PHPX-VALIDATION:  Component names are capitalized (or imported)
147. [x] PHPX-VALIDATION:  Built-in tags are lowercase
148. [x] PHPX-VALIDATION:  Component props match definition (if available)
149. [x] PHPX-VALIDATION:  Special components: `<Link>`, `<Hydration>`, `<ContextProvider>`
150. [x] PHPX-VALIDATION:  Add to `crates/modules_php/src/validation/jsx.rs`
151. [x] PHPX-VALIDATION:  Implement `validate_components(program: &Program, source: &str) -> Vec<ValidationError>`
152. [x] PHPX-VALIDATION:  Check component naming
153. [x] PHPX-VALIDATION:  Validate props (if type info available)
154. [x] PHPX-VALIDATION:  Track imported components
155. [x] PHPX-VALIDATION:  Frontmatter starts at beginning of file
156. [x] PHPX-VALIDATION:  Proper `---` delimiters
157. [x] PHPX-VALIDATION:  No explicit exports in template files (under `php_modules/`)
158. [x] PHPX-VALIDATION:  Template section is valid JSX
159. [x] PHPX-VALIDATION:  Imports in frontmatter only
160. [x] PHPX-VALIDATION:  Add to `crates/modules_php/src/validation/jsx.rs`
161. [x] PHPX-VALIDATION:  Implement `validate_frontmatter(source: &str, file_path: &str) -> Vec<ValidationError>`
162. [x] PHPX-VALIDATION:  Check frontmatter position
163. [x] PHPX-VALIDATION:  Validate delimiters
164. [x] PHPX-VALIDATION:  Check for exports (if in php_modules/)
165. [x] PHPX-VALIDATION:  Validate imports placement
166. [x] PHPX-VALIDATION:  Module exists in `php_modules/`
167. [x] PHPX-VALIDATION:  Module has valid entry point
168. [x] PHPX-VALIDATION:  Circular imports detected
169. [x] PHPX-VALIDATION:  Import/export names match
170. [x] PHPX-VALIDATION:  Create `crates/modules_php/src/validation/modules.rs`
171. [x] PHPX-VALIDATION:  Implement `validate_module_resolution(source: &str, file_path: &str) -> Vec<ValidationError>`
172. [x] PHPX-VALIDATION:  Scan php_modules/ for available modules
173. [x] PHPX-VALIDATION:  Build dependency graph
174. [x] PHPX-VALIDATION:  Detect cycles
175. [x] PHPX-VALIDATION:  Check export names
176. [x] PHPX-VALIDATION:  `@user/module` format
177. [x] PHPX-VALIDATION:  `deka.json` exists
178. [x] PHPX-VALIDATION:  `module.wasm` exists
179. [x] PHPX-VALIDATION:  `.d.phpx` stub file exists (suggest generating if missing)
180. [x] PHPX-VALIDATION:  Imported names exist in stubs
181. [x] PHPX-VALIDATION:  Add to `crates/modules_php/src/validation/modules.rs`
182. [x] PHPX-VALIDATION:  Implement `validate_wasm_imports(source: &str, file_path: &str) -> Vec<ValidationError>`
183. [x] PHPX-VALIDATION:  Scan php_modules/@*/ for WASM modules
184. [x] PHPX-VALIDATION:  Check deka.json, module.wasm, .d.phpx
185. [x] PHPX-VALIDATION:  Parse .d.phpx for exported names
186. [x] PHPX-VALIDATION:  Suggest deka wasm commands
187. [x] PHPX-VALIDATION:  Enum match covers all cases
188. [x] PHPX-VALIDATION:  No unreachable match arms
189. [x] PHPX-VALIDATION:  Variable binding in match arms
190. [x] PHPX-VALIDATION:  Payload destructuring correct
191. [x] PHPX-VALIDATION:  Create `crates/modules_php/src/validation/patterns.rs`
192. [x] PHPX-VALIDATION:  Implement `validate_match_exhaustiveness(program: &Program, source: &str) -> Vec<ValidationError>`
193. [x] PHPX-VALIDATION:  Build enum case registry
194. [x] PHPX-VALIDATION:  Check match coverage
195. [x] PHPX-VALIDATION:  Validate payload destructuring
196. [x] PHPX-VALIDATION:  Detect unreachable arms
197. [x] PHPX-VALIDATION:  Create `crates/modules_php/src/compiler_api.rs`
198. [x] PHPX-VALIDATION:  Expose `compile_phpx(source: &str, file_path: &str) -> ValidationResult`
199. [x] PHPX-VALIDATION:  Run all validation passes in order:
200. [x] PHPX-VALIDATION:  Collect all errors and warnings
201. [x] PHPX-VALIDATION:  Format with `deka-validation`
202. [x] PHPX-VALIDATION:  Return structured result
203. [x] PHPX-VALIDATION:  Create test file for each validation rule
204. [x] PHPX-VALIDATION:  Positive tests (valid code passes)
205. [x] PHPX-VALIDATION:  Negative tests (invalid code caught)
206. [x] PHPX-VALIDATION:  Error message quality
207. [x] PHPX-VALIDATION:  Help text accuracy
208. [x] PHPX-VALIDATION:  Edge cases
209. [x] PHPX-VALIDATION:  Support for multiple errors in single file
210. [x] PHPX-VALIDATION:  Color coding by error kind
211. [x] PHPX-VALIDATION:  Code suggestions (auto-fix hints)
212. [x] PHPX-VALIDATION:  Reference links to PHPX docs
213. [x] PHPX-VALIDATION:  `format_multiple_errors()` - Format list of errors
214. [x] PHPX-VALIDATION:  Color codes by severity (error=red, warning=yellow, info=blue)
215. [x] PHPX-VALIDATION:  `format_with_suggestion()` - Include code fix suggestions
216. [x] PHPX-VALIDATION:  `format_with_docs_link()` - Add doc links

## DX / Tooling (from `tasks/DX-TASKS.md`)
1. [x] DX-TASKS:  Create `tooling/tree-sitter-phpx/` directory
2. [x] DX-TASKS:  Clone `tree-sitter-php` as starting point
3. [x] DX-TASKS:  Rename project to `tree-sitter-phpx`
4. [x] DX-TASKS:  Update `package.json` metadata (name, description, repo)
5. [x] DX-TASKS:  Install tree-sitter CLI: `npm install -g tree-sitter-cli`
6. [x] DX-TASKS:  Verify build: `tree-sitter generate && tree-sitter test`
7. [x] DX-TASKS:  Add type annotation rules to `grammar.js`
48:  - [x] Primitive types: `int`, `string`, `bool`, `float`, `mixed`
49:  - [x] Generic types: `Option<T>`, `Result<T, E>`, `array<T>`
50:  - [x] Object types: `Object<{ field: Type }>`
51:  - [x] Type aliases: `type Name = ...`
8. [x] DX-TASKS:  Create `queries/highlights.scm` for type highlighting
9. [x] DX-TASKS:  Test with PHPX files containing type annotations
10. [x] DX-TASKS:  Verify types are highlighted differently from values
11. [x] DX-TASKS:  Add `import_statement` rule to `grammar.js`
66:  - [x] Named imports: `import { foo, bar } from 'module'`
67:  - [x] WASM imports: `import { fn } from '@user/mod' as wasm`
68:  - [x] Default import (if needed)
12. [x] DX-TASKS:  Add `export_statement` rule
70:  - [x] Export functions: `export function foo() {}`
71:  - [x] Export constants: `export const X = 1`
72:  - [x] Re-exports: `export { foo } from './bar'`
13. [x] DX-TASKS:  Add highlighting for `import`, `export`, `from`, `as` keywords
14. [x] DX-TASKS:  Test with module examples from `examples/php/modules/`
15. [x] DX-TASKS:  Add `struct_literal` rule to `grammar.js`
82:  - [x] Type name: `Point`
83:  - [x] Field list: `{ $x: 1, $y: 2 }`
84:  - [x] Shorthand: `{ $x, $y }`
16. [x] DX-TASKS:  Add highlighting for struct names and fields
17. [x] DX-TASKS:  Test with struct examples
18. [x] DX-TASKS:  Verify nested struct literals work
19. [x] DX-TASKS:  Port JSX grammar from `tree-sitter-javascript`
101:  - [x] Opening tags: `<Component>`
102:  - [x] Self-closing tags: `<Component />`
103:  - [x] Attributes: `<Component id={$val} />`
104:  - [x] Children: `<div>text</div>`
105:  - [x] Fragments: `<>...</>`
20. [x] DX-TASKS:  Add PHPX-specific JSX expressions
107:  - [x] Variables: `{$user->name}`
108:  - [x] Conditional expressions: `{$user->admin ? <Admin /> : null}`
109:  - [x] Object literals (double braces): `{{ host: 'localhost' }}`
110:  - [x] Statements are not allowed in JSX expressions (validation error; use expressions).
21. [x] DX-TASKS:  Add highlighting for tags, attributes, expressions
22. [x] DX-TASKS:  Test with component examples from `examples/phpx-components/`
23. [x] DX-TASKS:  Add `frontmatter` rule to `grammar.js`
120:  - [x] Detect `---` at start of file
121:  - [x] Parse PHPX code section
122:  - [x] Parse JSX template section
24. [x] DX-TASKS:  Add highlighting for frontmatter delimiters
25. [x] DX-TASKS:  Test with template examples
26. [x] DX-TASKS:  Verify code and template sections have correct highlighting
27. [x] DX-TASKS:  Create `extensions/phpx/` directory
28. [x] DX-TASKS:  Create `extension.toml` with PHPX language config
147:  - [x] Set file suffixes: `["phpx"]`
148:  - [x] Set comment syntax
149:  - [x] Link to tree-sitter-phpx grammar
29. [ ] DX-TASKS:  Add syntax highlighting theme overrides (if needed)
30. [ ] DX-TASKS:  Install extension in Zed:
152:  - [ ] Symlink: `ln -s /path/to/deka/extensions/phpx ~/.config/zed/extensions/phpx`
31. [ ] DX-TASKS:  Test with real PHPX files
32. [ ] DX-TASKS:  Verify highlighting works for all features
33. [x] DX-TASKS:  Create `tooling/tree-sitter-phpx/README.md`
166:  - [x] Installation instructions
167:  - [x] Testing instructions
168:  - [x] Editor integration guides (Zed, Neovim, Helix)
169:  - [x] Grammar rules overview
34. [x] DX-TASKS:  Add examples to `test/corpus/`
35. [x] DX-TASKS:  Document known limitations
36. [x] DX-TASKS:  Add contributing guidelines
37. [x] DX-TASKS:  Create `crates/phpx_lsp/` directory
38. [x] DX-TASKS:  Initialize Cargo project: `cargo new phpx_lsp --bin`
39. [x] DX-TASKS:  Add dependencies to `Cargo.toml`:
188:  - [x] `tower-lsp = "0.20"`
189:  - [x] `tokio` (workspace)
190:  - [x] `serde_json = "1"`
191:  - [x] `anyhow = "1"`
192:  - [x] `modules_php` (path dependency to existing PHPX compiler)
193:  - [x] `deka-validation` (for error formatting)
40. [x] DX-TASKS:  Add to workspace members in root `Cargo.toml`
41. [x] DX-TASKS:  Verify build: `cargo build -p phpx_lsp`
42. [x] DX-TASKS:  Create `src/main.rs` with LSP boilerplate
43. [x] DX-TASKS:  Implement `initialize` method with server capabilities:
204:  - [x] `textDocumentSync`: Full sync mode
205:  - [x] `diagnosticProvider`: Report errors
206:  - [ ] (Others later: hover, completion, etc.)
44. [x] DX-TASKS:  Implement `initialized` method (log ready message)
45. [x] DX-TASKS:  Implement `shutdown` method
46. [x] DX-TASKS:  Implement `did_open` and `did_change` handlers (log only)
47. [x] DX-TASKS:  Test with manual stdio: `echo '{"jsonrpc":"2.0","method":"initialize",...}' | cargo run`
48. [x] DX-TASKS:  Create `crates/modules_php/src/compiler_api.rs`
49. [x] DX-TASKS:  Define public structs:
50. [x] DX-TASKS:  Implement `compile_phpx(source: &str, file_path: &str) -> ValidationResult`
235:  - [x] Call existing PHPX parser/compiler
236:  - [x] Collect syntax errors
237:  - [x] Collect type errors
238:  - [x] Return structured results
51. [x] DX-TASKS:  Add unit tests for error collection
52. [x] DX-TASKS:  Export from `crates/modules_php/src/lib.rs`
53. [x] DX-TASKS:  Add `deka-validation` dependency to `phpx_lsp`
54. [x] DX-TASKS:  Implement error formatting in LSP:
55. [x] DX-TASKS:  Convert formatted errors to LSP Diagnostic messages
56. [x] DX-TASKS:  Test with PHPX files containing errors
57. [ ] DX-TASKS:  Verify beautiful error output in editor
58. [x] DX-TASKS:  Implement `validate_document` method in LSP server:
275:  - [x] Call `compile_phpx` API
276:  - [x] Convert `CompileError` to LSP `Diagnostic`
277:  - [x] Map line/column positions
278:  - [x] Set severity (Error vs Warning)
279:  - [x] Include formatted message
59. [x] DX-TASKS:  Call `client.publish_diagnostics` on document open/change
60. [ ] DX-TASKS:  Test with PHPX files containing:
282:  - [x] Syntax errors
283:  - [x] Type errors
284:  - [x] Import errors
285:  - [x] WIT import errors (missing stubs)
61. [ ] DX-TASKS:  Verify errors appear in editor as red squiggles
62. [x] DX-TASKS:  Update `extensions/phpx/extension.toml` with LSP config:
63. [ ] DX-TASKS:  Add LSP binary path to Zed settings:
64. [ ] DX-TASKS:  Rebuild LSP: `cargo build --release -p phpx_lsp`
65. [ ] DX-TASKS:  Restart Zed
66. [ ] DX-TASKS:  Test with PHPX files
67. [ ] DX-TASKS:  Verify diagnostics appear in problems panel
68. [x] DX-TASKS:  Extend compiler API to validate imports
69. [x] DX-TASKS:  Check PHPX module imports:
323:  - [x] Verify module exists in `php_modules/`
324:  - [x] Verify exported names exist
325:  - [x] Detect unused imports
326:  - [x] Detect circular imports
70. [x] DX-TASKS:  Check WASM imports:
328:  - [x] Verify `deka.json` exists
329:  - [x] Verify `module.wasm` exists
330:  - [x] Check for `.d.phpx` stub file
331:  - [x] Suggest running `deka wasm stubs` if missing
71. [x] DX-TASKS:  Add helpful error messages with fixes
72. [x] DX-TASKS:  Test with various import scenarios
73. [x] DX-TASKS:  Create `crates/phpx_lsp/README.md`
348:  - [x] Installation instructions
349:  - [x] Editor integration guides (Zed, VSCode, Neovim)
350:  - [x] Configuration options
351:  - [x] Debugging tips
74. [x] DX-TASKS:  Document compiler API in `crates/modules_php/src/compiler_api.rs`
75. [x] DX-TASKS:  Add troubleshooting section
76. [x] DX-TASKS:  List supported features and roadmap
77. [x] DX-TASKS:  Test and fix tight dot access: `$user.name.first`
78. [x] DX-TASKS:  Test and fix nested object literals in JSX: `{{ nested: { value: 1 } }}`
79. [x] DX-TASKS:  Test and fix multiline JSX expressions
80. [x] DX-TASKS:  Test and fix if/foreach blocks in JSX
81. [x] DX-TASKS:  Add error recovery rules for better partial highlighting
82. [x] DX-TASKS:  Test with malformed PHPX (ensure no crashes)
83. [x] DX-TASKS:  Create `queries/folds.scm`
84. [x] DX-TASKS:  Add folding for:
381:  - [x] Function bodies
382:  - [x] Struct definitions
383:  - [x] JSX elements
384:  - [x] If/foreach blocks
385:  - [x] Frontmatter sections
85. [ ] DX-TASKS:  Test in Zed (verify fold markers appear)
86. [x] DX-TASKS:  Create `queries/indents.scm`
87. [x] DX-TASKS:  Define indent increases for:
395:  - [x] Function bodies
396:  - [x] If/else/foreach blocks
397:  - [x] JSX children
398:  - [x] Struct literals
399:  - [x] Object literals (tooling accepts in php grammar)
88. [ ] DX-TASKS:  Test auto-indentation in Zed
89. [ ] DX-TASKS:  Verify correct indent after newline
90. [x] DX-TASKS:  Create `queries/textobjects.scm`
91. [x] DX-TASKS:  Define textobjects for:
409:  - [x] Functions (`@function.outer`, `@function.inner`)
410:  - [x] Structs (`@struct.outer`, `@struct.inner`)
411:  - [x] JSX elements (`@jsx.outer`, `@jsx.inner`)
412:  - [x] Parameters (`@parameter.outer`)
92. [ ] DX-TASKS:  Test in Neovim (via nvim-treesitter)
93. [x] DX-TASKS:  Document textobject usage
94. [x] DX-TASKS:  Add `hoverProvider` capability to LSP
95. [x] DX-TASKS:  Implement `hover` method:
429:  - [x] Parse PHPX to AST
430:  - [x] Find symbol at cursor position
431:  - [x] Look up type information
432:  - [x] Format hover contents (markdown)
96. [x] DX-TASKS:  Show hover info for:
434:  - [x] Variables (show inferred type)
435:  - [x] Functions (show signature)
436:  - [x] Imports (show module path)
437:  - [x] Struct fields (show type)
438:  - [x] WASM imports (show WIT signature from `.d.phpx`)
97. [ ] DX-TASKS:  Test with various PHPX constructs
98. [x] DX-TASKS:  Add `completionProvider` capability to LSP
99. [x] DX-TASKS:  Implement `completion` method:
448:  - [x] Parse PHPX to AST
449:  - [x] Determine completion context (import, variable, etc.)
450:  - [x] Generate completion items
100. [x] DX-TASKS:  Provide completions for:
452:  - [x] Import paths (scan `php_modules/`)
453:  - [x] WASM modules (scan `php_modules/@*/`)
454:  - [x] Exported functions from imports
455:  - [x] Struct fields
456:  - [x] Built-in types (`Option`, `Result`, `Object`)
457:  - [x] PHPX stdlib functions
101. [x] DX-TASKS:  Add snippets for common patterns
102. [ ] DX-TASKS:  Test in Zed
103. [x] DX-TASKS:  Add `definitionProvider` capability to LSP
104. [x] DX-TASKS:  Implement `goto_definition` method:
468:  - [x] Find symbol at cursor
469:  - [x] Resolve import paths
470:  - [x] Find definition location
471:  - [x] Return LSP `Location`
105. [x] DX-TASKS:  Support go-to-definition for:
473:  - [x] Imported functions
474:  - [x] Local variables
475:  - [x] Struct definitions
476:  - [x] WASM imports (jump to `.d.phpx` stub)
106. [x] DX-TASKS:  Test with multi-file projects
107. [x] DX-TASKS:  Extend compiler API to load `.d.phpx` stubs
108. [x] DX-TASKS:  Parse stub files for type information
109. [x] DX-TASKS:  Use stub types for:
487:  - [x] Hover info on WASM imports
488:  - [x] Autocomplete for WASM functions
489:  - [x] Type checking WASM function calls
490:  - [x] Go-to-definition (jump to stub)
110. [x] DX-TASKS:  Suggest generating stubs if missing
111. [x] DX-TASKS:  Test with WIT examples from `examples/wasm_hello_wit/`
112. [x] DX-TASKS:  Add `referencesProvider` capability to LSP
113. [x] DX-TASKS:  Implement `references` method:
501:  - [x] Find all uses of symbol
502:  - [x] Search across all files in workspace
503:  - [x] Return LSP `Location` list
114. [x] DX-TASKS:  Support find-references for:
505:  - [x] Functions
506:  - [x] Variables
507:  - [x] Imports
508:  - [x] Struct types
115. [x] DX-TASKS:  Test with multi-file projects
116. [x] DX-TASKS:  Add `renameProvider` capability to LSP
117. [x] DX-TASKS:  Implement `rename` method:
518:  - [x] Find all references to symbol
519:  - [x] Generate `TextEdit` for each reference
520:  - [x] Return `WorkspaceEdit`
118. [x] DX-TASKS:  Support renaming:
522:  - [x] Variables
523:  - [x] Functions
524:  - [x] Imports (update import path)
525:  - [x] Struct fields
119. [x] DX-TASKS:  Test rename across multiple files
120. [x] DX-TASKS:  Verify no broken references
121. [x] DX-TASKS:  Add `documentSymbolProvider` capability to LSP
122. [x] DX-TASKS:  Implement `document_symbol` method:
536:  - [x] Parse PHPX to AST
537:  - [x] Extract functions, structs, enums, type aliases, constants
538:  - [x] Return LSP `DocumentSymbol` hierarchy
123. [ ] DX-TASKS:  Show symbols in editor outline/breadcrumbs
124. [ ] DX-TASKS:  Test with large PHPX files
125. [x] DX-TASKS:  Create `extensions/vscode-phpx/` directory
126. [x] DX-TASKS:  Initialize extension: `npm init` or `yo code`
127. [x] DX-TASKS:  Update `package.json` metadata
128. [x] DX-TASKS:  Create `syntaxes/phpx.tmLanguage.json` (TextMate grammar)
557:  - [ ] Port from tree-sitter grammar OR
558:  - [ ] Use tree-sitter WASM in extension
129. [x] DX-TASKS:  Add language configuration
130. [x] DX-TASKS:  Add file icon
131. [x] DX-TASKS:  Add `vscode-languageclient` dependency
132. [x] DX-TASKS:  Create `src/extension.ts`:
569:  - [x] Start LSP server on activation
570:  - [x] Configure server options
571:  - [x] Handle server lifecycle
133. [x] DX-TASKS:  Bundle LSP binary with extension OR
134. [ ] DX-TASKS:  Download binary on activation (GitHub releases)
135. [ ] DX-TASKS:  Test extension locally: `code --extensionDevelopmentPath=.`
136. [x] DX-TASKS:  Option A: TextMate grammar in `syntaxes/`
137. [ ] DX-TASKS:  Option B: tree-sitter WASM bundle
583:  - [ ] Compile tree-sitter grammar to WASM
584:  - [ ] Bundle in extension
585:  - [ ] Use `vscode-textmate` or `web-tree-sitter`
138. [ ] DX-TASKS:  Test highlighting with PHPX files
139. [ ] DX-TASKS:  Verify matches Zed highlighting
140. [x] DX-TASKS:  Create `.vsix` package: `vsce package`
141. [x] DX-TASKS:  Test installation: `code --install-extension phpx-0.1.0.vsix`
142. [ ] DX-TASKS:  Create GitHub repository for extension
143. [x] DX-TASKS:  Write `README.md` with features and screenshots
144. [ ] DX-TASKS:  Publish to VSCode Marketplace (optional):
599:  - [ ] Create publisher account
600:  - [ ] Run `vsce publish`
145. [x] DX-TASKS:  Add to Deka documentation
146. [x] DX-TASKS:  Create Neovim plugin structure: `nvim-phpx/`
147. [ ] DX-TASKS:  Add tree-sitter grammar to nvim-treesitter:
616:  - [ ] Fork `nvim-treesitter`
617:  - [x] Add parser config for PHPX
618:  - [ ] Submit PR to nvim-treesitter
148. [x] DX-TASKS:  Document installation (Lazy.nvim, Packer, etc.)
149. [ ] DX-TASKS:  Test in Neovim
150. [x] DX-TASKS:  Document LSP setup with `nvim-lspconfig`:
151. [x] DX-TASKS:  Add autocommand for `.phpx` files
152. [ ] DX-TASKS:  Test LSP features in Neovim
153. [x] DX-TASKS:  Document keybindings
154. [x] DX-TASKS:  Create LuaSnip snippets for PHPX
155. [x] DX-TASKS:  Add common patterns:
646:  - [x] Function definition
647:  - [x] Struct literal
648:  - [x] Import statement
649:  - [x] JSX component
650:  - [x] Frontmatter template
156. [x] DX-TASKS:  Document snippet usage
157. [x] DX-TASKS:  Create `docs/phpx/editor-support.md`:
665:  - [x] Overview of tree-sitter and LSP
666:  - [x] Installation for each editor (Zed, VSCode, Neovim, Helix)
667:  - [x] Feature comparison matrix
668:  - [x] Troubleshooting guide
669:  - [x] Known limitations
158. [ ] DX-TASKS:  Update `CLAUDE.md` with editor support section
159. [ ] DX-TASKS:  Add screenshots/GIFs to documentation
160. [x] DX-TASKS:  Create `scripts/install-phpx-lsp.sh`:
679:  - [x] Build LSP binary
680:  - [x] Install to `~/.local/bin/` or system path
681:  - [x] Set up editor configs
161. [x] DX-TASKS:  Create `scripts/install-zed-extension.sh`
162. [x] DX-TASKS:  Create `scripts/install-vscode-extension.sh`
163. [ ] DX-TASKS:  Test on clean systems (Linux, macOS)
164. [x] DX-TASKS:  Add GitHub Actions workflow:
692:  - [x] Build tree-sitter grammar
693:  - [x] Build LSP server
694:  - [x] Run tests
695:  - [x] Create releases with binaries
165. [x] DX-TASKS:  Build for multiple platforms:
697:  - [x] Linux (x86_64)
698:  - [x] macOS (x86_64, arm64)
699:  - [x] Windows (x86_64)
166. [ ] DX-TASKS:  Publish VSCode extension to marketplace (automated)
167. [ ] DX-TASKS:  Write blog post/announcement:
708:  - [ ] Why PHPX needs editor support
709:  - [ ] What's included (tree-sitter, LSP)
710:  - [ ] How to install
711:  - [ ] Demo screenshots/GIFs
168. [ ] DX-TASKS:  Create video tutorial (optional)
169. [ ] DX-TASKS:  Post to appropriate channels
170. [ ] DX-TASKS:  Update Deka website
171. [x] DX-TASKS:  `examples/strlen.phpx` - Simple type annotations
172. [x] DX-TASKS:  `examples/php/modules-import/index.php` - Import/export
173. [x] DX-TASKS:  `examples/bridge_array.phpx` - Struct literals
174. [x] DX-TASKS:  `examples/phpx-components/app.phpx` - JSX + frontmatter
175. [x] DX-TASKS:  `examples/wasm_hello_wit/` - WASM imports with WIT stubs
176. [x] DX-TASKS:  Create edge case files:
730:  - [x] Nested JSX with PHPX expressions
731:  - [x] Complex type annotations
732:  - [x] Syntax errors
733:  - [x] Type errors
734:  - [x] Missing imports
177. [x] DX-TASKS:  **Prerequisite**: PHPX Validation System (See PHPX-VALIDATION.md)
801:  - [x] Foundation (syntax, imports, PHPX rules)
802:  - [x] Type system (type checking, generics)
803:  - [x] Structs, JSX, modules, patterns
178. [x] DX-TASKS:  Phase 1: Tree-sitter Grammar (core delivered)
179. [x] DX-TASKS:  Phase 2: LSP Server (core delivered)
180. [x] DX-TASKS:  Phase 3: Advanced Tree-sitter (core delivered)
181. [x] DX-TASKS:  Phase 4: LSP Intelligence (core delivered)
182. [ ] DX-TASKS:  Phase 5: VSCode Extension (in progress)
183. [ ] DX-TASKS:  Phase 6: Neovim Support (in progress)
184. [ ] DX-TASKS:  Phase 7: Documentation (in progress)
