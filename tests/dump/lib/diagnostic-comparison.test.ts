import { expect, test } from 'bun:test'
import { diagnosticsAgree } from './build-tests'
import { normalizeDiagnostics } from './build-wasm'

const diagnostic = (message: string, line?: number, column?: number) => ({
  severity: 'error' as const,
  message,
  line,
  column,
})

test('compares the complete ordered diagnostic list', () => {
  const primary = diagnostic('primary', 1, 1)
  const cascade = diagnostic('cascade', 2, 3)

  expect(diagnosticsAgree([primary, cascade], [primary, cascade])).toBe(true)
  expect(diagnosticsAgree([primary], [primary, cascade])).toBe(false)
  expect(diagnosticsAgree([primary, cascade], [cascade, primary])).toBe(false)
  expect(diagnosticsAgree([primary], [diagnostic('primary', 1, 2)])).toBe(false)
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
