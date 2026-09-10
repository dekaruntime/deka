import { expect, test } from 'bun:test'
import { computeOverallStatus } from './overall-status'

const sharedPassing = {
  wantNative: true,
  wantBrowser: true,
  nativeAvailable: true,
  browserAvailable: true,
  nativeMatches: true,
  browserMatches: true,
  nativeSkipped: false,
  browserSkipped: false,
  fmtHostsAgree: true,
}

test('a full diagnostic-list mismatch is a shared-host divergence', () => {
  expect(computeOverallStatus({ ...sharedPassing, diagnosticsAgree: false })).toBe('divergent')
  expect(computeOverallStatus({ ...sharedPassing, diagnosticsAgree: true })).toBe('pass')
})
