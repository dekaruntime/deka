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
const outPath =
  process.env.DEKA_DUMP_OUT ||
  path.join(repoRoot, 'dist', 'conformance', 'hats-results.json')
fs.mkdirSync(path.dirname(outPath), { recursive: true })
fs.writeFileSync(
  outPath,
  JSON.stringify({ nativeAvailable, browserAvailable, version, wasmSourceCommit, webIdeKitVersion, categories }, null, 2)
)
console.log(`[hats] wrote ${outPath}`)
