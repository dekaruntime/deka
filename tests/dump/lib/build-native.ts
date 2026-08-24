import fs from 'fs'
import path from 'path'
import { execSync, spawnSync } from 'child_process'
import os from 'os'

const RELEASES_BASE = 'https://releases.deka.gg'

const DEFAULT_DEKA_LOCK = '{\n  "lockfileVersion": 1,\n  "packages": {}\n}\n'

const DEFAULT_DEKA_JSON = {
  name: 'conformance-fixture',
  security: {
    allow: {
      read: ['./'],
      write: ['.cache'],
    },
    prompt: false,
  },
}

const PACKAGE_DEKA_JSON = {
  name: 'conformance-fixture',
  security: {
    allow: {
      read: ['./'],
      write: ['.cache', 'php_modules'],
    },
    prompt: false,
  },
}

export interface NativeRunResult {
  ok: boolean
  stdout: string
  stderr: string
  error?: string
  transpileFailed: boolean
  emittedJs?: string
  diagnostics: Array<{
    severity: 'error' | 'warning' | 'info'
    message: string
    line?: number
    column?: number
  }>
}

let nativeCliPath: string | null = null

function getPlatformBinaryName(): string | null {
  const platform = os.platform()
  const arch = os.arch()
  if (platform === 'linux' && arch === 'x64') return 'deka-linux-x64'
  if (platform === 'darwin' && arch === 'x64') return 'deka-darwin-x64'
  if (platform === 'darwin' && arch === 'arm64') return 'deka-darwin-arm64'
  return null
}

export async function prepareNativeCli(version: string): Promise<string | null> {
  if (nativeCliPath) return nativeCliPath

  // CI uses the published CLI. Local / branch validation must be able to point
  // both hosts at the same unreleased build (pair with DEKA_WASM):
  //   DEKA_NATIVE=../deka/target/release/cli \
  //   DEKA_WASM=../deka/target/wasm32-unknown-unknown/release/deka_compiler_wasm.wasm \
  //     bun scripts/dump-results.mjs
  const localNative = process.env.DEKA_NATIVE
  if (localNative) {
    const resolved = path.resolve(localNative)
    if (!fs.existsSync(resolved)) {
      throw new Error(`DEKA_NATIVE is set to ${resolved} but that file does not exist`)
    }
    fs.chmodSync(resolved, 0o755)
    console.log(`[hats] using local native CLI: ${resolved}`)
    nativeCliPath = resolved
    return nativeCliPath
  }

  const binaryName = getPlatformBinaryName()
  if (!binaryName) {
    console.warn(`[hats] native CLI not available for ${os.platform()}-${os.arch()}; skipping native drift checks`)
    return null
  }

  const downloadUrl = `${RELEASES_BASE}/${version}/${binaryName}`
  const cacheDir = path.join(process.cwd(), '.cache', 'deka-cli')
  fs.mkdirSync(cacheDir, { recursive: true })
  const binaryPath = path.join(cacheDir, binaryName)

  if (!fs.existsSync(binaryPath)) {
    console.log(`[hats] downloading native CLI ${downloadUrl}`)
    const res = await fetch(downloadUrl)
    if (!res.ok) {
      throw new Error(`Failed to download native CLI ${downloadUrl}: ${res.status}`)
    }
    const bytes = Buffer.from(await res.arrayBuffer())
    fs.writeFileSync(binaryPath, bytes)
  }

  fs.chmodSync(binaryPath, 0o755)

  // Verify the binary actually executes in this environment (glibc compatibility, etc.).
  try {
    execSync(`"${binaryPath}" --version`, {
      encoding: 'utf-8',
      timeout: 10000,
      stdio: ['ignore', 'pipe', 'pipe'],
    })
  } catch (err) {
    const stderr = String((err as { stderr?: string }).stderr ?? '')
    console.warn(`[hats] native CLI ${binaryPath} failed to run: ${stderr.trim()}`)
    console.warn('[hats] native drift detection disabled; falling back to wasm-only results')
    return null
  }

  nativeCliPath = binaryPath
  return binaryPath
}

