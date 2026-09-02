import { describe, expect, it } from 'bun:test'
import { HARNESS_PROJECT_BASE, projectLoaderJs } from './project-loader'

describe('projectLoaderJs', () => {
  it('bootstraps the entry module with a dynamic import', () => {
    const loader = projectLoaderJs('main.pass.ds')
    expect(loader).toBe(`await import("${HARNESS_PROJECT_BASE}/main.pass.ds");\n`)
    // The bootstrap runs inside an async IIFE in `new Function()` in the
    // sandbox worker, so it must not contain static import/export syntax.
    expect(() => new Function(`return (async () => {\n${loader}\n})();`)).not.toThrow()
  })

  it('normalizes leading ./ from the entry path', () => {
    const loader = projectLoaderJs('./main.pass.ds')
    expect(loader).toContain(`${HARNESS_PROJECT_BASE}/main.pass.ds`)
    expect(loader).not.toContain('./main.pass.ds')
  })

  it('escapes the entry path as a JSON string', () => {
    const loader = projectLoaderJs('sub/dir/main.pass.ds')
    expect(loader).toBe(`await import("${HARNESS_PROJECT_BASE}/sub/dir/main.pass.ds");\n`)
  })
})
