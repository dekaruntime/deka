// Execute the actual distributable loader + WASM, not a native substitute.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { createHash } from 'node:crypto'
import { resolve } from 'node:path'
import { pathToFileURL } from 'node:url'
const root = resolve(process.argv[2])
const manifest = JSON.parse(readFileSync(resolve(root, 'manifest.json')))
assert.equal(manifest.runtime, 'deka_vm')
assert.equal(manifest.abiVersion, 1)
assert.equal(manifest.compilerCommit, manifest.commit)
if (process.argv[3]) assert.equal(manifest.version, process.argv[3])
if (process.argv[4]) assert.equal(manifest.commit, process.argv[4])
for (const [name, expected] of Object.entries(manifest.files)) {
  const bytes = readFileSync(resolve(root, name))
  assert.equal(bytes.length, expected.bytes)
  assert.equal(createHash('sha256').update(bytes).digest('hex'), expected.sha256)
}
const { default: init, NativePreview } = await import(pathToFileURL(resolve(root, 'deka_native_web.js')))
await init({ module_or_path: readFileSync(resolve(root, 'deka_native_web_bg.wasm')) })
const runtime = new NativePreview()
try {
  runtime.compile('export fn App() { let count = 0; return (<view><p>Count: {count}</p><button onClick={fn() { count += 1; }}>Add</button></view>); }', true)
  const scene = () => JSON.parse(runtime.frame(400, 300, 1))
  assert(scene().nodes.some(n => n.text === 'Count: 0'))
  const rect = scene().targets[0].rect
  assert(runtime.pointer(rect.x + 2, rect.y + 2))
  assert(scene().nodes.some(n => n.text === 'Count: 1'))
  assert.throws(() => runtime.compile('export fn App() { !!!', true))
  assert(scene().nodes.some(n => n.text === 'Count: 1'))
} finally { runtime.free() }
console.log('PASS: distributed native WASM compiles source, renders, handles input and preserves state on errors.')
