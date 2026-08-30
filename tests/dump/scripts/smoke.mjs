// PR-CI smoke for the dump harness. Runs a handful of fixtures that cover the
// browser execution path end to end — plain DS, the io.mjs shim, and the
// ui/jsx.mjs shim — so harness breakage (bad shim paths, import-transform
// regressions, playwright wiring) fails on the PR instead of at release time.
//
// Env: DEKA_NATIVE and DEKA_WASM must point at artifacts built from the same
// commit (the CI rust job builds both). HATS_FILTER overrides the default
// fixture selection.

import { loadAndRunAllTests } from '../lib/build-tests.ts'

process.env.HATS_FILTER =
  process.env.HATS_FILTER || 'string-concatenation,component-fn-hoisted'

const { nativeAvailable, browserAvailable, categories } = await loadAndRunAllTests()

const selected = categories.flatMap((category) => category.tests)
if (selected.length === 0) {
  console.error(`[smoke] HATS_FILTER matched no fixtures: ${process.env.HATS_FILTER}`)
  process.exit(1)
}

let failed = 0
for (const test of selected) {
  const problems = []
  const describe = (label, test, result, matches, ignoreCode) => {
    if (result.skipped) return `${label} run skipped: ${result.error ?? 'unknown'}`
    if (matches) return null
    if (result.error) return `${label} mismatch: ${result.error}`
    if ((result.ok ? 'pass' : 'fail') !== test.status) return `${label} status mismatch: got ${result.ok ? 'pass' : 'fail'}, want ${test.status}`
    const expected = test.expectedStdout ?? ''
    if (result.stdout !== expected) return `${label} stdout mismatch: got ${JSON.stringify(result.stdout)}, want ${JSON.stringify(expected)}`
    if (!ignoreCode && test.expectedCode !== undefined) return `${label} formatted-code mismatch (formatter drift, not runtime)`
    return `${label} mismatch`
  }
  if (!browserAvailable) {
    problems.push('browser host unavailable')
  } else {
    const problem = describe('browser', test, test.wasmResult, test.wasmMatches, false)
    if (problem) problems.push(problem)
  }
  if (nativeAvailable) {
    const problem = describe('native', test, test.nativeResult, test.nativeMatches, true)
    if (problem) problems.push(problem)
  }
  if (problems.length > 0) {
    failed++
    console.error(`[smoke] FAIL ${test.slug}`)
    for (const problem of problems) console.error(`  ${problem}`)
  } else {
    console.log(`[smoke] ok ${test.slug}`)
  }
}

if (failed > 0) {
  console.error(`[smoke] ${failed}/${selected.length} fixtures failed`)
  process.exit(1)
}
console.log(`[smoke] ${selected.length} fixtures passed`)
