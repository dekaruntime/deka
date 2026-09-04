/**
 * Introspect Command - Inspect deka-runtime scheduler and isolates
 */

import { out } from '@tananetwork/stdio'

export interface IntrospectOptions {
  runtime?: string
  json?: boolean
}

interface TopOptions extends IntrospectOptions {
  sort?: string
  limit?: string
}

interface IsolateResponse {
  status: string
  metrics?: Record<string, unknown>
  error?: string
}

const DEFAULT_RUNTIME_URL = 'http://localhost:8530'

function resolveRuntimeUrl(runtime?: string) {
  return runtime?.trim() || DEFAULT_RUNTIME_URL
}

function joinUrl(base: string, path: string) {
  const url = new URL(base)
  url.pathname = path
  return url
}

async function fetchJson(url: URL, init?: RequestInit) {
  const response = await fetch(url.toString(), init)
  if (!response.ok) {
    const body = await response.text()
    throw new Error(`request failed (${response.status}): ${body || response.statusText}`)
  }
  return response.json()
}

export async function introspectTop(options: TopOptions = {}) {
  const runtime = resolveRuntimeUrl(options.runtime)
  const url = joinUrl(runtime, '/_deka/debug/top')
  if (options.sort) url.searchParams.set('sort', options.sort)
  if (options.limit) url.searchParams.set('limit', options.limit)

  try {
    const data = await fetchJson(url)
    if (options.json) {
      console.log(JSON.stringify(data, null, 2))
      return
    }

    const rows = (data as Array<Record<string, unknown>>).map((row) => ({
      id: String(row.isolate_id ?? ''),
      handler: String(row.handler_name ?? ''),
      worker: row.worker_id,
      cpu: typeof row.cpu_percent === 'number' ? row.cpu_percent.toFixed(1) : row.cpu_percent,
      heap_mb: typeof row.heap_used_bytes === 'number'
        ? (row.heap_used_bytes / (1024 * 1024)).toFixed(1)
        : row.heap_used_bytes,
      requests: row.total_requests,
      state: row.state ? JSON.stringify(row.state) : '',
    }))

    out.blank()
    console.table(rows)
    out.blank()
  } catch (error: unknown) {
    const message = error instanceof Error ? error.message : String(error)
    out.error('introspect', message)
  }
}

export async function introspectWorkers(options: IntrospectOptions = {}) {
  const runtime = resolveRuntimeUrl(options.runtime)
  const url = joinUrl(runtime, '/_deka/debug/workers')

  try {
    const data = await fetchJson(url)
    if (options.json) {
      console.log(JSON.stringify(data, null, 2))
      return
    }

    const rows = (data as Array<Record<string, unknown>>).map((row) => ({
      worker: row.worker_id,
      isolates: row.active_isolates,
      queued: row.queued_requests,
      requests: row.total_requests,
      avg_ms: typeof row.avg_latency_ms === 'number' ? row.avg_latency_ms.toFixed(2) : row.avg_latency_ms,
    }))

    out.blank()
    console.table(rows)
    out.blank()
  } catch (error: unknown) {
    const message = error instanceof Error ? error.message : String(error)
    out.error('introspect', message)
  }
}

export async function introspectInspect(handler: string, options: IntrospectOptions = {}) {
  const runtime = resolveRuntimeUrl(options.runtime)
  const url = joinUrl(runtime, `/_deka/debug/isolate/${encodeURIComponent(handler)}`)

  try {
    const data = await fetchJson(url) as IsolateResponse
    if (options.json) {
      console.log(JSON.stringify(data, null, 2))
      return
    }

    if (data.status !== 'ok' || !data.metrics) {
      out.error('introspect', data.error || 'isolate not found')
      return
    }

    out.blank()
    console.log(JSON.stringify(data.metrics, null, 2))
    out.blank()
  } catch (error: unknown) {
    const message = error instanceof Error ? error.message : String(error)
    out.error('introspect', message)
  }
}

export async function introspectKill(handler: string, options: IntrospectOptions = {}) {
  const runtime = resolveRuntimeUrl(options.runtime)
  const url = joinUrl(runtime, `/_deka/debug/isolate/${encodeURIComponent(handler)}`)

  try {
    const data = await fetchJson(url, { method: 'DELETE' }) as IsolateResponse
    if (data.status === 'ok') {
      out.status('introspect', `killed isolate ${handler}`, true)
      return
    }
    out.error('introspect', data.error || 'failed to kill isolate')
  } catch (error: unknown) {
    const message = error instanceof Error ? error.message : String(error)
    out.error('introspect', message)
  }
}
