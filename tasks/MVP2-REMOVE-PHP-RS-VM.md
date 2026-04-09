# MVP2 Remove php-rs VM

## Goal
Remove VM/runtime fallback code from `php-rs` and keep only parser/typecheck surfaces used by MVP2 JS lowering.

## Scope
- Keep:
  - `php_rs::parser::*`
  - `php_rs::phpx::typeck::*`
- Remove:
  - VM execution layer
  - Core runtime extension registration
  - Legacy wasm export/runtime paths

## Tasks
- [ ] Narrow `php-rs` public API to parser + phpx only.
- [ ] Delete unused `php-rs` source trees (`vm`, `runtime`, `builtins`, `compiler`, `sapi`, wasm exports) once references are removed.
- [ ] Remove remaining VM/fallback wiring in non-`php-rs` crates.
- [ ] Rebuild `cli` release and run PHPX conformance suite.
- [ ] Capture LOC before/after and record delta.

## Notes
- Hard requirement: scaffold output must fail fast (no VM fallback execution).
- We accept temporary breakage during removal while references are being cut.
