---
title: "PHPX DX + Syntax Summary"
section: "phpx"
category: "reference"
categoryLabel: "Reference"
categoryOrder: 100
---

# PHPX DX + Syntax Summary

This is a concise, developer-facing summary of the PHPX experience and the
syntax additions we have introduced or locked in. It is meant to help future
contributors quickly understand the current behavior.

## File types + runtime behavior
- `.php`: Not supported in MVP2 (reserved for future or archived legacy usage).
- `.phpx`: PHPX mode is always on. Import/export is required for cross-module use.
- `.d.phpx`: Typed stub files (used for WASM/WIT stubs and IDE/typing only).

Runtime notes (MVP2 JS runtime):
- `.php` is not supported in MVP2; only `.phpx` executes.
- `deka run`/`deka serve` transpile PHPX to JS and execute it as ESM.
- The ESM loader resolves stdlib imports from `php_modules/`.
- Dev mode transpiles on demand and caches JS output under `.cache/phpx_js/`.
- Entry modules should either set a top-level `app` variable or export
  `default`/`app`/`handler`; the runtime maps that value to `globalThis.app`.

## JS target semantics (`deka build`)
- JS target runtime behavior is JavaScript-first by design.
- PHPX type/module/validation rules are enforced before emit.
- Emitted output should prefer idiomatic JS lowering over PHP emulation.
- Full contract: `docs/phpx/phpx-js-target-semantics.md`.
- `deka build` now also emits `importmap.json` beside the JS output by default.
- The import map includes stable short-path prefixes (`@/`, `component/`, `deka/`, `encoding/`, `db/`) plus fallback entries for other bare specifiers found in source imports.
- `deka build --bundle` emits a single-file JS bundle in-memory (no intermediate JS file).
- `--minify` enables SWC minification for bundled output.

## Module system (PHPX)
- `import { foo } from './bar.phpx'` and `export function foo()` are supported.
- Default imports use `import Foo from 'module'` and expect
  `export { Foo as default }` (or any identifier aliased to `default`).
- Exports are explicit; non-exported functions are private to the module.
- Unused imports are rejected at runtime compile time.
- `.php` files opt in by placing `import` at the very top of the file.

## Encoding namespace
- Canonical JSON module path is now `encoding/json`.
- Root `json` remains as a temporary compatibility proxy.
- New code should import from `encoding/json`.
- Compatibility removal target: first post-MVP breaking release (Phase 5 cleanup window).

## Foundation IO modules
- `bytes`: canonical byte-string helpers (`from_string`, `to_string`, `len`, `get`, `set`, `slice`, `concat`).
- `buffer`: cursor/framing helpers over `bytes` (`buffer_new`, `read`, `read_exact`, `write_u16_be`, `read_u16_be`).
- `tcp`: low-level socket operations (`connect`, `read`, `read_exact`, `write`, `set_deadline`, `close`).
- `tls`: upgrades a tcp handle (`upgrade`) and provides `read`/`write`/`close`.

## Auth primitives
- `crypto`: random ids/bytes, SHA-256/HMAC, secure compare.
- `jwt`: `sign`/`verify` with claim-time validation (`exp`, `nbf`, `iat`) and optional `iss`/`aud` checks.
- `cookies`: safe cookie read/build/header helpers.
- `auth`: shared OAuth helpers (`pkce_pair`, `new_state`, `new_nonce`, `sign_oauth_state`, `verify_oauth_state`).

For usage examples, see `docs/php/auth/index.mdx`.

## Database modules
- Canonical driver paths are `db/postgres`, `db/mysql`, and `db/sqlite`.
- Legacy top-level paths `postgres`, `mysql`, and `sqlite` remain as compatibility proxies.
- Shared contract helpers live in `db` (`open_handle`, `rows`, `query_one`, `affected_rows`).

## ORM annotations (PHPX structs)
- Canonical relation syntax is `@relation("hasMany|belongsTo|hasOne", "Model", "foreignKey")`.
- `@relation(...)` fields are virtual relation metadata and are not emitted as table columns.
- `@relation("hasMany", ...)` requires an `array<...>` field type.
- `@relation("belongsTo", ..., "foreignKey")` auto-generates an index for the foreign key in generated Postgres migrations.
- Non-relation `array<T>` fields map to `JSONB` in generated Postgres migrations.

## Semicolons (PHPX)
- Semicolons are optional in `.phpx` (JS-style automatic semicolon insertion).
- A line terminator ends a statement unless the expression clearly continues
  (inside `(...)`, `[...]`, `{...}`, or after `->`, `.`, `::`, or an operator).
- `return`, `break`, and `continue` only consume their expression/level if it
  appears on the **same line** (JS-style rule).

## Objects + dot access
PHPX introduces JS-style object literals and tight-dot access:

```php
$cfg = { host: 'localhost', port: 5432 };
$host = $cfg.host;
```