export function nativeCliVersion(cliPath: string): string | undefined {
  // `deka [version x.y.z]` is written to stderr, same as preflight.mjs.
  const r = spawnSync(cliPath, ['--version'], { encoding: 'utf-8', timeout: 10_000 })
  return `${r.stdout ?? ''}${r.stderr ?? ''}`.match(/(\d+\.\d+\.\d+)/)?.[1]
}

function parseNativeDiagnostics(stderr: string): NativeRunResult['diagnostics'] {
  const diagnostics: NativeRunResult['diagnostics'] = []
  const lines = stderr.split('\n')

  let message: string | undefined
  let line: number | undefined
  let column: number | undefined

  for (let i = 0; i < lines.length; i++) {
    const current = lines[i]
    // Header line: ┌─ /path/to/file.ds:LINE:COLUMN
    const headerMatch = current.match(/^┌─\s+\S+:(\d+):(\d+)\s*$/)
    if (headerMatch) {
      line = Number(headerMatch[1])
      column = Number(headerMatch[2])
      continue
    }
    // Message line: │   ^ MESSAGE
    const messageMatch = current.match(/\^\s+(.+)$/)
    if (messageMatch) {
      message = messageMatch[1].trim()
      if (message) {
        diagnostics.push({ severity: 'error', message, line, column })
      }
      message = undefined
      line = undefined
      column = undefined
    }
  }

  // Fallback: if no rich diagnostic was parsed, treat the first non-empty,
  // non-bracketed line as a single-line diagnostic. This covers simple parser
  // errors like "Missing semicolon" or "DekaScript parameters require a type
  // annotation" that the native CLI emits without position annotations.
  if (diagnostics.length === 0) {
    const firstLine = lines.find((l) => {
      const trimmed = l.trim()
      return trimmed.length > 0 && !trimmed.startsWith('[') && !trimmed.startsWith('Validation') && !trimmed.startsWith('❌')
    })
    if (firstLine) {
      diagnostics.push({ severity: 'error', message: firstLine.trim() })
    }
  }

  return diagnostics
}

function createPrivateTempDir(): string {
  const prefix = path.join(os.tmpdir(), 'hats-native-run-')
  const dir = fs.mkdtempSync(prefix)
  fs.chmodSync(dir, 0o700)
  return dir
}

function removeTempDir(tmpDir: string): void {
  try {
    fs.rmSync(tmpDir, { recursive: true, force: true })
  } catch {
    // Best-effort cleanup; don't let temp-dir removal mask the real result.
  }
}

function writeProjectFiles(tmpDir: string, entryPath: string, source: string, files?: Record<string, string>): { inputPath: string; outputPath: string; isProject: boolean } {
  const isProject = files && Object.keys(files).length > 0
  const outputPath = path.join(tmpDir, 'test.js')

  if (!isProject) {
    const inputPath = path.join(tmpDir, 'test.ds')
    fs.writeFileSync(inputPath, source)
    return { inputPath, outputPath, isProject: false }
  }

  // Multi-file project: write all modules into the temp dir and mirror the
  // relative paths from the test fixture. Then copy the entry module to
  // main.ds so `deka transpile <dir> --bundle` has a discoverable entry point.
  fs.writeFileSync(path.join(tmpDir, entryPath), source)
  for (const [filePath, content] of Object.entries(files!)) {
    const fullPath = path.join(tmpDir, filePath)
    fs.mkdirSync(path.dirname(fullPath), { recursive: true })
    fs.writeFileSync(fullPath, content)
  }

  return { inputPath: tmpDir, outputPath, isProject: true }
}

