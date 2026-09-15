// The dump gate's pass/fail verdict for CI. Kept as a pure function so the
// exit-code decision can be unit tested without running the full conformance
// suite (deka#906).
//
// Three independent reasons can fail a run, and all three must be checked
// independently -- none of them substitutes for another:
//   1. The divergence ratchet: an unlisted host divergence, or a listed one
//      that now silently agrees (see dump-results.mjs, expected-failures.txt).
//   2. The failure ratchet: a fixture fails outright and is NOT in the known
//      failure baseline (tests/dump/expected-failures-list.txt) -- a new,
//      unbudgeted failure.
//   3. The failure ratchet, other direction: a fixture IS in the known
//      failure baseline but now passes -- a stale entry. Same "silently
//      agrees with debt that no longer exists" logic as staleDivergences.
//      Left unenforced, the baseline only ever grows.
//
// deka#906: summary.overall.fail > 0 used to be either ignored (the original
// bug) or, briefly, an absolute zero (PR #1040) -- which meant no release
// could ship until a large pre-existing backlog of failures was fully
// cleared. This ratchet is the middle ground: today's known failures don't
// block, but neither regressing to a new failure nor quietly "fixing" a
// listed one without updating the baseline can pass unnoticed.
export function computeGateExitCode(args: {
  unexpectedDivergences: number
  staleDivergences: number
  unexpectedFailures: number
  staleFailures: number
}): number {
  if (args.unexpectedDivergences > 0) return 1
  if (args.staleDivergences > 0) return 1
  if (args.unexpectedFailures > 0) return 1
  if (args.staleFailures > 0) return 1
  return 0
}
