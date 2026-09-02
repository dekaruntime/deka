# Testing

How conformance is measured, and the rules that keep two people from getting
two different numbers for the same corpus.

> This file is a deliberate exception to the "only `CLAUDE.md` at repo root"
> rule. It is here because the rules below have to be readable by anyone who
> reports a number, and an issue is not where you look before running a suite.
> Do not delete it as stray documentation.

## The one rule

**One runner produces every number.** `tests/testsuite/run.mjs` is the single
source of verdicts for both hosts. Nothing else computes pass/fail. The dump
pipeline *publishes* the runner's output; it does not decide anything.

Everything below follows from that.

## Vocabulary

Every `(fixture, host)` pair lands in exactly one bucket:

| bucket | meaning |
|---|---|
| `passed` | ran, matched its expectation |
| `failed` | ran, did not match — **breaks the build** |
| `known` | ran, did not match, and is listed in `expected-failures.txt` |
| `skipped` | not run, for a reason that is a fact about the fixture |

The runner asserts `passed + failed + known + skipped == total` and exits
non-zero if it does not hold. A fixture that falls between the cases is owned
by nothing, which is the failure this whole file exists to prevent.

## Skips must be facts about the fixture, never about tooling

A legal skip reason describes the fixture: its `hosts` exclude this host, or it
pins a different compiler. 

**A skip may never name another harness.** `"exercised by the dump, not the
language gate"` was a legal-looking skip that hid **125 fixtures — 16% of the
corpus — from every gate.** The dump did run them, failed 46, and exited `0`.
Each side assumed the other held the line. If you find yourself writing a skip
reason containing the name of another tool, you are creating that hole again.

## `expected-failures.txt` is a ratchet, not a suppression list

The runner enforces both directions:

- a listed fixture that fails → `known`, does not break the build
- a listed fixture that **passes** → **hard error**, delete the line
- an unlisted fixture that fails → `failed`

So the list can only shrink. A stale entry cannot mask a fresh regression.
Every line must trace to an issue. **Never add a line to make a build green.**

## Numbers carry provenance, or they are not numbers

Every published figure carries the commit SHA, compiler version, host, fixture
count and timestamp.

**The testsuite site must never show a previous run's metrics.** The numbers and
the version on screen are one unit: whatever version is displayed, the metrics
shown are that version's. Never render a figure computed at one commit beside a
version string from another. If the pack is behind `main`, the site says so.

## What proves what

Hard-won, each from a real wrong diagnosis:

- **A green unit test on a cross-boundary feature means nothing.** Test the real
  topology — separate processes, the real client, the real VM.
- **`deka check` does not build the module graph.** Imported symbols resolve to
  `<infer>`, so `check` will report errors that `run` does not, and miss errors
  that `run` catches. Cross-module claims must be verified with `deka run`.
- **Asserting that emitted JS contains a call proves nothing about types.**
  `Infer` is assignable to everything, so the call is emitted whether the type
  was preserved or lost. Type fixes need a *negative* case: wrong use must error.
- **A passing tool run is not evidence the tool did the work.** `cli fmt <dir>`
  silently skipped every `.dsx`; `--as-package` reported `[ok]` on a deliberately
  broken package. Before trusting a gate, break something and confirm it notices.
- **`scripts/typeck-published-stdlib.sh` probes one function per package.**
  "All packages typecheck" means one call each typechecked. It is a smoke test,
  not coverage.
- **Read the bytes back.** A tag is not a release. Verify a published artifact by
  downloading it and reading its own version, never by trusting the tag or a
  `deploy: success` line.

## Running things

```sh
cargo build -p cli                                  # or --release
DEKA_NATIVE=target/debug/cli bun tests/testsuite/run.mjs
DEKA_NATIVE=target/debug/cli bun tests/testsuite/run.mjs --json   # machine-readable
scripts/typeck-published-stdlib.sh                  # smoke, one fn per package
```

A healthy native run is `failed: 0` with `known` equal to the line count of
`expected-failures.txt`. Any other combination needs explaining before merge.

## When a fixture fails, classify before fixing

Failures are not evenly distributed across causes, and guessing wastes a day.
Ask in this order:

1. **Is the test wrong?** Most recently: 31 of 46 failures were fixtures passing
   a `number` or `boolean` to `echo`, which takes a `string`, and 12 more were
   `await`ing a synchronous function. The compiler was right in every case.
2. **Is the expectation stale?** A `.fail` fixture pinned to diagnostic wording
   that has since improved.
3. **Is it the stdlib's types?** Unannotated exports surface as
   `Result<<infer>, <infer>>` at the call site.
4. **Is it the runtime?** Only after the above are excluded.

Column numbers matter: `3:6` in `echo(f(x))` points inside `echo`, not `f`.
Attributing an error to the wrong call produces a confident wrong diagnosis.
