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
  process.env.HATS_FILTER || 'boolean_logic,component_fn_hoisted'

const { nativeAvailable, browserAvailable, categories } = await loadAndRunAllTests()

const selected = categories.flatMap((category) => category.tests)
if (selected.length === 0) {
  console.error(`[smoke] HATS_FILTER matched no fixtures: ${process.env.HATS_FILTER}`)
  process.exit(1)
}

let failed = 0
for (const test of selected) {
  const problems = []
  if (!browserAvailable) {
    problems.push('browser host unavailable')
  } else if (test.wasmResult.skipped) {
    problems.push(`browser run skipped: ${test.wasmResult.error ?? 'unknown'}`)
  } else if (!test.wasmMatches) {
    problems.push(`browser mismatch: ${test.wasmResult.error ?? JSON.stringify(test.wasmResult.stdout)}`)
  }
  if (nativeAvailable && !test.nativeResult.skipped && !test.nativeMatches) {
    problems.push(`native mismatch: ${test.nativeResult.error ?? JSON.stringify(test.nativeResult.stdout)}`)
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
