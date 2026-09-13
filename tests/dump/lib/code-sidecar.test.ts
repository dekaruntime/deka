import { expect, test } from 'bun:test'
import fs from 'fs'
import os from 'os'
import path from 'path'
import { runtimeMatchesExpectation } from './build-tests'
import {
  collectFixtureFiles,
  isFixtureAsset,
  isWasmProjectSource,
  parseCodeSidecar,
  type HatsTest,
} from './tests'

test('numeric .code sidecars are process exit codes (dsc Hats, deka#929)', () => {
  expect(parseCodeSidecar('0\n')).toEqual({ expectedExitCode: 0 })
  expect(parseCodeSidecar('1\n')).toEqual({ expectedExitCode: 1 })
  expect(parseCodeSidecar('  0  ')).toEqual({ expectedExitCode: 0 })
  expect(parseCodeSidecar(undefined)).toEqual({})
})

test('non-numeric .code sidecars stay formatted source', () => {
  const source = 'fn main() void {\n  print("hi")\n}\n'
  expect(parseCodeSidecar(source)).toEqual({ expectedFormattedCode: source })
  expect(parseCodeSidecar('0 still source\n')).toEqual({ expectedFormattedCode: '0 still source\n' })
})

test('fixture collection includes sibling foreign.mjs and .css (deka#930)', () => {
  expect(isFixtureAsset('arity_mismatch.fail.ds')).toBe(true)
  expect(isFixtureAsset('page.pass.dsx')).toBe(true)
  expect(isFixtureAsset('foreign.mjs')).toBe(true)
  expect(isFixtureAsset('theme.css')).toBe(true)
  expect(isFixtureAsset('arity_mismatch.code')).toBe(false)
  expect(isFixtureAsset('arity_mismatch.json')).toBe(false)
  expect(isFixtureAsset('arity_mismatch.stdout')).toBe(false)

  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'dump-fixture-'))
  try {
    fs.writeFileSync(path.join(root, 'arity_mismatch.fail.ds'), 'summon { total read(): number } from "./foreign.mjs"\n')
    fs.writeFileSync(path.join(root, 'foreign.mjs'), 'export function read() { return 1 }\n')
    fs.writeFileSync(path.join(root, 'arity_mismatch.json'), '{}\n')
    fs.writeFileSync(path.join(root, 'arity_mismatch.code'), '1\n')
    fs.writeFileSync(path.join(root, 'theme.css'), '.x{}\n')
    const files = collectFixtureFiles(root, root).sort()
    expect(files).toEqual(['arity_mismatch.fail.ds', 'foreign.mjs', 'theme.css'])
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
})

test('wasm project sources exclude native-only .mjs assets', () => {
  expect(isWasmProjectSource('main.pass.ds')).toBe(true)
  expect(isWasmProjectSource('page.dsx')).toBe(true)
  expect(isWasmProjectSource('theme.css')).toBe(true)
  expect(isWasmProjectSource('foreign.mjs')).toBe(false)
})

test('numeric .code does not fail the formatted-source comparison', () => {
  const sidecar = parseCodeSidecar('0\n')
  const testCase: HatsTest = {
    slug: 'exceptions-async-local-try',
    category: 'exceptions',
    status: 'pass',
    name: 'async_local_try',
    dir: '/tmp',
    source: 'fn main() void {}\n',
    title: 'async local try',
    stage: 'run',
    hosts: ['native', 'browser'],
    expectedCode: sidecar.expectedFormattedCode,
    expectedExitCode: sidecar.expectedExitCode,
  }
  const result = {
    ok: true,
    stage: 'run' as const,
    stdout: '',
    stderr: '',
    exitCode: 0,
    formattedCode: 'fn main() void {}\n',
    diagnostics: [],
  }
  expect(runtimeMatchesExpectation(testCase, result, 'browser')).toBe(true)
  expect(runtimeMatchesExpectation(testCase, { ...result, exitCode: 1, ok: false }, 'browser')).toBe(
    false
  )
})
