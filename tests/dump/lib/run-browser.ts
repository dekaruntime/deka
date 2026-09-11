import fs from 'fs'
import path from 'path'
import { spawnSync } from 'child_process'
import { fileURLToPath } from 'url'

const DUMP_ROOT = path.join(path.dirname(fileURLToPath(import.meta.url)), '..')
import type { Browser, Page, Route } from 'playwright'
import type { BuildCompileProjectResult } from './build-wasm'
import type { HatsTestStage } from './tests'
import { HARNESS_PROJECT_BASE, projectLoaderJs } from './project-loader'

export interface BrowserRunResult {
  ok: boolean
  stage: HatsTestStage
  stdout: string
  stderr: string
  error?: string
  diagnostics: Array<{
    severity: 'error' | 'warning' | 'info'
    message: string
    line?: number
    column?: number
  }>
}

let browser: Browser | null = null
let harnessBundlePath: string | null = null
let browserUnavailableReason: string | null = null
const EVALUATE_TIMEOUT_MS = 15_000

function harnessPath(): string {
  return path.join(DUMP_ROOT, '.cache', 'browser-harness.js')
}

export function getBrowserUnavailableReason(): string | null {
  return browserUnavailableReason
}

export async function closeBrowserHost(): Promise<void> {
  if (browser) {
    try {
      await browser.close()
    } catch {
      // ignore
    }
    browser = null
  }
}

function isInfraError(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error)
  return (
    message.includes('Target closed') ||
    message.includes('has been closed') ||
    message.includes('Browser closed') ||
    message.includes('Protocol error') ||
    message.includes('Execution context was destroyed')
  )
}

async function ensureHarnessBundle(): Promise<string> {
  const outPath = harnessPath()
  fs.mkdirSync(path.dirname(outPath), { recursive: true })
  const entry = path.join(DUMP_ROOT, 'lib', 'browser-harness-entry.ts')
  const bundled = spawnSync(
    'bun',
    ['build', entry, '--outfile', outPath, '--format', 'iife', '--target', 'browser'],
    { encoding: 'utf-8' }
  )
  if (bundled.status !== 0) {
    throw new Error(`failed to bundle browser harness: ${bundled.stderr || bundled.stdout}`)
  }
  return outPath
}

export async function prepareBrowserHost(): Promise<boolean> {
  if (browserUnavailableReason) return false
  if (browser) return true

  try {
    harnessBundlePath = await ensureHarnessBundle()
    const { chromium } = await import('playwright')
    browser = await chromium.launch({ headless: true })
    return true
  } catch (error) {
    browserUnavailableReason = error instanceof Error ? error.message : String(error)
    await closeBrowserHost()
    console.warn(`[hats] browser host unavailable: ${browserUnavailableReason}`)
    return false
  }
}

async function relaunchBrowser(): Promise<boolean> {
  await closeBrowserHost()
  browserUnavailableReason = null
  return prepareBrowserHost()
}

type HarnessRun = {
  ok: boolean
  stdout: string
  stderr: string
  error?: string
}

