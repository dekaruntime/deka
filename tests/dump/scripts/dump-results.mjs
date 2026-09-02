import fs from 'fs'
import path from 'path'
import { fileURLToPath } from 'url'
import { loadAndRunAllTests } from '../lib/build-tests.ts'
import { runAdhocScenarios, toHatsCategory } from '../../testsuite/adhoc/cases.mjs'

const repoRoot = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', '..', '..')

// Which web-ide-kit produced this pack. The site renders results with its own
// copy, and a caret range on a 0.x version is minor-locked -- `^0.1.8` can never
// resolve to `0.2.x` -- so the two silently skew and the pack ends up describing
// a compiler nobody is running (deka#357). Recording the RESOLVED version, not
// the manifest range, makes that detectable from the artifact itself instead of
// by comparing two repositories' manifests.
function resolveWebIdeKitVersion() {
  try {
    const url = new URL('../node_modules/@dekaruntime/web-ide-kit/package.json', import.meta.url);
    return JSON.parse(fs.readFileSync(url, 'utf-8')).version ?? null;
  } catch {
    return null;
  }
}
const webIdeKitVersion = resolveWebIdeKitVersion();

const { nativeAvailable, browserAvailable, version, wasmSourceCommit, categories } =
  await loadAndRunAllTests()

const adhoc = await runAdhocScenarios({
  cli: process.env.DEKA_NATIVE || undefined,
  wasmPath: process.env.DEKA_WASM || undefined,
})
const adhocCategory = toHatsCategory(adhoc.results)
categories.unshift(adhocCategory)
console.log(`[hats] ADHOC scenarios=${adhoc.results.length}`)
for (const test of adhocCategory.tests) {
  console.log(`[hats] ${test.slug}: overall=${test.overallStatus}`)
}

console.log(
  `[hats] nativeAvailable=${nativeAvailable} browserAvailable=${browserAvailable} version=${version}` +
    (wasmSourceCommit ? ` source_commit=${wasmSourceCommit}` : '') +
    (webIdeKitVersion ? ` web-ide-kit=${webIdeKitVersion}` : ' web-ide-kit=UNKNOWN')
)

for (const category of categories) {
  for (const test of category.tests) {
    console.log(`[hats] ${test.slug}: overall=${test.overallStatus} wasm=${test.wasmMatches} native=${test.nativeMatches}`)
    console.log(`  wasm stage=${test.wasmResult.stage} ok=${test.wasmResult.ok} stdout=${JSON.stringify(test.wasmResult.stdout)} stderr=${JSON.stringify(test.wasmResult.stderr)} error=${JSON.stringify(test.wasmResult.error)} skipped=${test.wasmResult.skipped || false}`)
    console.log(`  native stage=${test.nativeResult.stage} ok=${test.nativeResult.ok} stdout=${JSON.stringify(test.nativeResult.stdout)} stderr=${JSON.stringify(test.nativeResult.stderr)} error=${JSON.stringify(test.nativeResult.error)}`)
  }
}

// Persist results so the static export can read them without re-running the
// full conformance suite inside the Next.js SSG environment.
// The pack carries the summary the site renders. Consumers must not recompute
// it -- one producer owns every number (see TESTING.md, deka#503).
//
// Three groups, decided by each fixture's `hosts`. Divergence exists only in
// `shared`, because it is meaningless for a fixture that runs on one host, and
// computing it corpus-wide is how harness gaps get counted as compiler defects
// (deka#509). There is no skip bucket in any group.
function summarize(categories) {
  const groups = {
    'native-only': { pass: 0, fail: 0, total: 0 },
    shared: { pass: 0, fail: 0, diverge: 0, total: 0 },
    'browser-only': { pass: 0, fail: 0, total: 0 },
  }
  for (const category of categories) {
    for (const test of category.tests) {
      const hosts = new Set(test.hosts || [])
      const onNative = hosts.has('native')
      const onBrowser = hosts.has('browser')
      const name = onNative && onBrowser ? 'shared' : onNative ? 'native-only' : 'browser-only'
      const g = groups[name]
      g.total++
      if (name === 'shared') {
        if (test.nativeMatches && test.wasmMatches) g.pass++
        else if (test.nativeMatches !== test.wasmMatches) g.diverge++
        else g.fail++
      } else if (name === 'native-only') {
        test.nativeMatches ? g.pass++ : g.fail++
      } else {
        test.wasmMatches ? g.pass++ : g.fail++
      }
    }
  }
  // Every fixture lands in exactly one group, and every group accounts for all
  // of its own. Nothing skips, so a shortfall means a fixture fell out.
  for (const [name, g] of Object.entries(groups)) {
    const accounted = g.pass + g.fail + (g.diverge || 0)
    if (accounted !== g.total) {
      throw new Error(`reconciliation failed in ${name}: ${accounted} of ${g.total}`)
    }
  }
  return groups
}

const groups = summarize(categories)
console.log(
  `[hats] native-only ${groups['native-only'].pass}/${groups['native-only'].total} · ` +
    `shared ${groups.shared.pass}/${groups.shared.total} (${groups.shared.diverge} diverge) · ` +
    `browser-only ${groups['browser-only'].pass}/${groups['browser-only'].total}`
)

const outPath =
  process.env.DEKA_DUMP_OUT ||
  path.join(repoRoot, 'dist', 'conformance', 'hats-results.json')
fs.mkdirSync(path.dirname(outPath), { recursive: true })
fs.writeFileSync(
  outPath,
  JSON.stringify({ nativeAvailable, browserAvailable, version, wasmSourceCommit, webIdeKitVersion, groups, categories }, null, 2)
)
console.log(`[hats] wrote ${outPath}`)
