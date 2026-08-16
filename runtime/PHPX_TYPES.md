# PHPX Types and Interop

This document describes how phpx types map to runtime PHP values and how they
interact with the Rust/WASM bridge.

## Design goals
- phpx types are compile-time only (TypeScript-style).
- phpx compiles to valid PHP for execution in the runtime.
- Runtime values stay compatible with normal PHP userland.
- PHPX code avoids `null` in favor of `Option<T>` (nullable types are banned).
- PHPX avoids PHP’s OOP model: classes/traits/inheritance are not supported.

## Current compiler behavior
- Type annotations are stripped from phpx function signatures before execution.
- This applies to parameter types and return types.
- Inline variable annotations are stripped:
  - `int $count = 1;` → `$count = 1;`
  - `$count: int = 1;` → `$count = 1;`
- Typed properties are stripped:
  - `public int $count;` → `public $count;`
- `type` aliases are compile-time only and stripped from output.
- Generic parameters in phpx (`<T>`) are compile-time only.
- PHPX disallows:
  - `class`/`trait` declarations.
  - `struct` inheritance (`extends`) and `implements`.
  - `interface` inheritance (`extends`).
  - anonymous classes.
  - `new` (use struct literals instead).
  - `namespace` and top-level `use` (use `import` instead).
  - `use` aliases inside structs (composition only).
  - `throw` and `try/catch` (use `Result`/`Option` instead).
- Use `panic($message)` to abort (implemented in `php_modules/deka.php`).

## Runtime mapping (PHP)
- `int`, `float`, `string`, `bool`, `array`, `object`, `callable`, `mixed`,
  `null` map directly to PHP equivalents.
- `byte` is a phpx-only alias for an integer in the range 0..255.
- `bytes` is a primitive type for raw binary data. In the JS runtime it is
  represented as a `Uint8Array`; in PHP-runtime contexts it is a binary string.
  The `bytes` type is distinct from `string`: strings are UTF-8 text, while
  `bytes` holds arbitrary 0..255 values.
- `Option<T>` is a builtin enum:
  - `Option::Some(<T>)` and `Option::None`.
  - `Option<T>` values are enum instances, not `null`.

## Bytes operations
The `bytes` primitive is supported by the `core/bytes` module, which is also
re-exported from the `bytes` stdlib package:

- `bytes_from_string(string $s): bytes` — encode a UTF-8 string into bytes.
- `bytes_to_string(bytes $b): string` — decode bytes as UTF-8.
- `bytes_len(bytes $b): int` — number of bytes in the buffer.
- `bytes_get(bytes $b, int $i): int|null` — byte at index, or null if out of bounds.
- `bytes_set(bytes $b, int $i, int $byte): bytes` — copy of the buffer with the
  byte at index replaced.
- `bytes_slice(bytes $b, int $start, ?int $length = null): bytes` — slice of the
  buffer.
- `bytes_concat(bytes $a, bytes $b): bytes` — concatenate two buffers.
- `bytes_to_array(bytes $b): array<int>` — copy bytes into an int array.
- `bytes_from_array(array<int> $a): bytes` — create a buffer from an int array.
- `Result<T, E>` is a builtin enum:
  - `Result::Ok(<T>)` and `Result::Err(<E>)`.
- `null` literals and nullable type syntax (`?T`, `T|null`) are rejected in PHPX.
  Use `Option<T>` with `Option::None`.
- Null comparisons (`=== null`, `!= null`, etc.) are rejected in PHPX. Prefer
  `isset()` to check optional values.
- `Option`/`Result` helpers:
  - `unwrap()` calls `panic()` on `None`/`Err`.
  - `expect($message)` calls `panic()` with a custom message.
  - `unwrap_or($fallback)` and `unwrap_or_else($fn)` return fallback values.
- `Object<{...}>` is a phpx-only structural type that maps to the runtime
  `Object` value (object literal). Field names are checked by the typechecker.
  - Optional fields use `?`: `Object<{ foo?: int }>` allows missing `foo`.
  - Object literals are checked exactly against `Object<{...}>` (missing required
    or extra fields are type errors).
  - Dot access on optional fields yields `T|null` during inference.
- `type Name = ...;` creates a named alias for any type expression (including
  object-shape sugar `type Name = { ... };`). Aliases are compile-time only.
- Generic aliases and functions:
  - `type Box<T> = { value: T };`
  - `function unwrap<T>(Box<T> $b): T { ... }`
  - Constraints use Go-style syntax: `function f<T: Reader>(T $x) { ... }`
- Interfaces are structural in PHPX (Go/TS style): a `struct` satisfies an
  interface if it provides all required methods with compatible signatures.
- Struct composition (Go-style embedding):
  - Use `use` inside a `struct` body: `struct B { use A; }`.
  - The embedded struct is added as a field (by name) and its fields are
    promoted for dot access.
  - Ambiguous promoted fields are a type error.
- Struct literals (Rust-style):
  - Define fields as `$name: Type` with optional defaults (`= expr`).
  - `__construct` is not allowed in PHPX structs (use literals instead).
  - Construct values with `Point { $x: 1, $y: 2 }` (no `new`).
  - Shorthand field init is allowed: `Point { $x, $y }`.
  - Defaults must be constant expressions (literals/arrays/consts).
- Enums (phpx extension):
  - Payload cases are allowed: `enum Msg { case Text(string $body); }`.
  - `Enum::Case(...)` constructs a new enum value with payload fields.
  - `Enum::Case` (no call) is the case descriptor (name/value only).
  - `UnitEnum::cases()` returns these case descriptors (no payload fields).
- Enum values expose `name` (string) and, for backed enums, `value`.
  - Dot access to payload fields is allowed only when every case provides
    that field.
  - Within a `match` arm that matches a specific enum case, the matched
    variable narrows to that case, so payload fields for that case are allowed.
  - Enum equality (`===`) is case-based: two enum values compare equal when
    their case names match (payload is ignored). This keeps `match` usable.
  - `match ($e)` is exhaustively checked when the condition type is an enum
    (or enum|null) and there is no `default` arm.

## Rust/WASM bridge impact
- All values crossing the boundary are still PHP values.
- phpx types are not enforced across the boundary; they exist for tooling.
- `byte` should be treated as a u8 in host ops, but is passed as a PHP int.
- If an op needs strict validation, perform it in the op itself or in a phpx
  wrapper before calling the op.

## PHP interop
- phpx compiles into plain PHP functions.
- Userland PHP can call phpx exports normally (no special casing needed).
- The only magic is auto-including `php_modules/deka.php` before user code.
- Boundary conversions are lenient for legacy PHP:
  - `null` -> `Option::None` for `Option<T>` params.
  - arrays/stdClass -> Object/object-shape/struct params (extra keys ignored).
  - Struct params set provided fields; missing fields keep defaults (or remain
    uninitialized for required typed fields with no default). PHPX struct
    literals still require all non-default fields.
  - `Option<T>` return -> `null` or inner value.
  - `Result<T,E>` return -> `T` or `['ok' => false, 'error' => ...]`.
  - Preferred Result array shape:
    - `['ok' => true, 'value' => ...]` for Ok
    - `['ok' => false, 'error' => ...]` (or `err`) for Err
  - Legacy/lenient Result inputs still accepted:
    - `['value' => ...]` => Ok
    - `['error' => ...]` or `['err' => ...]` => Err
    - Non-boolean `ok` is treated as the Ok value when no `value`/`error` is provided

## TODO
- Add typed properties and inline type annotations to the stripper.
- Introduce a dedicated phpx type-checker (build-time).
- Consider a standard library for runtime validation helpers.