// Vendored stdlib shims served to the browser harness. Keep in sync with the
// real packages; io's echo is the console.log shim by design. The ui/*
// modules are served straight from the deka_ui crate so the harness never
// drifts from the real UI runtime: jsx/router/form/suspense ship as pinned
// compiler emit in emit/, the rest are still hand-written js/. Relative
// imports inside them (`./jsx.js`) are rewritten to the flat `.mjs` names the
// shim route serves.
function uiModuleSource(file: string): string {
  const crateDir = path.join(DUMP_ROOT, '..', '..', 'crates', 'deka_ui')
  let source: string
  try {
    source = fs.readFileSync(path.join(crateDir, 'js', file), 'utf8')
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error
    source = fs.readFileSync(path.join(crateDir, 'emit', file), 'utf8')
  }
  return source.replace(/from\s+['"]\.\/(\w+)\.js['"]/g, 'from "./$1.mjs"')
}
const MODULE_SHIMS: Record<string, string> = {
  // Closed compiler module (dsc#142): the compiler normally lowers
  // `import { PI } from "math"` to a local binding, but serve the module too
  // so any emitted JS that keeps the specifier resolves like io/time/crypto.
  'math.mjs': 'export const PI = Math.PI\n',
  'io.mjs': 'export function echo(message) {\n  console.log(message)\n}\n',
  'time.mjs': 'export function now() {\n  return Date.now()\n}\n',
  'crypto.mjs':
    'function uuid_v4_value() {\n' +
    '  const bytes = new Uint8Array(16);\n' +
    '  if (globalThis.crypto && typeof globalThis.crypto.getRandomValues === "function") {\n' +
    '    globalThis.crypto.getRandomValues(bytes);\n' +
    '  } else {\n' +
    '    for (let i = 0; i < 16; i++) bytes[i] = Math.floor(Math.random() * 256);\n' +
    '  }\n' +
    '  bytes[6] = (bytes[6] & 0x0f) | 0x40;\n' +
    '  bytes[8] = (bytes[8] & 0x3f) | 0x80;\n' +
    '  const hex = Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");\n' +
    '  return hex.slice(0, 8) + "-" + hex.slice(8, 12) + "-" + hex.slice(12, 16) + "-" + hex.slice(16, 20) + "-" + hex.slice(20);\n' +
    '}\n' +
    'export function uuid_v4() {\n' +
    '  return { __case: "Ok", value: uuid_v4_value() }\n' +
    '}\n',
  'jsx.mjs': uiModuleSource('jsx.js'),
  'reactive.mjs': uiModuleSource('reactive.js'),
  'suspense.mjs': uiModuleSource('suspense.js'),
  'server.mjs': uiModuleSource('server.js'),
  'island-marker.mjs': uiModuleSource('island-marker.js'),
}

async function evaluateInFreshPage(
  jsCode: string,
  projectModules?: Record<string, string>
): Promise<HarnessRun> {
  if (!browser || !harnessBundlePath) {
    throw new Error(browserUnavailableReason ?? 'browser host not started')
  }

  const context = await browser.newContext()
  context.setDefaultTimeout(EVALUATE_TIMEOUT_MS)
  // The compiler rewrites bare stdlib imports to HARNESS_MODULE_BASE URLs.
  // Intercept those and serve the vendored shims so the dump is
  // self-contained — no dependency on a live site, and the CORS header lets
  // the blob Worker (null origin) import them. Both patterns are registered
  // because module paths may be flat (io.mjs) or nested (ui/jsx.mjs).
  const shimHandler = (route: Route) => {
    const url = new URL(route.request().url())
    const name = url.pathname.split('/').pop() ?? ''
    const body = MODULE_SHIMS[name]
    if (body === undefined) {
      return route.fulfill({ status: 404, body: `no harness shim for ${name}` })
    }
    return route.fulfill({
      status: 200,
      contentType: 'text/javascript',
      headers: { 'access-control-allow-origin': '*' },
      body,
    })
  }
  await context.route('**/modules/*.mjs', shimHandler)
  await context.route('**/modules/**/*.mjs', shimHandler)
  // Project mode emits real ES modules; serve the compiler's per-module
  // output under HARNESS_PROJECT_BASE so relative specifiers (`./math.ds`)
  // resolve as ordinary ESM URLs. Stdlib imports in those modules point at
  // the shim routes above via the compiler's moduleBase rewrite.
  if (projectModules) {
    const projectPrefix = new URL(HARNESS_PROJECT_BASE).pathname // "/project"
    await context.route('**/project/**', (route: Route) => {
      const url = new URL(route.request().url())
      const key = decodeURIComponent(url.pathname.slice(projectPrefix.length + 1))
      const code = projectModules[key]
      if (code === undefined) {
        return route.fulfill({ status: 404, body: `no project module ${key}` })
      }
      return route.fulfill({
        status: 200,
        contentType: 'text/javascript',
        headers: { 'access-control-allow-origin': '*' },
        body: code,
      })
    })
  }
  const page = await context.newPage()
  try {
    await page.addScriptTag({ path: harnessBundlePath })
    return await page.evaluate(async (code: string) => {
      const g = globalThis as unknown as {
        __dekaRunJs: (js: string) => Promise<HarnessRun>
        __dekaTerminate?: () => void
      }
      try {
        return await g.__dekaRunJs(code)
      } finally {
        g.__dekaTerminate?.()
      }
    }, jsCode)
  } finally {
    await context.close()
  }
}

export async function runCompiledJsInBrowser(
  jsCode: string,
  projectModules?: Record<string, string>
): Promise<BrowserRunResult> {
  if (!browser) {
    return {
      ok: false,
      stage: 'run',
      stdout: '',
      stderr: '',
      error: browserUnavailableReason ?? 'browser host not started',
      diagnostics: [],
    }
  }

  const toResult = (result: HarnessRun): BrowserRunResult => ({
    ok: result.ok,
    stage: 'run',
    stdout: result.stdout ?? '',
    stderr: result.stderr ?? '',
    error: result.ok ? undefined : result.error,
    diagnostics: result.ok || !result.error ? [] : [{ severity: 'error', message: result.error }],
  })

  try {
    return toResult(await evaluateInFreshPage(jsCode, projectModules))
  } catch (error) {
    // Retry only closed-browser / protocol failures. A Deka program that
    // returns ok:false is a fixture finding, never an infra retry.
    if (isInfraError(error) && (await relaunchBrowser())) {
      try {
        return toResult(await evaluateInFreshPage(jsCode, projectModules))
      } catch (retryError) {
        const message = retryError instanceof Error ? retryError.message : String(retryError)
        return {
          ok: false,
          stage: 'run',
          stdout: '',
          stderr: '',
          error: message,
          diagnostics: [{ severity: 'error', message }],
        }
      }
    }
    const message = error instanceof Error ? error.message : String(error)
    return {
      ok: false,
      stage: 'run',
      stdout: '',
      stderr: '',
      error: message,
      diagnostics: [{ severity: 'error', message }],
    }
  }
}


/**
 * Run a compiled project in the browser. The project is compiled beforehand
 * by `compileProjectWithWasm` (build-wasm.ts) against the same local/published
 * compiler as single-file fixtures, with `moduleBase` set so bare stdlib
 * imports (`io`, …) resolve to the vendored shims (deka#497).
 */
export async function runProjectInBrowser(
  entryPath: string,
  compileResult: BuildCompileProjectResult
): Promise<BrowserRunResult> {
  const diagnostics = compileResult.diagnostics.map((d) => ({
    severity: d.severity,
    message: d.message,
    ...(d.line !== undefined ? { line: d.line } : {}),
    ...(d.column !== undefined ? { column: d.column } : {}),
  }))

  if (!compileResult.ok) {
    return {
      ok: false,
      stage: 'parse',
      stdout: '',
      stderr: '',
      error: compileResult.error ?? 'project compilation failed',
      diagnostics,
    }
  }

  const projectModules = Object.fromEntries(
    Object.entries(compileResult.modules).map(([modulePath, module]) => [modulePath, module.code])
  )
  const loader = projectLoaderJs(entryPath)
  const runResult = await runCompiledJsInBrowser(loader, projectModules)
  if (!runResult.ok && runResult.error) {
    diagnostics.push({ severity: 'error', message: runResult.error })
  }
  return { ...runResult, diagnostics }
}
