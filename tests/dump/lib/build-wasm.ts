const MANIFEST_URL = 'https://dsc-wasm.deka.gg/latest/deka-compiler-artifact.json'

const LOCAL_WASM = typeof process !== 'undefined' ? process.env.DEKA_WASM : undefined

interface WasmExports {
  memory: WebAssembly.Memory
  deka_compiler_alloc: (size: number) => number
  deka_compiler_free: (ptr: number, size: number) => void
  deka_compiler_compile: (
    sourcePtr: number,
    sourceLen: number,
    filenamePtr: number,
    filenameLen: number,
    modePtr: number,
    modeLen: number
  ) => number
  deka_compiler_format_ds: (sourcePtr: number, sourceLen: number) => number
  deka_compiler_metadata?: () => number
  deka_compiler_project_new: () => number
  deka_compiler_project_write: (
    projectId: number,
    pathPtr: number,
    pathLen: number,
    sourcePtr: number,
    sourceLen: number
  ) => void
  deka_compiler_project_compile: (projectId: number) => number
  deka_compiler_project_free: (projectId: number) => void
  // Added for deka#497; absent from artifacts published before it.
  deka_compiler_project_set_module_base?: (
    projectId: number,
    basePtr: number,
    baseLen: number
  ) => void
}

export interface WasmCompilerMetadata {
  name: string
  version: string
  source_commit: string
}

export interface BuildCompileResult {
  ok: boolean
  js?: string
  error?: string
  diagnostics: Array<{
    severity: 'error' | 'warning' | 'info'
    message: string
    line?: number
    column?: number
  }>
}

export interface BuildFormatResult {
  ok: boolean
  code?: string
  error?: string
}

export interface WasmCompiler {
  exports: WasmExports
}

const textEncoder = new TextEncoder()
const textDecoder = new TextDecoder()

export async function loadWasmCompiler(): Promise<WasmCompiler> {
  if (LOCAL_WASM) {
    const fs = await import('node:fs/promises')
    const path = await import('node:path')
    const resolved = path.resolve(LOCAL_WASM)
    let bytes: Buffer
    try {
      bytes = await fs.readFile(resolved)
    } catch (err) {
      throw new Error(
        `DEKA_WASM is set to ${resolved} but that file could not be read: ${String(err)}`
      )
    }
    console.log(`[build-wasm] using local compiler: ${resolved} (${bytes.byteLength} bytes)`)
    const localModule = await WebAssembly.compile(new Uint8Array(bytes))
    const localInstance = await WebAssembly.instantiate(localModule, {})
    return { exports: localInstance.exports as unknown as WasmExports }
  }

  const manifestRes = await fetch(MANIFEST_URL)
  if (!manifestRes.ok) {
    throw new Error(`Failed to fetch compiler manifest: ${manifestRes.status}`)
  }
  const manifest = (await manifestRes.json()) as {
    artifact: { file: string; sha256: string }
  }

  const wasmUrl = new URL(manifest.artifact.file, MANIFEST_URL).toString()
  const wasmRes = await fetch(wasmUrl)
  if (!wasmRes.ok) {
    throw new Error(`Failed to fetch compiler wasm: ${wasmRes.status}`)
  }
  const bytes = await wasmRes.arrayBuffer()
  console.log(`[build-wasm] using published compiler: ${wasmUrl} (${bytes.byteLength} bytes)`)

  const wasmModule = await WebAssembly.compile(bytes)
  const instance = await WebAssembly.instantiate(wasmModule, {})
  return { exports: instance.exports as unknown as WasmExports }
}

function readWasmJson(exports: WasmExports, resultPtr: number): string {
  const resultView = new DataView(exports.memory.buffer)
  const jsonPtr = resultView.getUint32(resultPtr, true)
  const jsonLen = resultView.getUint32(resultPtr + 4, true)
  const jsonBytes = new Uint8Array(exports.memory.buffer, jsonPtr, jsonLen)
  const jsonText = textDecoder.decode(jsonBytes)
  exports.deka_compiler_free(resultPtr, 8 + jsonLen)
  return jsonText
}

/**
 * Identity of the loaded compiler, from the wasm bytes themselves.
 * Do not substitute the CDN sidecar: that is how v0.28.0 advertised 0.28.0
 * while deka_compiler_metadata() said 0.27.0 (deka#279).
 */
