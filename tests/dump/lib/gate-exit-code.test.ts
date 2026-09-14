import { expect, test } from 'bun:test'
import { computeGateExitCode } from './gate-exit-code'

test('clean ratchet and clean fail count exits 0', () => {
  expect(
    computeGateExitCode({ unexpectedDivergences: 0, staleDivergences: 0, overallFail: 0 }),
  ).toBe(0)
})

test('an unlisted divergence exits non-zero even with zero fails', () => {
  expect(
    computeGateExitCode({ unexpectedDivergences: 1, staleDivergences: 0, overallFail: 0 }),
  ).toBe(1)
})

test('a stale listed divergence exits non-zero even with zero fails', () => {
  expect(
    computeGateExitCode({ unexpectedDivergences: 0, staleDivergences: 1, overallFail: 0 }),
  ).toBe(1)
})

// deka#906: a run where every fixture outright fails, but the divergence
// ratchet is satisfied (no new/stale divergences -- e.g. failures aren't
// divergences at all, they're agreeing-failures on both hosts), must still
// exit non-zero. The gate previously only reacted to the ratchet and never
// looked at summary.overall.fail, so this run reported success.
test('a clean ratchet with every fixture failing still exits non-zero', () => {
  expect(
    computeGateExitCode({ unexpectedDivergences: 0, staleDivergences: 0, overallFail: 42 }),
  ).toBe(1)
})
