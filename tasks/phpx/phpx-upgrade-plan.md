# PHPX Upgrade Plan

## Goals
- Keep `.php` behavior unchanged unless it explicitly opts into phpx modules.
- Add JS-style object literals and tight dot access to `.phpx`.
- Add `struct` with value semantics and COW behavior (no classes/traits/OOP).
- Enforce strict typing in `.phpx` with hybrid nominal/structural types.
- Replace exceptions in `.phpx` with `Option`/`Result` + `panic()` for aborts.
- Ban `null` literals and nullable types in PHPX (use `Option<T>`).
- Make phpx modules non-global with explicit exports only (module isolation).
- Ensure unused imports are rejected and boundaries are explicit.
- Add JSX + a component core runtime (VNode + Context) and a DOM renderer.
- Add Astro-style frontmatter templates for PHPX (component-friendly).
- Provide explicit hydration via `<Hydration />` (static by default).

## Decisions (Locked)
- Parser/lexer stays unified; add a phpx mode instead of forking.
- JS-style object literals create a new runtime Object value.
- Tight dot access: `.` is member access only when no whitespace and RHS is an identifier.
- Dot access works for Object literals + structs, not for classes.
- Object literal keys: identifier + quoted string only (no computed keys yet).
- Hybrid typing: structs are nominal, `Object<...>` is structural.
- PHPX disallows classes/traits/inheritance (`class`, `trait`, `extends`, `implements`).
- Safe widening allowed (e.g., int -> float).
- Type errors block module load at runtime compile time.
- Structs are stored as `Val::Struct(Rc<ObjectData>)` with COW value semantics.
- Structs reuse class metadata but are not backed by `Val::Object(ObjPayload)`.
- PHPX modules are compiled into per-module namespaces for isolation.
- PHPX exports are registered in a global registry for PHP-side bridging (`phpx_import`).
- `.php` files opt into module semantics only via top-of-file `import`.
- Frontmatter template mode is opt-in via `---` delimiter in `.phpx`.
- `<Hydration />` is optional; omit it for fully static HTML.

## Phased Plan
### Phase 0: Baseline
- Confirm phpx mode gating and boundary conversion rules.
- Confirm module isolation behavior (explicit exports only).

### Phase 1: Parser/Lexer (phpx mode)
- Add `ParserMode::Phpx` and a lexer flag for phpx-only keywords.
- Parse `struct` declarations (phpx only).
- Add AST nodes for object literals and dot access.

### Phase 2: Runtime Object Value
- Add a new `Val` variant for Object literals with COW storage.
- Implement fetch/assign/isset/unset for the new Object value.
- Dispatch dot access for Object + struct values.

### Phase 3: Struct Value Semantics
- Add `is_struct` metadata to class definitions.
- Implement COW for structs on property writes and assignments.
- Update strict equality (`===`) to compare struct values.

### Phase 4: Strict Typing (phpx)
- Add a type checker with hybrid nominal/structural types.
- Add inference for literals, object literals, and struct constructors.
- Enforce safe widening only (no other implicit coercions).

### Phase 4.5: Type System Enhancements (phpx)
- Go-style structural interfaces (method-shape based).
- Rust-style enums (sum types with payloads).
- Exhaustive `match` checking for enums/unions.
- Generics with Go-style constraint syntax (`T: Reader`), `array<T>` canonical.
- Struct composition/embedding syntax (Go-style).
- Option/Result enums + `unwrap` helpers (panic-only via `panic()`).

### Phase 4.75: Rust-Style Struct Syntax (phpx)
- Replace class-like struct declarations with Rust-style field syntax.
- Field syntax: `$name: Type` with optional default `= expr`.
- Allow methods + `use` inside struct bodies; no visibility keywords required.
- Add struct literal expressions: `Point { $x: 1, $y: 2 }`.
- Shorthand field init: `Point { $x, $y }`.
- `new` is a hard error in PHPX (structs are constructed via literals).

### Phase 5: Module Isolation (phpx) ✅
- Replace global concatenation with a module registry.
- Only explicit exports are visible.
- Lazy evaluate modules on import.
- Unused imports raise errors at runtime compile time.
- Entry code executes in `namespace __phpx_entry` to avoid leaking globals.

