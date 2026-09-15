import { expect, test } from 'bun:test'
import { computeGateExitCode } from './gate-exit-code'

const clean = {
  unexpectedDivergences: 0,
  staleDivergences: 0,
  unexpectedFailures: 0,
  staleFailures: 0,
}

test('clean ratchets on both axes exits 0', () => {
  expect(computeGateExitCode(clean)).toBe(0)
})

test('an unlisted divergence exits non-zero even with everything else clean', () => {
  expect(computeGateExitCode({ ...clean, unexpectedDivergences: 1 })).toBe(1)
})

test('a stale listed divergence exits non-zero even with everything else clean', () => {
  expect(computeGateExitCode({ ...clean, staleDivergences: 1 })).toBe(1)
})

// deka#906: a run where every fixture outright fails, but the divergence
// ratchet is satisfied (no new/stale divergences -- e.g. failures aren't
// divergences at all, they're agreeing-failures on both hosts), must still
// exit non-zero if those failures are not accounted for in the baseline.
// The gate previously only reacted to the divergence ratchet and never
// looked at fixture failures at all, so this run reported success.
test('an unbaselined fixture failure exits non-zero even with a clean divergence ratchet', () => {
  expect(computeGateExitCode({ ...clean, unexpectedFailures: 42 })).toBe(1)
})

// The failure ratchet follow-up (this PR): #1040 turned the check above into
// an absolute zero-tolerance check on summary.overall.fail, which meant no
// release could ship until a large pre-existing backlog (71 known failures)
// was fully fixed. A slug in tests/dump/expected-failures-list.txt is
// pre-existing debt and must NOT fail the gate on its own.
test('a baselined known failure alone does not fail the gate', () => {
  expect(computeGateExitCode({ ...clean, unexpectedFailures: 0, staleFailures: 0 })).toBe(0)
})

// The other half of the ratchet: a baselined failure that starts passing is
// a stale entry, and must fail the gate exactly like a stale divergence
// does -- otherwise the baseline only ever grows and nobody is ever forced
// to prune it as debt gets paid down.
test('a stale baselined failure (now passing) exits non-zero even with zero unexpected failures', () => {
  expect(computeGateExitCode({ ...clean, staleFailures: 1 })).toBe(1)
})

test('an unexpected failure and a stale failure at the same time both surface as non-zero', () => {
  expect(computeGateExitCode({ ...clean, unexpectedFailures: 3, staleFailures: 2 })).toBe(1)
})
