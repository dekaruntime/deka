/**
 * Builds the bootstrap the browser harness evaluates to run a compiled
 * multi-file project.
 *
 * Project mode emits real ES modules. Instead of wrapping them in
 * CommonJS factory functions (which cannot parse `import`/`export`), the
 * harness serves each compiled module over an intercepted HTTP route under
 * HARNESS_PROJECT_BASE (see run-browser.ts) and dynamically imports the
 * entry. Relative specifiers in the emitted code (`./math.ds`) resolve
 * against the module URLs, and bare stdlib specifiers were already rewritten
 * by the compiler's moduleBase option to shim URLs the harness also serves —
 * the same mechanism the single-file path uses (deka#497).
 *
 * Pure string construction, deliberately free of playwright and compiler
 * imports so it can be unit tested without a browser or the compiler.
 */

/** Fake origin path the harness intercepts to serve compiled project modules. */
export const HARNESS_PROJECT_BASE = 'https://hats.dump.invalid/project'

function normalizePath(filePath: string): string {
  return filePath.replace(/\\/g, '/').replace(/^\.\//, '')
}

export function projectLoaderJs(entryPath: string): string {
  const normalizedEntry = normalizePath(entryPath)
  // Dynamic import: the sandbox executes this bootstrap inside an async IIFE
  // via `new Function()`, where static import declarations are not allowed.
  return `await import(${JSON.stringify(`${HARNESS_PROJECT_BASE}/${normalizedEntry}`)});\n`
}