Rules:
- Dot access only when there is no whitespace and RHS is an identifier.
- Keys: identifiers or quoted strings only (no computed keys yet).
- Dot access works for object literals and structs (not classes).
- Object literals use value semantics for `==` and `===` (deep compare by key).
- JS target note: when compiling with `deka build`, emitted comparisons follow JS semantics (`===`/`!==`) rather than PHPX runtime deep-equality behavior.
- `get_class()` returns `stdClass` for object literals and the struct name for structs.
- `property_exists()` checks object-literal keys and struct fields (including promoted).
- `method_exists()` works for structs; object literals always return false.
- `count()` returns the number of keys/fields for object literals and structs.

## Structs (value semantics)
Structs are nominal value types with Rust-style syntax:

```php
struct Position {
    $x: int = 0;
    $y: int = 0;
}

$p1 = Position { $x: 1, $y: 2 };
$p2 = $p1;
$p2.x++;
```

Notes:
- Structs reuse class metadata but are not regular PHP objects.
- `===` compares struct values, not handles.
- Struct composition uses `use` inside the struct body.
- `__construct` is not allowed in PHPX structs (use literals instead).
- `new` is not allowed in PHPX; use struct literals (`Point { ... }`).
- Shorthand field init is allowed: `Point { $x, $y }`.
- Struct field defaults must be constant expressions.

## Types + inference (PHPX)
PHPX has a strict type checker with hybrid nominal/structural behavior.

Key rules:
- Safe widening only (e.g. int -> float). No other implicit coercions.
- `Object<{...}>` is structural; `struct` is nominal.
- `type Name = ...;` creates compile-time-only aliases.
- Generic aliases and type params use Go-style constraints: `T: Reader`.
- `array<T>` is the canonical generic array spelling.

## Option/Result + panic (no exceptions in PHPX)
PHPX replaces exceptions with enums and explicit error handling.

```php
enum Result {
    case Ok(mixed $value);
    case Err(mixed $error);
}

enum Option {
    case Some(mixed $value);
    case None;
}
```

- `throw` and `try/catch` are banned in PHPX.
- `panic($message)` aborts execution (provided by `php_modules/deka.php`).
- `Option::unwrap()` / `Result::unwrap()` call `panic(...)` on failure.
- `null` literals and nullable types (`?T`, `T|null`) are rejected in PHPX.

## Enums + match
- Enums support payload cases: `enum Msg { case Text(string $body); }`.
- `match` is exhaustively checked when possible.
- Enum case access supports payload field narrowing within match arms.

## JSX + components (PHPX)
- JSX is PHPX-only and lowers to `jsx/jsxs` calls from `component/core`.
- JSX auto-injects the runtime import; user code should not import `jsx/jsxs`.
- Unused-import checks ignore synthetic JSX/runtime imports.
- `<>...</>` lowers to a special fragment tag.
- JSX outputs VNode values (renderer lives in `component/dom`).
- `{ ... }` accepts any PHPX expression (no statements). Object literals use `{ { ... } }`.
- JSX text whitespace is normalized (indentation/newlines are trimmed).

## PHP interoperability
- PHP can call PHPX exports via `phpx_import` or bridged helpers.
- PHPX compiles to valid PHP, and types are stripped before execution.
- Runtime values remain standard PHP values at the boundary.
- Boundary coercions are lenient for legacy PHP:
  - `null` -> `Option::None` for `Option<T>` params.
  - Arrays/stdClass are accepted for `Object`/object-shape + `struct` params
    (extra keys are ignored).
  - `Option<T>` return -> `null` (None) or inner value (Some).
  - `Result<T,E>` return -> `T` (Ok) or `['ok' => false, 'error' => ...]` (Err).

## Not supported in PHPX (by design)
- `class`, `trait`, `extends`, `implements`
- `namespace` and top-level `use` (use `import` instead)
- `throw` and `try/catch`

## Related docs
- `PHPX_TYPES.md` for detailed type mapping and rules.
- `docs/php/php-modules.md` for module system details.
- `docs/phpx/phpx-upgrade-plan.md` for phased status.
- `docs/phpx/editor-support.md` for Zed/VSCode/Neovim/Helix setup.

## Supported features (summary)
- Module system (`import`/`export`), explicit exports, and unused-import checks.
- Value types: structs, object literals, and tight dot access.
- Strict typing with inference, generics, and Go-style constraints.
- `Option`/`Result` for error handling; `panic()` for hard failures.
- Enums + exhaustive `match`.
- JSX in `.phpx`, with VNode output for rendering.
- WASM imports via `@user/*` modules with `.d.phpx` stubs.

## Roadmap (short)
- Tree-sitter polish (error recovery, editor folding/indent verification).
- LSP intelligence: hover, completion, go-to-definition, references, rename.
- Editor integrations (VSCode, Neovim) and install scripts.
- WIT/component-model DX for typed WASM imports.

## Troubleshooting
- **Imports fail**: ensure the file is `.phpx` (or `.php` with top-level `import`)
  and the module exists under `php_modules/`.
- **LSP diagnostics missing**: rebuild `phpx_lsp` and update the editor's binary
  path; restart the editor after changes.
- **Local CLI mismatch**: rebuild with `cargo build --release -p cli` and run the
  wired `target/release/cli` binary.
