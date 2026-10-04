// Actual generated browser binding, executed headlessly without a screen.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve, join } from 'node:path'
import { pathToFileURL } from 'node:url'
const [bindings, cases] = process.argv.slice(2)
assert.ok(bindings && cases, 'usage: node formatter-wasm.mjs <generated-web-directory> <native-cases.jsonl>')
const module = await import(pathToFileURL(join(resolve(bindings), 'deka_native_web.js')))
await module.default({ module_or_path: readFileSync(join(bindings, 'deka_native_web_bg.wasm')) })
assert.equal(typeof module.format_ds, 'function', 'generated WASM package must expose format_ds')
assert.equal(module.format_ds('const count=0\n'), 'const count = 0\n')
assert.equal(module.format_ds('fn App(\n'), 'fn App(\n')
let count = 0
for (const line of readFileSync(cases, 'utf8').trim().split('\n')) {
  const { file, source, expected } = JSON.parse(line)
  const actual = module.format_ds(source)
  assert.equal(actual, expected, `native/WASM formatter output differs: ${file}`)
  assert.equal(module.format_ds(actual), actual, `WASM second pass differs: ${file}`)
  count++
}
assert.ok(count > 0, 'no conformance cases executed')
console.log(`actual WASM formatter agrees with native output for ${count} sources`)