### Phase 6: PHP <-> PHPX Bridge ✅
- Emit raw impls + wrapper exports.
- Wrapper does boundary conversions based on phpx types (lenient for legacy PHP).
- phpx-to-phpx calls use raw impls (no overhead).

### Phase 7: Tests + Docs
- Add tests for object literals, dot access, struct COW + equality,
  type errors, and module isolation behavior.
- Document new phpx syntax and typing rules.

### Phase 8: JSX + Component Core ✅
- JSX parsing (PHPX only) + AST nodes.
- Lower JSX to `jsx/jsxs` and a special fragment tag.
- `component/core` module:
  - `VNode` + Context types
  - `jsx`, `jsxs`, `createElement`
  - `createContext`, `useContext`, `ContextProvider`
  - `isValidElement`, `childrenToArray`
- Basic JSX typechecking (attr + child expressions).
- Tests for JSX parsing + core runtime.

### Phase 9: component/dom (Replace Mode Default)
- HTML renderer (`renderToString`, `renderToStream`).
- `createRoot({ container })` (replace-only).
- `<Link to>` helper for client-side routing.
- Partial response JSON format + client JS runtime.
- Tests for partials + Link.
- `<Hydration />` helper for explicit client bootstrapping.
- Frontmatter template mode for `.phpx` entrypoints.
- Component-style frontmatter modules under `php_modules/`.

### Phase 10: Tooling (Editors)
- Zed + Tree-sitter syntax highlighting for `.phpx`.

## Current Status (2026-02-01)
- Phase 1: Parser/Lexer (phpx mode) ✅
  - ✅ `ParserMode::Phpx` + keyword gating
  - ✅ `struct` parsing
  - ✅ Object literals + tight-dot access
- Phase 2: Runtime Object Value ✅
  - ✅ New Object value with COW storage
  - ✅ Fetch/assign/isset/unset + dot access dispatch
- Phase 3: Struct Value Semantics ✅
  - ✅ `is_struct` metadata on class defs
  - ✅ COW on struct writes/assigns
  - ✅ `===` compares struct values
- Phase 4: Strict Typing (phpx) — initial pass ✅
  - ✅ Inference for literals/object literals/dot access
  - ✅ Return type + struct default checks
  - ✅ Safe widening only (int -> float)
  - ✅ Call-site argument checking (local functions)
  - ✅ Union/nullable inference for assignments/conditionals
  - ✅ `Object<{...}>` parsing + enforcement
  - ✅ Optional object shape fields (`foo?: int`) + exact object literal checks
  - ✅ Flow-sensitive null narrowing for `===`/`!==` and `isset(...)`
  - ✅ Type aliases (`type Name = ...;`) with object-shape sugar
  - ✅ Generic aliases + type params with Go-style constraints (initial)
  - ✅ Structural interface checking (Go-style)
  - ✅ Struct composition (`use` inside struct) + promoted dot access
  - ✅ PHPX disallows `namespace` and top-level `use` (use `import` instead)
  - ✅ Option/Result enums + `unwrap` helpers (panic-only); PHPX bans `throw`/`try`
  - ✅ Tests in `crates/php-rs/src/phpx/typeck/tests.rs`
- Phase 4.5: Type System Enhancements (phpx) ✅
  - ✅ Rust-style enums with payload cases + enum case calls
  - ✅ Exhaustive `match` checking for enums (and enum|null)
  - ✅ Generics + Go-style constraints (`T: Reader`), `array<T>` canonical
- Phase 4.75: Rust-Style Struct Syntax (phpx) ✅
  - ✅ Rust-style field syntax: `$name: Type` (optional `= expr` defaults)
  - ✅ Struct literal expressions + shorthand init
  - ✅ Methods + `use` remain inside struct bodies
  - ✅ `new` is a hard error in PHPX
- Phase 8: JSX + Component Core ✅
  - ✅ JSX parsing + lowering (phpx only)
  - ✅ `component/core` module (VNode + Context)
  - ✅ JSX typechecking (attr/child expressions)
  - ✅ Tests for JSX parsing + core runtime