export function readCompilerMetadata(compiler: WasmCompiler): WasmCompilerMetadata {
  const metaFn = compiler.exports.deka_compiler_metadata
  if (typeof metaFn !== 'function') {
    throw new Error(
      'wasm exports no deka_compiler_metadata; cannot prove compiler identity. Rebuild from a current deka checkout.'
    )
  }
  const jsonText = readWasmJson(compiler.exports, metaFn())
  let parsed: Partial<WasmCompilerMetadata>
  try {
    parsed = JSON.parse(jsonText) as Partial<WasmCompilerMetadata>
  } catch {
    throw new Error(`deka_compiler_metadata returned invalid JSON: ${jsonText}`)
  }
  if (!parsed.version) {
    throw new Error(`deka_compiler_metadata missing version: ${jsonText}`)
  }
  return {
    name: typeof parsed.name === 'string' ? parsed.name : 'deka',
    version: parsed.version,
    source_commit: typeof parsed.source_commit === 'string' ? parsed.source_commit : 'unknown',
  }
}

function normalizeDiagnostics(value: unknown): BuildCompileResult['diagnostics'] {
  if (!Array.isArray(value)) return []
  return value.flatMap((diagnostic) => {
    if (!diagnostic || typeof diagnostic !== 'object') return []
    const raw = diagnostic as Record<string, unknown>
    if (typeof raw.message !== 'string') return []
    const severity =
      raw.severity === 'error' || raw.severity === 'warning' || raw.severity === 'info'
        ? raw.severity
        : 'info'
    return [
      {
        severity,
        message: raw.message,
        line: typeof raw.line === 'number' ? raw.line : undefined,
        column: typeof raw.column === 'number' ? raw.column : undefined,
      },
    ]
  })
}

export function compileWithWasm(
  compiler: WasmCompiler,
  source: string,
  filename: string,
  options?: { moduleBase?: string }
): BuildCompileResult {
  const exports = compiler.exports
  const allocate = exports.deka_compiler_alloc
  const free = exports.deka_compiler_free

  const sourceBytes = textEncoder.encode(source)
  const filenameBytes = textEncoder.encode(filename)
  const optionsBytes = textEncoder.encode(
    JSON.stringify({ mode: 'deka', moduleBase: options?.moduleBase })
  )

  const sourcePtr = allocate(sourceBytes.length)
  const filenamePtr = allocate(filenameBytes.length)
  const optionsPtr = allocate(optionsBytes.length)

  const memory = new Uint8Array(exports.memory.buffer)
  memory.set(sourceBytes, sourcePtr)
  memory.set(filenameBytes, filenamePtr)
  memory.set(optionsBytes, optionsPtr)

  const resultPtr = exports.deka_compiler_compile(
    sourcePtr,
    sourceBytes.length,
    filenamePtr,
    filenameBytes.length,
    optionsPtr,
    optionsBytes.length
  )

  const resultView = new DataView(exports.memory.buffer)
  const jsonPtr = resultView.getUint32(resultPtr, true)
  const jsonLen = resultView.getUint32(resultPtr + 4, true)

  const jsonBytes = new Uint8Array(exports.memory.buffer, jsonPtr, jsonLen)
  const jsonText = textDecoder.decode(jsonBytes)

  let parsed: Partial<BuildCompileResult> & { output?: { code?: string } }
  try {
    parsed = JSON.parse(jsonText) as Partial<BuildCompileResult>
  } catch {
    const error = `Compiler returned invalid JSON: ${jsonText}`
    free(resultPtr, 8 + jsonLen)
    free(sourcePtr, sourceBytes.length)
    free(filenamePtr, filenameBytes.length)
    free(optionsPtr, optionsBytes.length)
    return { ok: false, error, diagnostics: [] }
  }

  free(resultPtr, 8 + jsonLen)
  free(sourcePtr, sourceBytes.length)
  free(filenamePtr, filenameBytes.length)
  free(optionsPtr, optionsBytes.length)

  const diagnostics = normalizeDiagnostics(parsed.diagnostics)
  const error =
    parsed.error ?? diagnostics.find((d) => d.severity === 'error')?.message
  return {
    ok: parsed.ok ?? false,
    js: parsed.output?.code,
    error,
    diagnostics,
  }
}

export interface BuildCompileProjectResult {
  ok: boolean
  modules: Record<string, { code: string }>
  error?: string
  diagnostics: BuildCompileResult['diagnostics']
}

let warnedMissingProjectModuleBase = false

/**
 * Compile a multi-file project through the WASM project-mode ABI, using the
 * same local/published compiler the single-file path uses. `moduleBase` gives
 * project mode the same stdlib-shim resolution as single-file compiles: bare
 * specifiers like `io` are emitted as `<moduleBase>/io.mjs` URLs, which the
 * browser harness intercepts and serves from the vendored shims (deka#497).
 */
