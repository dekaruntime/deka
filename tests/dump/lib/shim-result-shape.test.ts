import { expect, test } from 'bun:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
import { MODULE_SHIMS } from './run-browser'

// The browser harness serves these vendored shims for the bare stdlib
// imports the compiler rewrites to HARNESS_MODULE_BASE URLs. A shim that
// returns a stale value shape breaks `match` in the emitted JS exactly the
// way deka#931 did: no arm matches, the binding is undefined, and the
// fixture fails at run stage on the browser host only.

async function importShim(name: string): Promise<Record<string, unknown>> {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'dump-shim-test-'))
  try {
    const file = path.join(dir, name)
    fs.writeFileSync(file, MODULE_SHIMS[name])
    return (await import(pathToFileURL(file).href)) as Record<string, unknown>
  } finally {
    fs.rmSync(dir, { recursive: true, force: true })
  }
}

test('crypto shim returns the compiler Result shape uuid_v4 match arms test', async () => {
  const shim = await importShim('crypto.mjs')
  const uuidV4 = shim.uuid_v4 as () => unknown
  const result = uuidV4()

  // Emitted JS for `match (uuid_v4()) { Ok(v) => v, Err(e) => "" }` binds
  // through `.ok` alone; this is the exact lowering the wasm compiler ships.
  let bound: unknown
  const scrutinee = result as { ok?: unknown; value?: unknown; error?: unknown }
  if (scrutinee.ok === true) {
    bound = scrutinee.value
  } else if (scrutinee.ok === false) {
    bound = ''
  }
  expect(bound).toBe(scrutinee.value)
  expect(typeof bound).toBe('string')
  expect(bound as string).toMatch(
    /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/,
  )
})

test('crypto shim value is a fresh RFC 4122 version-4 UUID per call', async () => {
  const shim = await importShim('crypto.mjs')
  const uuidV4 = shim.uuid_v4 as () => { ok: boolean; value: string }
  const first = uuidV4()
  const second = uuidV4()
  expect(first.ok).toBe(true)
  expect(second.ok).toBe(true)
  expect(first.value).not.toBe(second.value)
  expect(first.value).toHaveLength(36)
})
