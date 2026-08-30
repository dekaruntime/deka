import { describe, expect, it } from 'bun:test'
import { projectLoaderJs } from './project-loader'

/**
 * The generated loader splices each compiled module in as raw JavaScript
 * source. Regression guard for the escaping bug that produced
 * `SyntaxError: Invalid or unexpected token` on 205 conformance fixtures:
 * the body was escaped as if it were going inside a template literal.
 */
function loaderIsParseable(code: string): { ok: boolean; error?: string } {
  try {
    new Function(projectLoaderJs('main.ds', { 'main.ds': { code } }))
    return { ok: true }
  } catch (e) {
    return { ok: false, error: (e as Error).message }
  }
}

describe('projectLoaderJs', () => {
  it('accepts a module containing a template literal', () => {
    const r = loaderIsParseable('const k = "x";\nconst msg = `hello ${k}`;\nexports.msg = msg;')
    expect(r.error ?? 'ok').toBe('ok')
    expect(r.ok).toBe(true)
  })

  it('accepts the backtick form the struct prelude emits', () => {
    const r = loaderIsParseable(
      'const id = "Point";\nconst k = "set";\n' +
        'exports.m = () => `cannot call mutable method \'${k}\' on immutable ${id}`;'
    )
    expect(r.error ?? 'ok').toBe('ok')
  })

  it('preserves escape sequences instead of doubling the backslash', () => {
    // A literal newline must survive as a newline, not become backslash-n.
    const loader = projectLoaderJs('main.ds', {
      'main.ds': { code: 'module.exports.value = "a\\nb";' },
    })
    const run = new Function(`${loader.replace('__dekaRequire("main.ds");', '')}
      return __dekaRequire("main.ds").value;`)
    expect(run()).toBe('a\nb')
  })

  it('accepts a module with backslashes in a regex and a string', () => {
    const r = loaderIsParseable('exports.re = /\\d+\\\\/g;\nexports.s = "c:\\\\tmp";')
    expect(r.error ?? 'ok').toBe('ok')
  })
})