export function compileProjectWithWasm(
  compiler: WasmCompiler,
  files: Record<string, string>,
  options?: { moduleBase?: string }
): BuildCompileProjectResult {
  const exports = compiler.exports
  const allocate = exports.deka_compiler_alloc
  const free = exports.deka_compiler_free

  const writeString = (text: string): { ptr: number; len: number } => {
    const bytes = textEncoder.encode(text)
    const ptr = allocate(bytes.length)
    new Uint8Array(exports.memory.buffer).set(bytes, ptr)
    return { ptr, len: bytes.length }
  }

  const projectId = exports.deka_compiler_project_new()
  if (projectId === 0) {
    return {
      ok: false,
      modules: {},
      error: 'failed to create compiler project',
      diagnostics: [{ severity: 'error', message: 'failed to create compiler project' }],
    }
  }

  try {
    if (options?.moduleBase) {
      if (typeof exports.deka_compiler_project_set_module_base === 'function') {
        const base = writeString(options.moduleBase)
        exports.deka_compiler_project_set_module_base(projectId, base.ptr, base.len)
        free(base.ptr, base.len)
      } else if (!warnedMissingProjectModuleBase) {
        warnedMissingProjectModuleBase = true
        console.warn(
          '[build-wasm] compiler exports no deka_compiler_project_set_module_base; ' +
            'project-mode stdlib imports (io, …) will fail. Rebuild the wasm compiler from a checkout with deka#497.'
        )
      }
    }

    for (const [filePath, source] of Object.entries(files)) {
      const normalized = filePath.replace(/\\/g, '/').replace(/^\.\//, '')
      const path = writeString(normalized)
      const src = writeString(source)
      exports.deka_compiler_project_write(projectId, path.ptr, path.len, src.ptr, src.len)
      free(path.ptr, path.len)
      free(src.ptr, src.len)
    }

    const resultPtr = exports.deka_compiler_project_compile(projectId)
    const jsonText = readWasmJson(exports, resultPtr)

    let parsed: { ok?: boolean; modules?: Record<string, { code: string }>; diagnostics?: unknown }
    try {
      parsed = JSON.parse(jsonText)
    } catch {
      return {
        ok: false,
        modules: {},
        error: `Compiler returned invalid JSON: ${jsonText}`,
        diagnostics: [],
      }
    }

    const diagnostics = normalizeDiagnostics(parsed.diagnostics)
    const modules = parsed.modules ?? {}
    const ok = (parsed.ok ?? false) && Object.keys(modules).length > 0
    return {
      ok,
      modules,
      error: ok ? undefined : diagnostics.find((d) => d.severity === 'error')?.message ?? 'project compilation failed',
      diagnostics,
    }
  } finally {
    exports.deka_compiler_project_free(projectId)
  }
}

export function formatDsWithWasm(compiler: WasmCompiler, source: string): BuildFormatResult {
  const exports = compiler.exports
  const allocate = exports.deka_compiler_alloc
  const free = exports.deka_compiler_free

  const sourceBytes = textEncoder.encode(source)
  const sourcePtr = allocate(sourceBytes.length)
  const memory = new Uint8Array(exports.memory.buffer)
  memory.set(sourceBytes, sourcePtr)

  const resultPtr = exports.deka_compiler_format_ds(sourcePtr, sourceBytes.length)
  const resultView = new DataView(exports.memory.buffer)
  const jsonPtr = resultView.getUint32(resultPtr, true)
  const jsonLen = resultView.getUint32(resultPtr + 4, true)

  const jsonBytes = new Uint8Array(exports.memory.buffer, jsonPtr, jsonLen)
  const jsonText = textDecoder.decode(jsonBytes)

  let parsed: Partial<BuildFormatResult> & { output?: { code?: string }; diagnostics?: unknown }
  try {
    parsed = JSON.parse(jsonText) as Partial<BuildFormatResult>
  } catch {
    free(resultPtr, 8 + jsonLen)
    free(sourcePtr, sourceBytes.length)
    return { ok: false, error: `Formatter returned invalid JSON: ${jsonText}` }
  }

  free(resultPtr, 8 + jsonLen)
  free(sourcePtr, sourceBytes.length)

  const diagnostics = normalizeDiagnostics(parsed.diagnostics)
  const error =
    parsed.error ?? diagnostics.find((d) => d.severity === 'error')?.message
  return {
    ok: parsed.ok ?? false,
    code: parsed.output?.code,
    error,
  }
}
