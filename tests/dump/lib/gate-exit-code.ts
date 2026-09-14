// The dump gate's pass/fail verdict for CI. Kept as a pure function so the
// exit-code decision can be unit tested without running the full conformance
// suite (deka#906).
//
// Two independent reasons can fail a run, and both must be checked
// independently -- neither one substitutes for the other:
//   1. The divergence ratchet: an unlisted host divergence, or a listed one
//      that now silently agrees (see dump-results.mjs).
//   2. summary.overall.fail > 0: at least one fixture outright failed. This
//      is NOT the same signal as the ratchet -- a run can have a clean
//      ratchet (no *new* divergences) while every fixture fails outright,
//      and that must not exit 0.
export function computeGateExitCode(args: {
  unexpectedDivergences: number
  staleDivergences: number
  overallFail: number
}): number {
  if (args.unexpectedDivergences > 0) return 1
  if (args.staleDivergences > 0) return 1
  if (args.overallFail > 0) return 1
  return 0
}