- Phase 9: component/dom (Replace Mode Default) ✅
  - ✅ HTML renderer (`renderToString`, `renderToStream` stub)
  - ✅ `createRoot({ container })` (replace-only)
  - ✅ `<Link to>` helper + client JS runtime
  - ✅ Partial response JSON format
  - ✅ `<Hydration />` helper (explicit client boot)
  - ✅ Frontmatter template mode (Astro-style) for `.phpx` entrypoints
  - ✅ Component-style frontmatter modules under `php_modules/`
- Runtime parity (PHP) ✅
  - ✅ Namespace + `use` resolution in the compiler/emitter
  - ✅ Unqualified function/const fallback to global (PHP semantics)
  - ✅ `class_alias()` builtin + alias resolution (Option/Result bridge)
- Phase 5: Module Isolation (phpx) ✅
  - ✅ Replace global concatenation with a module registry
  - ✅ Only explicit exports are visible
  - ✅ Lazy evaluate modules on import
  - ✅ Unused imports raise errors at runtime compile time
  - ✅ Entry code runs in `namespace __phpx_entry` (global prelude stays global)
- Phase 6: PHP <-> PHPX Bridge ✅
  - ✅ Emit raw impls + wrapper exports
  - ✅ Boundary conversions based on phpx types (lenient):
    - `null` -> `Option::None` for `Option<T>` params
    - arrays/stdClass -> Object/object-shape/struct (extra keys ignored)
    - `Option<T>` return -> `null` or inner value
    - `Result<T,E>` return -> `T` or `['ok' => false, 'error' => ...]`
  - ✅ phpx-to-phpx calls use raw impls (no overhead)
- Phase 7: Tests + Docs — in progress (paused)
  - ✅ Add PHPX fixtures for object literals, structs, enums, Option/Result, module import
  - ✅ Document new phpx syntax and typing rules (`docs/phpx/phpx-dx.md`, `PHPX_TYPES.md`)
  - ⏳ Add module isolation + unused import runtime coverage
  - ✅ Add tests/docs for frontmatter templates + Hydration
- Phase 10: Tooling (Editors) ⏳
  - ⏳ Zed + Tree-sitter syntax highlighting for `.phpx`

## File-Level Breakdown
### Parser/Lexer
- `deka/crates/php-rs/src/parser/parser/mod.rs`
  - Add `ParserMode::Php|Phpx` and store mode on the parser.
- `deka/crates/php-rs/src/parser/lexer/mod.rs`
  - Add phpx keyword handling (struct), mode flag.
- `deka/crates/php-rs/src/parser/ast/mod.rs`
  - Add `Expr::ObjectLiteral`, `Expr::DotAccess`.
  - Add class kind or struct marker in the AST.
- `deka/crates/php-rs/src/parser/parser/definitions.rs`
  - Parse `struct` when in phpx mode.
- `deka/crates/php-rs/src/parser/parser/expr.rs`
  - Parse object literals in expression context (phpx).
  - Dot access with tight-dot check (no whitespace).

### Compiler/Emitter
- `deka/crates/php-rs/src/compiler/emitter.rs`
  - Emit struct metadata into `ClassDef`.
  - Emit Object literal construction + dot access ops.

### Runtime Types + VM
- `deka/crates/php-rs/src/core/value.rs`
  - Add `Val::ObjectMap` (or similar) with COW storage.
- `deka/crates/php-rs/src/runtime/context.rs`
  - Add `is_struct` on `ClassDef`.
- `deka/crates/php-rs/src/vm/engine.rs`
  - Handle ObjectMap in FetchProp/AssignProp/Unset/Isset paths.
  - Apply COW for struct property writes.
- `deka/crates/php-rs/src/vm/opcodes/comparison.rs`
  - `===` compares struct values instead of handles.

### Module Isolation + Bridge
- `deka/crates/modules_php/src/modules/deka_php/php.js`
  - Replace global concat with per-module registry and explicit exports.
  - Emit raw impls + wrapper exports.
  - Enforce unused import errors at compile time (runtime compile).

### Typing
- `deka/crates/php-rs/src/phpx/typeck/`
  - New type checker modules: `types.rs`, `infer.rs`, `check.rs`.
- Hook type checking into phpx compile path before emission.

### Tests
- Add tests in `deka/crates/php-rs/src/parser/tests/` for syntax.
- Add runtime tests for struct COW + equality in `deka/crates/php-rs/test-old/` or a new suite.
