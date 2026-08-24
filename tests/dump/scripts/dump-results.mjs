import fs from 'fs'
import path from 'path'
import { fileURLToPath } from 'url'
import { loadAndRunAllTests } from '../lib/build-tests.ts'

const repoRoot = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', '..', '..')

const { nativeAvailable, browserAvailable, version, wasmSourceCommit, categories } =
  await loadAndRunAllTests()

console.log(
  `[hats] nativeAvailable=${nativeAvailable} browserAvailable=${browserAvailable} version=${version}` +
    (wasmSourceCommit ? ` source_commit=${wasmSourceCommit}` : '')
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
  JSON.stringify({ nativeAvailable, browserAvailable, version, wasmSourceCommit, categories }, null, 2)
)
console.log(`[hats] wrote ${outPath}`)
