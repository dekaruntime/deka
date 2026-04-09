# PHPX Destructuring Tasks

This file tracks the PHPX destructuring rollout selected for implementation:

1. Function parameter destructuring
2. Nested object destructuring
3. Defaults in destructuring
4. Array/tuple parameter destructuring
5. Loop destructuring (`foreach`)
6. Assignment object destructuring

## Scope

- PHPX-only behavior.
- Keep existing PHP behavior unchanged.
- Align syntax with TS-style destructuring where practical.

## Tasks

1. [x] Add parser plan/state for destructuring prologue lowering.
2. [x] Add PHPX parameter pattern parsing for object/array patterns.
3. [x] Support nested destructuring patterns in parameter parsing.
4. [x] Support default values in parameter destructuring patterns.
5. [x] Lower parameter destructuring into function/method/closure prologue assignments.
6. [x] Reject unsupported arrow-function param destructuring with actionable error.
7. [x] Enable foreach destructuring lowering in PHPX.
8. [x] Allow object-literal destructuring targets in assignment parsing.
9. [x] Extend emitter for deep nested object assignment destructuring.
10. [x] Extend typechecker/LSP hints for destructured binding names and default inference.
11. [x] Add runtime behavior tests under `tests/phpx/` for function/loop/assignment destructuring.
12. [x] Fix `cli run` PHPX runtime parity for destructuring fixtures.
Notes: rebuilt embedded `php_rs.wasm` for `modules_php` and added PHPX ASI guard before newline `(` to prevent accidental call-chain parsing across statements.

## Validation

1. [x] `cargo test -p php-rs parser::parser::tests --release`
2. [x] `cargo test -p php-rs phpx::typeck::tests --release`
3. [x] Add and run targeted destructuring regression tests.
4. [x] `target/release/cli run /tmp/phpx_suite/object_destructuring.phpx`
