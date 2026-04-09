# Wasm development log

## 2026-01-27
- WIT runtime v1: added `enum` + `flags` support in the wasm bridge (lower/lift, layout, validation).
- Enum/flags marshalling: enums now round-trip as `snake_case` strings; flags as `array<string>` (also accepts integer bitmasks). Flags limited to 32 entries in v1.
- Stub generator: WIT `flags` now emit `array<string>` in `.d.phpx`.
- Docs: updated `docs/php/php-wasm-wit.md` with enum/flags runtime mapping + support list.
- Added `@user/enum_flags` wasm module and manual smoke test for enum/flags.
- Fixed WIT syntax in `php_modules/@user/hello/module.wit` (missing semicolons).
