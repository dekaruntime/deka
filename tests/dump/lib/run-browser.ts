import fs from 'fs'
import path from 'path'
import { spawnSync } from 'child_process'
import { fileURLToPath } from 'url'

const DUMP_ROOT = path.join(path.dirname(fileURLToPath(import.meta.url)), '..')
import type { Browser, Page } from 'playwright'
import { compileDekaProject } from '@dekaruntime/web-ide-kit/runtime'
import type { HatsTestStage } from './tests'

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

async function evaluateInFreshPage(jsCode: string): Promise<HarnessRun> {
  if (!browser || !harnessBundlePath) {
    throw new Error(browserUnavailableReason ?? 'browser host not started')
  }

  const context = await browser.newContext()
  context.setDefaultTimeout(EVALUATE_TIMEOUT_MS)
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

function normalizePath(filePath: string): string {
  return filePath.replace(/\\/g, '/').replace(/^\.\//, '')
}

function projectLoaderJs(entryPath: string, modules: Record<string, { code: string }>): string {
  const normalizedEntry = normalizePath(entryPath)
  const moduleEntries = Object.entries(modules).map(([modulePath, module]) => {
    const safePath = JSON.stringify(modulePath)
    const escapedCode = module.code.replace(/\\/g, '\\\\').replace(/`/g, '\\`').replace(/\$\{/g, '\\${')
    return `  ${safePath}: function(exports, __dekaRequire, module) {\n${escapedCode}\n}`
  })
  return `
const __dekaModules = {\n${moduleEntries.join(',\n')}\n};
const __dekaCache = new Map();
function __dekaResolve(spec, currentPath) {
  if (!spec.startsWith('./') && !spec.startsWith('../')) return spec;
  const base = currentPath.includes('/') ? currentPath.slice(0, currentPath.lastIndexOf('/') + 1) : '';
  const parts = (base + spec).split('/').filter(Boolean);
  const resolved = [];
  for (const part of parts) {
    if (part === '..') resolved.pop();
    else if (part !== '.') resolved.push(part);
  }
  return resolved.join('/');
}
function __dekaRequire(spec, currentPath) {
  const normalized = __dekaResolve(spec, currentPath || ${JSON.stringify(normalizedEntry)});
  if (__dekaCache.has(normalized)) return __dekaCache.get(normalized);
  const factory = __dekaModules[normalized];
  if (!factory) throw new Error('Module not found: ' + spec + ' (resolved to ' + normalized + ')');
  const module = { exports: {} };
  factory(module.exports, (s) => __dekaRequire(s, normalized), module);
  __dekaCache.set(normalized, module.exports);
  return module.exports;
}
__dekaRequire(${JSON.stringify(normalizedEntry)});
`
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
