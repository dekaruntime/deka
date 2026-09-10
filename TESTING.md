# Testing

How conformance is measured, and the rules that keep two people from getting
two different numbers for the same corpus.

> This file is a deliberate exception to the "only `CLAUDE.md` at repo root"
> rule. It is here because the rules below have to be readable by anyone who
> reports a number, and an issue is not where you look before running a suite.
> Do not delete it as stray documentation.

## The one rule

**One runner produces every number.** The pinned `dekaruntime/testsuite` corpus runner is the single
source of verdicts for both hosts. Nothing else computes pass/fail. The dump
pipeline *publishes* the runner's output; it does not decide anything.

Everything below follows from that.

## Three groups, decided by which hosts a fixture can run on

Report numbers in three groups, never as one blended total:

```
native-only     x pass · x fail
shared          x pass · x fail · x diverge
browser-only    x pass · x fail
```

A fixture's `hosts` decides its group. Membership is a property of the fixture,
not a runtime outcome.

**Divergence only exists in `shared`.** It is the one metric that catches
host-specific miscompilation, and it is meaningless for a fixture that only
ever runs on one host. Computing it across the whole corpus is how infrastructure
gaps get counted as compiler defects — at v0.38.2, 25 of 34 "divergences" were
the browser harness failing to serve a stdlib module (#509).

### Nothing skips. There is no skip bucket, in any group.

Every fixture in every group **runs**, and its result is `pass` or `fail`.
No third state, no escape hatch, no reason code.

"Skipped" used to do two unrelated jobs, and collapsing them is exactly how 125
fixtures stayed invisible (#503):

- *this fixture is not for this host* — that is **group membership**, decided by
  `hosts`, and it is not a skip. A native-only fixture is not "skipped on wasm";
  it was never a wasm test.
- *we declined to run it* — that is a **hole**, and it is not allowed.

If a fixture cannot run, that is a bug in the harness or the fixture, and it is
fixed — not recorded. A fixture pinned to a different compiler is stale and gets
updated or deleted. A module the harness cannot serve fails the **run**, loudly,
naming the module (#509) — it never degrades into a per-fixture non-result.

`known` is not a skip. A `known` fixture **runs and fails**; the ratchet only
decides whether that failure breaks the build. Nothing is exempt from executing.

Within a group, `pass + fail == group total`, asserted per group by the runner,
non-zero exit otherwise.

### Worked example — v0.38.2, commit 08e777b2

```
native-only     87 pass ·  46 fail              (133)
shared         599 pass ·   0 fail ·  34 diverge (633)
browser-only    28 pass ·   2 fail              ( 30)
```

Read in one pass: every failure is a native-only fixture, shared native is
100%, and every wasm problem is a divergence. The single blended line this
replaced — `638 passed | 0 failed | 155 skipped` — said none of that.

## Never write a skip reason

There is no legitimate one. The instinct to write a skip reason is the instinct
to stop testing something, and it always reads as housekeeping at the time.

`"index packages are exercised by the dump, not the language gate"` was a
legal-looking skip that hid **125 fixtures — 16% of the corpus — from every
gate.** The dump did run them, failed 46, and exited `0`. Each side assumed the
other held the line, and the published `638 passed | 0 failed` was a zero
measured over a population with every known failure removed from it.

The tell is a reason naming another tool. If you are writing one, you are
building that hole again. Make the fixture run, or delete it.

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
scripts/ci-fetch-testsuite-corpus.sh
DEKA_NATIVE=target/debug/cli bun .cache/testsuite-corpus/run.mjs
DEKA_NATIVE=target/debug/cli bun .cache/testsuite-corpus/run.mjs --json   # machine-readable
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
