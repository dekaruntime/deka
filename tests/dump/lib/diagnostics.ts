export type HostDiagnostic = {
  severity: 'error' | 'warning' | 'info'
  message: string
  line?: number
  column?: number
}

// Compact diagnostic line emitted by the CLI. Two shapes occur in one run and
// BOTH must match: the first diagnostic of a file carries a `[transpile] `
// prefix, subsequent ones do not. The previous pattern anchored on `\\d+:\\d+:`
// with no optional prefix, so it silently dropped the *primary* diagnostic of
// every failure — 388 of them across a release pack — while still matching the
// follow-ons. That is what made the conformance report show 358 host
// divergences where only 16 exist (deka#739).
//
// The file-path segment is optional because not every diagnostic carries one.
//   [transpile] 6:1: /tmp/test.ds: async function must return Promise<T>
//   10:6: /tmp/test.ds: `await` expected Promise<T>, found type `number`
//   2:6: leading-zero octal-style integers are not allowed
const COMPACT_DIAGNOSTIC = /^\s*(?:\[transpile\]\s+)?(\d+):(\d+):\s+(?:\S+?\.dsx?:\s+)?(.+)$/

// Project-mode wasm sometimes prefixes each message with `file.ds: ` and, when
// several errors exist, concatenates them with newlines into one ABI slot
// (code: "emitter", span 1:1). Native stderr is one compact line per error.
// Strip the prefix so both channels share a message.
const FILE_PREFIX = /^(?:\S+\.dsx?:\s+)/

// A wrapper the runtime used to prepend ahead of dsc's real output. Kept as an
// exclusion so old packs and any other caller that still wraps cannot have the
// wrapper recorded as if it were the diagnostic.
const NON_DIAGNOSTIC_PREAMBLE = /^dsc transpile failed\b/

export function normalizeDiagnosticMessage(message: string): string {
  return message.replace(/^(?:\S+\.dsx?:\s+)+/, '').trim()
}

function isFilePrefixedLine(line: string): boolean {
  return FILE_PREFIX.test(line.trim())
}

/**
 * Split a wasm diagnostic message that packed several errors into one string.
 * A single file-prefixed line stays one diagnostic (prefix stripped later).
 */
export function expandDiagnosticMessage(message: string): string[] {
  const lines = message.split('\n').map((line) => line.trim()).filter(Boolean)
  if (lines.length > 1 && lines.every(isFilePrefixedLine)) {
    return lines
  }
  return [message]
}

export function parseNativeDiagnostics(stderr: string): HostDiagnostic[] {
  const diagnostics: HostDiagnostic[] = []
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
        diagnostics.push({
          severity: 'error',
          message: normalizeDiagnosticMessage(message),
          line,
          column,
        })
      }
      message = undefined
      line = undefined
      column = undefined
    }
  }

  // Fallback: if no rich diagnostic was parsed, the CLI emitted the compact
  // form (`LINE:COL: /path/file.ds: message`). Collect EVERY such line — a
  // single failure can carry several diagnostics, and a fixture's expected
  // diagnostic may be any of them (e.g. a type-mismatch fixture expecting the
  // second of two emitted diagnostics). Previously only the first line became
  // a diagnostic, so later lines were invisible to
  // expectedDiagnosticContains.
  if (diagnostics.length === 0) {
    let sawCompact = false
    for (const l of lines) {
      const compactMatch = l.match(COMPACT_DIAGNOSTIC)
      if (compactMatch) {
        sawCompact = true
        diagnostics.push({
          severity: 'error',
          message: normalizeDiagnosticMessage(compactMatch[3].trim()),
          line: Number(compactMatch[1]),
          column: Number(compactMatch[2]),
        })
      }
    }
    if (!sawCompact) {
      const firstLine = lines.find((l) => {
        const trimmed = l.trim()
        return (
          trimmed.length > 0 &&
          !trimmed.startsWith('[') &&
          !trimmed.startsWith('Validation') &&
          !trimmed.startsWith('❌') &&
          !NON_DIAGNOSTIC_PREAMBLE.test(trimmed)
        )
      })
      if (firstLine) {
        diagnostics.push({
          severity: 'error',
          message: normalizeDiagnosticMessage(firstLine.trim()),
        })
      }
    }
  }

  return diagnostics
}

function diagnosticPosition(
  raw: Record<string, unknown>,
  shortName: 'line' | 'column',
  longName: 'start_line' | 'start_column',
): number | undefined {
  if (typeof raw[shortName] === 'number') return raw[shortName]
  if (typeof raw[longName] === 'number') return raw[longName]
  return undefined
}

/**
 * Map the dsc wasm ABI `diagnostics` array into the host diagnostic shape.
 * Project-mode compiles can return one concatenated emitter diagnostic; expand
 * that here so comparison always sees the same SET native parsed from stderr.
 */
export function normalizeDiagnostics(value: unknown): HostDiagnostic[] {
  if (!Array.isArray(value)) return []
  return value.flatMap((diagnostic) => {
    if (!diagnostic || typeof diagnostic !== 'object') return []
    const raw = diagnostic as Record<string, unknown>
    if (typeof raw.message !== 'string') return []
    const severity =
      raw.severity === 'error' || raw.severity === 'warning' || raw.severity === 'info'
        ? raw.severity
        : 'info'
    const line = diagnosticPosition(raw, 'line', 'start_line')
    const column = diagnosticPosition(raw, 'column', 'start_column')
    const parts = expandDiagnosticMessage(raw.message)
    const concatenated = parts.length > 1
    return parts.map((part) => ({
      severity,
      message: normalizeDiagnosticMessage(part),
      ...(concatenated ? {} : { line, column }),
    }))
  })
}

function diagnosticKey(diagnostic: HostDiagnostic): string {
  return `${diagnostic.severity}\0${normalizeDiagnosticMessage(diagnostic.message)}`
}

/**
 * Compare the failure SIGNATURE: the SET of error diagnostics. Order and
 * (when project-mode wasm collapses spans to 1:1) positions are not part of
 * the signature. Info/warning notes (wasm summon unverified, etc.) stay on
 * the recorded result but do not count as host divergence.
 */
export function diagnosticsAgree(native: HostDiagnostic[], wasm: HostDiagnostic[]): boolean {
  const left = native.filter((d) => d.severity === 'error').map(diagnosticKey).sort()
  const right = wasm.filter((d) => d.severity === 'error').map(diagnosticKey).sort()
  if (left.length !== right.length) return false
  return left.every((key, index) => key === right[index])
}
