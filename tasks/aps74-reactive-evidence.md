# APS 74: reactive core validation

This is the independent reactive part of phase 1, step 1. It is **not** the
completed counter/View PR. The shared-store blocker is recorded at
https://github.com/dekaruntime/aps/issues/74#issuecomment-6033016684.

Base: `origin/main` at `08511f1b76f19f8bda828c631254d59251af6180`.
Checkout: `/Volumes/Projects/codex/deka`.
Branch: `codex/deka-ui-core-aps74`.

All builds ran serially, in release mode, using the machine's sccache settings,
`TMPDIR=/Volumes/Projects/codex/.tmp-deka` and this checkout's `.target`.
No VM crate, original tour fixture, renderer, or release setting was changed.
The new crate is `deka-ui` 0.1.0, Apache-2.0, `publish = false`.

## Final validation

Full logs and explicit exit-code files are in [evidence/aps74-reactive](evidence/aps74-reactive/).

| Log prefix | Command | Exit/result |
| --- | --- | --- |
| core-check | `cargo check --release --locked --workspace --all-targets` | 0 |
| core-clippy | `cargo clippy --release --locked --workspace --all-targets --all-features` | 0; existing warnings outside the new crate |
| core-clippy-strict | `cargo clippy --release --locked -p deka-ui --all-targets --all-features -- -D warnings` | 0; no warnings |
| core-test | `cargo test --release --locked -p deka-ui` | 0; 13 integration tests |
| core-example | `cargo run --release --locked -p deka-ui --example reactive_counter` | 0; docs output matched exactly |
| core-fmt | `cargo fmt -p deka-ui --check` | 0 |

## Literal behavior reverts

Each `.patch` was applied to the actual source, the named integration test ran,
and the original source was restored before the next proof. Each failed on a
behavior assertion with exit **101**, not a compilation error. Final restored
check, clippy and tests passed. There are no permanent failure switches.

| Revert log/patch prefix | Disabled behavior | Assertion that failed |
| --- | --- | --- |
| revert-notifications | Queueing subscribers on a write | output stayed `[1]` instead of `[1, 2, 20, 21]` |
| revert-read-tracking | Registering reads in `get()` (including `Display`) | formatted effect stayed `0:false` after event writes |
| revert-batching | Holding writes until the event batch ends | intermediate formatted outputs appeared |
| revert-old-dependencies | Removing subscriptions from the previous branch | changing the now-unobserved left signal reran the effect |
| revert-derived-priority | Settling derived reactions before effects | an early-registered effect observed a stale later-derived value |
| revert-derived-equality | Suppressing unchanged derived output | parity effect ran twice instead of once |

The committed proof script is the exact script used here. It is host-specific:
run it in this checkout, with no simultaneous Rust build. It restores source in
a `finally` block. Full local logs, including the preservation checks on the
paused HTTP branch, are also at `/Volumes/Projects/codex/deka-ui-logs`.

## Macro parser investigation (step 2 remains unimplemented)

Recommend `rstml` for `view!`. Its [primary documentation](https://docs.rs/rstml/0.13.1/rstml/)
uses `proc_macro`/`proc_macro2` TokenStreams, syn expressions, node spans and
span-attached diagnostics. It supports quoted text, braced expressions and
colon-separated names such as `class:name`.

`deka_syntax::parse::parse` instead accepts `&str`; its `ast::Span` holds line,
column and byte offsets into that string. Stringifying a proc-macro TokenStream
would lose Rust's original token spans, and its expression grammar is DekaScript,
not Rust. Keep vocabulary/style catalogs shared, while using rstml for markup
parsing and retaining its token spans through generated prop/function calls.

This is an API/source review, not a completed parser experiment or compile-fail
suite. Unknown-component/missing-prop/wrong-type diagnostics still require the
requested trybuild tests in step 2.

-codex