function installPackages(
  cliPath: string,
  tmpDir: string,
  packages: string[]
): { ok: boolean; error?: string; stderr: string } {
  const cacheKey = packages.slice().sort().join('+')
  const cacheDir = path.join(process.cwd(), '.cache', 'deka-packages', cacheKey)
  const cachedLock = path.join(cacheDir, 'deka.lock')
  const cachedModules = path.join(cacheDir, 'php_modules')

  if (fs.existsSync(cachedLock) && fs.existsSync(cachedModules)) {
    fs.cpSync(cachedModules, path.join(tmpDir, 'php_modules'), { recursive: true })
    fs.copyFileSync(cachedLock, path.join(tmpDir, 'deka.lock'))
    return { ok: true, stderr: '' }
  }

  const spawned = spawnSync(cliPath, ['add', ...packages, '--yes'], {
    cwd: tmpDir,
    encoding: 'utf-8',
    timeout: 120000,
    env: { ...process.env, DEKA_SECURITY_NO_PROMPT: '1' },
  })
  const stderr = spawned.stderr ?? ''
  if (spawned.status !== 0 || spawned.error) {
    return {
      ok: false,
      error:
        spawned.error?.message ??
        stderr
          .split('\n')
          .map((line) => line.trim())
          .find((line) => line.length > 0) ??
        `deka add ${packages.join(' ')} failed`,
      stderr,
    }
  }

  fs.mkdirSync(cacheDir, { recursive: true })
  const modulesDir = path.join(tmpDir, 'php_modules')
  if (fs.existsSync(modulesDir)) {
    fs.cpSync(modulesDir, cachedModules, { recursive: true })
  }
  const lockPath = path.join(tmpDir, 'deka.lock')
  if (fs.existsSync(lockPath)) {
    fs.copyFileSync(lockPath, cachedLock)
  }
  return { ok: true, stderr }
}

export async function runNativeCli(
  cliPath: string,
  source: string,
  entryPath?: string,
  files?: Record<string, string>,
  options?: { dekaJson?: Record<string, unknown>; packages?: string[] }
): Promise<NativeRunResult> {
  const tmpDir = createPrivateTempDir()
  const packages = options?.packages ?? []

  try {
    const { isProject } = writeProjectFiles(tmpDir, entryPath ?? 'test.ds', source, files)

    fs.writeFileSync(path.join(tmpDir, 'deka.lock'), DEFAULT_DEKA_LOCK)
    const dekaJson =
      options?.dekaJson ?? (packages.length > 0 ? PACKAGE_DEKA_JSON : DEFAULT_DEKA_JSON)
    fs.writeFileSync(path.join(tmpDir, 'deka.json'), JSON.stringify(dekaJson, null, 2) + '\n')

    if (packages.length > 0) {
      const installed = installPackages(cliPath, tmpDir, packages)
      if (!installed.ok) {
        return {
          ok: false,
          stdout: '',
          stderr: installed.stderr,
          error: installed.error,
          transpileFailed: true,
          diagnostics: installed.error
            ? [{ severity: 'error', message: installed.error }]
            : [],
        }
      }
    }

    const entryRel = isProject ? `./${entryPath ?? 'main.ds'}` : './test.ds'

    const jsOut = path.join(tmpDir, 'captured.js')
    const transpiled = spawnSync(cliPath, ['transpile', entryRel, '--out', jsOut], {
      cwd: tmpDir,
      encoding: 'utf-8',
      timeout: 30000,
      env: { ...process.env, DEKA_SECURITY_NO_PROMPT: '1' },
    })
    const emittedJs =
      transpiled.status === 0 && fs.existsSync(jsOut)
        ? fs.readFileSync(jsOut, 'utf-8')
        : undefined

    const spawned = spawnSync(cliPath, ['run', entryRel], {
      cwd: tmpDir,
      encoding: 'utf-8',
      timeout: 30000,
      env: { ...process.env, DEKA_SECURITY_NO_PROMPT: '1' },
    })

    const stdout = spawned.stdout ?? ''
    const rawStderr = spawned.stderr ?? ''
    const stderr = rawStderr
      .split('\n')
      .filter((line) => !line.startsWith('[security]'))
      .join('\n')
    const failed = spawned.status !== 0 || spawned.error !== undefined
    const ranInIsolate = rawStderr.includes('Run failed:') || stdout.length > 0
    const diagnostics = failed ? parseNativeDiagnostics(stderr || rawStderr) : []
    const firstError =
      diagnostics[0]?.message ??
      stderr
        .split('\n')
        .map((line) => line.trim())
        .find((line) => line.length > 0) ??
      spawned.error?.message ??
      (failed ? 'deka run failed' : undefined)

    return {
      ok: !failed,
      stdout,
      stderr,
      error: failed ? firstError : undefined,
      transpileFailed: failed && !ranInIsolate,
      emittedJs,
      diagnostics,
    }
  } finally {
    removeTempDir(tmpDir)
  }
}
