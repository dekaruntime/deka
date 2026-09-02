import fs from 'fs'
import path from 'path'
import { spawnSync } from 'child_process'
import { fileURLToPath } from 'url'

const DUMP_ROOT = path.join(path.dirname(fileURLToPath(import.meta.url)), '..')
import type { Browser, Page, Route } from 'playwright'
import { compileDekaProject } from '@dekaruntime/web-ide-kit/runtime'
import type { HatsTestStage } from './tests'
import { projectLoaderJs } from './project-loader'

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
// drifts from the real UI runtime. Relative imports inside them (`./jsx.js`)
// are rewritten to the flat `.mjs` names the shim route serves.
function uiModuleSource(file: string): string {
  const source = fs.readFileSync(
    path.join(DUMP_ROOT, '..', '..', 'crates', 'deka_ui', 'js', file),
    'utf8',
  )
  return source.replace(/from\s+['"]\.\/(\w+)\.js['"]/g, 'from "./$1.mjs"')
}
const MODULE_SHIMS: Record<string, string> = {
  'io.mjs': 'export function echo(message) {\n  console.log(message)\n}\n',
  'jsx.mjs': uiModuleSource('jsx.js'),
  'reactive.mjs': uiModuleSource('reactive.js'),
  'suspense.mjs': uiModuleSource('suspense.js'),
  'server.mjs': uiModuleSource('server.js'),
}

async function evaluateInFreshPage(jsCode: string): Promise<HarnessRun> {
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

export async function runCompiledJsInBrowser(jsCode: string): Promise<BrowserRunResult> {
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
    return toResult(await evaluateInFreshPage(jsCode))
  } catch (error) {
    // Retry only closed-browser / protocol failures. A Deka program that
    // returns ok:false is a fixture finding, never an infra retry.
    if (isInfraError(error) && (await relaunchBrowser())) {
      try {
        return toResult(await evaluateInFreshPage(jsCode))
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


export async function runProjectInBrowser(
  entryPath: string,
  files: Record<string, string>
): Promise<BrowserRunResult> {
  const compileResult = await compileDekaProject(files)
  const diagnostics = (compileResult.diagnostics ?? []).map((d) => ({
    severity: (d.severity === 'error' || d.severity === 'warning' || d.severity === 'info'
      ? d.severity
      : 'error') as 'error' | 'warning' | 'info',
    message: d.message,
  }))

  if (!compileResult.ok || Object.keys(compileResult.modules).length === 0) {
    return {
      ok: false,
      stage: 'parse',
      stdout: '',
      stderr: '',
      error: diagnostics.find((d) => d.severity === 'error')?.message ?? 'project compilation failed',
      diagnostics,
    }
  }

  const loader = projectLoaderJs(entryPath, compileResult.modules)
  const runResult = await runCompiledJsInBrowser(loader)
  if (!runResult.ok && runResult.error) {
    diagnostics.push({ severity: 'error', message: runResult.error })
  }
  return { ...runResult, diagnostics }
}
