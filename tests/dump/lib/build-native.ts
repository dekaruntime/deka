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
      write: ['.cache', 'php_modules', 'ds_modules'],
    },
    prompt: false,
  },
}

function sourceImportsIo(source: string, files?: Record<string, string>): boolean {
  const blobs = [source, ...Object.values(files ?? {})]
  return blobs.some((s) => /\bfrom\s+["']io["']/.test(s))
}

function packagesFor(source: string, files: Record<string, string> | undefined, declared?: string[]): string[] {
  const packages = [...(declared ?? [])]
  if (sourceImportsIo(source, files) && !packages.some((p) => p === 'io' || p === '@deka/io')) {
    packages.push('io')
  }
  return packages
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

  // Fallback: if no rich diagnostic was parsed, the CLI emitted the compact
  // form (`LINE:COL: /path/file.ds: message`). Collect EVERY such line — a
  // single failure can carry several diagnostics, and a fixture's expected
  // diagnostic may be any of them (e.g. generic-export-return-types-negative
  // expects the type-mismatch line, which is the second one). Previously only
  // the first line became a diagnostic, so later lines were invisible to
  // expectedDiagnosticContains.
  if (diagnostics.length === 0) {
    let sawCompact = false
    for (const l of lines) {
      const compactMatch = l.match(/^\s*\d+:\d+:\s+\S+:\s+(.+)$/)
      if (compactMatch) {
        sawCompact = true
        diagnostics.push({ severity: 'error', message: compactMatch[1].trim() })
      }
    }
    if (!sawCompact) {
      const firstLine = lines.find((l) => {
        const trimmed = l.trim()
        return trimmed.length > 0 && !trimmed.startsWith('[') && !trimmed.startsWith('Validation') && !trimmed.startsWith('❌')
      })
      if (firstLine) {
        diagnostics.push({ severity: 'error', message: firstLine.trim() })
      }
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

export interface NativeFormatResult {
  ok: boolean
  code?: string
  error?: string
}

/**
 * Format a DekaScript source with the native CLI (`deka fmt`). The dump
 * compares this against the wasm formatter's output for every fixture so a
 * host that starts rewriting source differently (deka#477) shows up as
 * divergence instead of silently shipping two formatters.
 */
export function formatDsWithNative(cliPath: string, source: string): NativeFormatResult {
  const tmpDir = createPrivateTempDir()
  try {
    const file = path.join(tmpDir, 'fmt-input.ds')
    fs.writeFileSync(file, source)
    const result = spawnSync(cliPath, ['fmt', file], {
      cwd: tmpDir,
      encoding: 'utf-8',
      timeout: 30000,
      env: { ...process.env, DEKA_SECURITY_NO_PROMPT: '1' },
    })
    if (result.status !== 0) {
      return {
        ok: false,
        error: (result.stderr ?? '').trim() || `deka fmt exited ${result.status}`,
      }
    }
    return { ok: true, code: fs.readFileSync(file, 'utf-8') }
  } finally {
    removeTempDir(tmpDir)
  }
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
    // Follow the fixture's extension — .dsx unlocks JSX in the compiler.
    const inputPath = path.join(tmpDir, entryPath.endsWith('.dsx') ? 'test.dsx' : 'test.ds')
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

// A cache hit restores ds_modules/ and deka.lock but not the manifest, so the
// fixture ended up with packages installed and never declared -- exactly the
// shape the project gate rejects (deka#403, deka#430). Derive the dependency
// block from what was actually restored, so it does not matter which runner
// filled the shared cache.
function declareRestoredModules(tmpDir: string) {
  const manifestPath = path.join(tmpDir, 'deka.json')
  const manifest = fs.existsSync(manifestPath)
    ? JSON.parse(fs.readFileSync(manifestPath, 'utf-8'))
    : {}
  const deps: Record<string, string> = { ...(manifest.dependencies ?? {}) }
  for (const modulesDir of ['ds_modules', 'php_modules']) {
    const scope = path.join(tmpDir, modulesDir, '@deka')
    if (!fs.existsSync(scope)) continue
    for (const name of fs.readdirSync(scope)) {
      const pkg = '@deka/' + name
      if (deps[pkg]) continue
      const pkgManifest = path.join(scope, name, 'deka.json')
      let version = '*'
      if (fs.existsSync(pkgManifest)) {
        try {
          version = JSON.parse(fs.readFileSync(pkgManifest, 'utf-8')).version ?? '*'
        } catch {
          version = '*'
        }
      }
      deps[pkg] = version
    }
  }
  manifest.dependencies = deps
  fs.writeFileSync(manifestPath, JSON.stringify(manifest, null, 2) + String.fromCharCode(10))
}

function restoreCachedModules(cacheDir: string, tmpDir: string) {
  for (const name of ['ds_modules', 'php_modules']) {
    const cached = path.join(cacheDir, name)
    if (fs.existsSync(cached)) {
      fs.cpSync(cached, path.join(tmpDir, name), { recursive: true })
    }
  }
}

function installPackages(
  cliPath: string,
  tmpDir: string,
  packages: string[]
): { ok: boolean; error?: string; stderr: string } {
  const cacheKey = packages.slice().sort().join('+')
  const cacheDir = path.join(process.cwd(), '.cache', 'deka-packages', cacheKey)
  const cachedLock = path.join(cacheDir, 'deka.lock')
  const hasCachedModules =
    fs.existsSync(path.join(cacheDir, 'ds_modules')) ||
    fs.existsSync(path.join(cacheDir, 'php_modules'))

  if (fs.existsSync(cachedLock) && hasCachedModules) {
    restoreCachedModules(cacheDir, tmpDir)
    fs.copyFileSync(cachedLock, path.join(tmpDir, 'deka.lock'))
    declareRestoredModules(tmpDir)
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
  for (const name of ['ds_modules', 'php_modules']) {
    const dir = path.join(tmpDir, name)
    if (fs.existsSync(dir)) {
      fs.cpSync(dir, path.join(cacheDir, name), { recursive: true })
    }
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
  const packages = packagesFor(source, files, options?.packages)

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

    const entryRel = isProject
      ? `./${entryPath ?? 'main.ds'}`
      : `./test${entryPath?.endsWith('.dsx') ? '.dsx' : '.ds'}`

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
