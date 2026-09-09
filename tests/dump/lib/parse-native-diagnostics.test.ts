import { describe, expect, test } from 'bun:test'
import { parseNativeDiagnostics } from './build-native'

// Every fixture below is verbatim stderr taken from the conformance pack
// shipped with deka v0.45.0 (conformance.tar.gz, hats-results.json). Before
// deka#739 the parser dropped the FIRST diagnostic of every one of these,
// because that line carries a `[transpile] ` prefix the pattern did not admit.
describe('parseNativeDiagnostics', () => {
  test('captures the [transpile]-prefixed primary diagnostic', () => {
    // async-async-argument-type-mismatch-fail
    const stderr =
      '[transpile] 6:15: /tmp/hats-native-run-x/test.ds: expected argument type `string`, found type `number`\n'
    const d = parseNativeDiagnostics(stderr)
    expect(d).toHaveLength(1)
    expect(d[0].message).toBe('expected argument type `string`, found type `number`')
    expect(d[0].line).toBe(6)
    expect(d[0].column).toBe(15)
  })

  test('captures BOTH diagnostics when the first is prefixed and the second is not', () => {
    // async-async-method-missing-promise-return-fail — the case that made this
    // test suite report a host divergence that does not exist.
    const stderr = [
      '[transpile] 6:1: /tmp/hats-native-run-x/test.ds: async function must return Promise<T>, found type `number`',
      '10:6: /tmp/hats-native-run-x/test.ds: `await` expected Promise<T>, found type `number`',
      '',
    ].join('\n')
    const d = parseNativeDiagnostics(stderr)
    expect(d.map((x) => x.message)).toEqual([
      'async function must return Promise<T>, found type `number`',
      '`await` expected Promise<T>, found type `number`',
    ])
    // The primary must come first: wasm reports diagnostics[0] as the error,
    // so ordering is what makes the two hosts comparable at all.
    expect(d[0].line).toBe(6)
    expect(d[1].line).toBe(10)
  })

  test('parses a diagnostic that carries no file path', () => {
    // basics-leading-zero-octal-fail
    const d = parseNativeDiagnostics(
      '[transpile] 2:6: leading-zero octal-style integers are not allowed in DekaScript\n',
    )
    expect(d).toHaveLength(1)
    expect(d[0].message).toBe(
      'leading-zero octal-style integers are not allowed in DekaScript',
    )
  })

  test('never records the legacy wrapper line as a diagnostic', () => {
    // The runtime used to prepend this ahead of dsc's real output. It was
    // recorded as THE diagnostic for 317 tests in the v0.45.0 pack.
    const stderr = [
      'dsc transpile failed (need dsc >= 0.5.0 for --self-contained):',
      '[transpile] 2:6: leading-zero octal-style integers are not allowed in DekaScript',
      '',
    ].join('\n')
    const d = parseNativeDiagnostics(stderr)
    expect(d.map((x) => x.message)).toEqual([
      'leading-zero octal-style integers are not allowed in DekaScript',
    ])
    expect(d.some((x) => x.message.startsWith('dsc transpile failed'))).toBe(false)
  })

  test('wrapper alone, with no diagnostic behind it, is not reported as one', () => {
    const d = parseNativeDiagnostics(
      'dsc transpile failed (need dsc >= 0.5.0 for --self-contained):\n',
    )
    expect(d.some((x) => x.message.startsWith('dsc transpile failed'))).toBe(false)
  })

  test('still parses the rich ┌─ / ^ diagnostic form', () => {
    const stderr = ['┌─ /tmp/test.ds:3:9', '│   ^ unknown identifier `nope`'].join('\n')
    const d = parseNativeDiagnostics(stderr)
    expect(d).toHaveLength(1)
    expect(d[0].message).toBe('unknown identifier `nope`')
    expect(d[0].line).toBe(3)
    expect(d[0].column).toBe(9)
  })
})
