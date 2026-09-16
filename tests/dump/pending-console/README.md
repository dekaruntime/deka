# Pending `console` conformance fixtures (rfd#44)

These are **not** wired into the dump gate. They are the corpus rows
`dekaruntime/testsuite` should get once dsc ships the typed `console`
global + `Printable` type (rfd#44, "Globals" -> `console` addition,
2026-09-16) — parallel to this PR, not part of it.

## Why they live here instead of in the corpus

`tests/dump`'s dual-host ("snippet") category reads fixtures from a pinned
`dekaruntime/testsuite` checkout (`deka self fetch testsuite`), fetched at
dump time — that repo, not this one, owns the corpus. Every fixture below
uses `console.log`, which no shipped dsc version can compile yet (`console`
does not exist as a DekaScript identifier until dsc's parallel PR lands).
Landing them in `testsuite/corpus` today would fail every dsc version's
Hats run immediately — the opposite of "do not break the gate." So they
sit here, ready to copy over, until that PR ships.

## One row per `Printable` category

Each directory is one fixture: `<name>.pending.ds` (source — drop the
`.pending` and rename to `<name>.pass.ds` once it compiles),
`<name>.json` (title/stage/notes, matching the corpus's existing
metadata shape), and `<name>.stdout` (expected **native** stdout, produced
by `crates/pool/src/wintertc.js`'s formatter and pinned by
`crates/pool/tests/console_output.rs::console_log_prints_structured_values_not_object_object`
in this PR). There is no `tuple` runtime representation in DekaScript today
(no dedicated syntax; see docs/dekascript — arrays are the closest
runtime shape), so that fixture is a mixed-type array literal, noted in
its `.json`.

Once dsc ships `console`:
1. Confirm each `.pending.ds` compiles (dsc's `Printable` type must accept
   every argument shown).
2. Rename to `.pass.ds`, move the eight directories into
   `dekaruntime/testsuite`'s `corpus/` (a new `console` category, or spread
   across existing categories — testsuite's call), and drop this README.
3. Add the browser-host expectation once `@dekaruntime/web-ide-kit` ports
   the same formatter (tracked separately — see this PR's body for the
   web-ide-kit issue).

## Formatter contract these fixtures pin

- Top-level string arguments print raw; every other value — including
  nested strings — goes through the structured printer.
- `Option`/`Result`/user `enum` print as a bare case label
  (`Some(1)`, `None`, `Ok(1)`, `Err("e")`, `Circle(5)`) — no enum-name
  prefix.
- Structs print as `Name { field: value, ... }`, using the struct's own
  enumerable fields (`__deka_struct` is a hidden tag, not a field).
- Arrays print as `[ item, item ]`; nested values recurse through the same
  rules.
