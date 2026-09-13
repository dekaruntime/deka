import { expect, test } from 'bun:test'
import { diagnosticsAgree } from './build-tests'
import { normalizeDiagnostics } from './build-wasm'
import { packagesFor, parseNativeDiagnostics } from './build-native'

const diagnostic = (message: string, line?: number, column?: number) => ({
  severity: 'error' as const,
  message,
  line,
  column,
})

test('compares the complete diagnostic SET, ignoring order and span', () => {
  const primary = diagnostic('primary', 1, 1)
  const cascade = diagnostic('cascade', 2, 3)

  expect(diagnosticsAgree([primary, cascade], [primary, cascade])).toBe(true)
  expect(diagnosticsAgree([primary], [primary, cascade])).toBe(false)
  expect(diagnosticsAgree([primary, cascade], [cascade, primary])).toBe(true)
  expect(diagnosticsAgree([primary], [diagnostic('primary', 1, 2)])).toBe(true)
})

test('normalizes dsc ABI start positions into the host diagnostic shape', () => {
  expect(
    normalizeDiagnostics([
      {
        severity: 'error',
        message: 'primary',
        start_line: 4,
        start_column: 7,
        end_line: 4,
        end_column: 8,
      },
    ]),
  ).toEqual([diagnostic('primary', 4, 7)])
})

test('keeps a multi-diagnostic WASM result comparable to native', () => {
  const wasm = normalizeDiagnostics([
    { severity: 'error', message: 'primary', start_line: 4, start_column: 7 },
    { severity: 'error', message: 'cascade', start_line: 9, start_column: 2 },
  ])
  expect(
    diagnosticsAgree(wasm, [diagnostic('primary', 4, 7), diagnostic('cascade', 9, 2)]),
  ).toBe(true)
})

test('expands a concatenated project-mode wasm diagnostic to the native SET', () => {
  const wasm = normalizeDiagnostics([
    {
      severity: 'error',
      message:
        'test.ds: cannot call mutable method `inc` on an immutable receiver\ntest.ds: expected argument type `string`, found type `number`',
      start_line: 1,
      start_column: 1,
    },
  ])
  const native = parseNativeDiagnostics(
    [
      '[transpile] 10:9: /tmp/hats-native-run-x/test.ds: cannot call mutable method `inc` on an immutable receiver',
      '11:8: /tmp/hats-native-run-x/test.ds: expected argument type `string`, found type `number`',
      '',
    ].join('\n'),
  )
  expect(wasm.map((d) => d.message)).toEqual([
    'cannot call mutable method `inc` on an immutable receiver',
    'expected argument type `string`, found type `number`',
  ])
  expect(diagnosticsAgree(native, wasm)).toBe(true)
})

test('packagesFor auto-adds io so native install and wasm stubs share a list', () => {
  expect(packagesFor('import { echo } from "io"\n', undefined, undefined)).toEqual(['io'])
  expect(packagesFor('const x = 1\n', undefined, ['crypto'])).toEqual(['crypto'])
  expect(packagesFor('import { echo } from "io"\n', undefined, ['crypto'])).toEqual(['crypto', 'io'])
})

test('strips a project-mode filename prefix from a single wasm diagnostic', () => {
  const wasm = normalizeDiagnostics([
    {
      severity: 'error',
      message: 'test.ds: expected argument type `string`, found type `number`',
      start_line: 1,
      start_column: 1,
    },
  ])
  expect(wasm[0].message).toBe('expected argument type `string`, found type `number`')
  expect(
    diagnosticsAgree(wasm, [diagnostic('expected argument type `string`, found type `number`', 11, 8)]),
  ).toBe(true)
})
