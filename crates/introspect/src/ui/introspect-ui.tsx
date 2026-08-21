import React, { useEffect, useMemo, useState } from 'react'
import { spawnSync } from 'child_process'
import { Box, Text, useApp, useInput, useStdout } from 'ink'
import { withFullScreen } from './fullscreen'

type RuntimeStats = {
  config?: {
    num_workers?: number
    max_isolates_per_worker?: number
    idle_timeout_secs?: number
    metrics_enabled?: boolean
    code_cache_enabled?: boolean
    request_timeout_ms?: number
    queue_timeout_ms?: number
    scheduler?: string
    introspect_profiling?: boolean
  }
  metrics?: {
    total_requests?: number
    cache_hits?: number
    cache_misses?: number
    cache_hit_rate?: number
    evictions?: number
  }
}

type WorkerStats = {
  worker_id: number
  active_isolates: number
  queued_requests: number
  total_requests: number
  avg_latency_ms: number
}

type RequestTrace = {
  id: string
  handler_name: string
  isolate_id: string
  worker_id: number
  started_at_ms: number
  state: {
    Executing?: unknown
    Completed?: { duration_ms: number }
    Failed?: { error: string; duration_ms: number }
    QueueTimeout?: { waited_ms: number }
  }
  op_timings?: Array<{
    name: string
    count: number
    total_ms: number
    avg_ms: number
  }>
  queue_wait_ms?: number
  warm_time_us?: number
  total_time_us?: number
  heap_before_bytes?: number
  heap_after_bytes?: number
  heap_delta_bytes?: number
  response_status?: number
  response_body?: string | null
}

type KnownRequest = {
  id: string
  handler: string
  isolateId: string
  firstSeen: number
  lastSeen: number
  trace?: RequestTrace
}

type ReplLine = {
  text: string
  color?: 'red' | 'yellow'
}

const REFRESH_MS = 1000
const MAX_REPL_LINES = 500

function fmtMs(value?: number) {
  if (value === undefined) return '-'
  return value.toFixed(2)
}

function fmtPct(value?: number) {
  if (value === undefined) return '-'
  return value.toFixed(1)
}

function fmtBytes(value?: number) {
  if (!value) return '0'
  const mb = value / (1024 * 1024)
  return `${mb.toFixed(1)} MB`
}

function fmtSignedBytes(value?: number) {
  if (!value) return '0'
  const sign = value < 0 ? '-' : ''
  const mb = Math.abs(value) / (1024 * 1024)
  return `${sign}${mb.toFixed(1)} MB`
}

function truncate(text: string | undefined, max: number) {
  if (!text) return ''
  if (text.length <= max) return text
  return text.slice(0, Math.max(0, max - 3)) + '...'
}

async function fetchJson<T>(url: string): Promise<T> {
  const res = await fetch(url)
  if (!res.ok) {
    const body = await res.text()
    throw new Error(body || res.statusText)
  }
  return res.json() as Promise<T>
}

function joinUrl(base: string, path: string) {
  const url = new URL(base)
  url.pathname = path
  return url
}

function formatState(state: unknown) {
  if (!state) return 'unknown'
  if (typeof state === 'string') return state
  return JSON.stringify(state)
}

function formatRequestState(state: RequestTrace['state']) {
  if (state.Completed) return `done ${state.Completed.duration_ms}ms`
  if (state.Failed) return `fail ${state.Failed.duration_ms}ms`
  if (state.QueueTimeout) return `queue ${state.QueueTimeout.waited_ms}ms`
  return 'running'
}

function formatClockTime(ms?: number) {
  if (!ms) return '--:--:--'
  const date = new Date(ms)
  const hh = String(date.getHours()).padStart(2, '0')
  const mm = String(date.getMinutes()).padStart(2, '0')
  const ss = String(date.getSeconds()).padStart(2, '0')
  return `${hh}:${mm}:${ss}`
}

function sanitizeErrorLine(line: string) {
  const replaced = line
    .replaceAll('❌', 'ERROR')
    .replaceAll('💡', 'Hint')
    .trimEnd()
  return replaced
    .split('')
    .filter((ch) => ch.charCodeAt(0) <= 0x7f)
    .join('')
}

function copyToClipboard(text: string): { ok: boolean; message?: string } {
  const platform = process.platform
  const command =
    platform === 'darwin'
      ? 'pbcopy'
      : platform === 'win32'
        ? 'clip'
        : 'xclip'
  const args = platform === 'linux' ? ['-selection', 'clipboard'] : []
  const result = spawnSync(command, args, { input: text })
  if (result.error) {
    return { ok: false, message: result.error.message }
  }
  if (result.status !== 0) {
    return { ok: false, message: `clipboard command failed (${result.status ?? 'unknown'})` }
  }
  return { ok: true }
}

function detailText(
  selected: { type: 'runtime' } | { type: 'request'; item: KnownRequest } | undefined,
  stats: RuntimeStats | null,
  workers: WorkerStats[],
) {
  if (!selected || selected.type === 'runtime') {
    const totalRequests = stats?.metrics?.total_requests ?? 0
    const cacheHitRate = stats?.metrics?.cache_hit_rate ?? 0
    const evictions = stats?.metrics?.evictions ?? 0
    const activeIsolates = workers.reduce((sum, w) => sum + w.active_isolates, 0)
    const queued = workers.reduce((sum, w) => sum + w.queued_requests, 0)
    const avgLatency = workers.length
      ? workers.reduce((sum, w) => sum + w.avg_latency_ms, 0) / workers.length
      : 0
    const totalWorkers = stats?.config?.num_workers ?? workers.length
    const scheduler = stats?.config?.scheduler ?? 'n/a'
    const requestTimeout = stats?.config?.request_timeout_ms ?? 0
    const queueTimeout = stats?.config?.queue_timeout_ms ?? 0

    return [
      'runtime',
      `workers: ${totalWorkers}`,
      `requests: ${totalRequests}`,
      `cache hit rate: ${(cacheHitRate * 100).toFixed(1)}%`,
      `evictions: ${evictions}`,
      `active isolates: ${activeIsolates}`,
      `queued: ${queued}`,
      `avg latency: ${avgLatency.toFixed(2)} ms`,
      `scheduler: ${scheduler}`,
      `request timeout: ${requestTimeout} ms`,
      `queue timeout: ${queueTimeout} ms`,
    ].join('\n')
  }

  const trace = selected.item.trace
  if (!trace) {
    return `${selected.item.id}\nno request trace yet`
  }

  const totalMs = trace.total_time_us ? trace.total_time_us / 1000 : undefined
  const warmMs = trace.warm_time_us ? trace.warm_time_us / 1000 : undefined
  const queueMs = trace.queue_wait_ms ?? 0

  const lines = [
    trace.id,
    `handler: ${trace.handler_name}`,
    `isolate: ${trace.isolate_id || 'n/a'}`,
    `worker: ${trace.worker_id}`,
    `state: ${formatRequestState(trace.state)}`,
    `started: ${formatClockTime(trace.started_at_ms)}`,
    `queue: ${queueMs} ms`,
    `warm: ${warmMs ? warmMs.toFixed(2) : '0.00'} ms`,
    `total: ${totalMs ? totalMs.toFixed(2) : '0.00'} ms`,
  ]

  const ops = trace.op_timings ?? []
  if (ops.length > 0) {
    lines.push('ops:')
    for (const op of ops) {
      const pct = totalMs ? ` (${((op.total_ms / totalMs) * 100).toFixed(1)}%)` : ''
      lines.push(`${op.name} ${op.count}x ${op.total_ms.toFixed(2)}ms avg ${op.avg_ms.toFixed(2)}ms${pct}`)
    }
  }

  const heapBefore = trace.heap_before_bytes ?? 0
  const heapAfter = trace.heap_after_bytes ?? 0
  const heapDelta = trace.heap_delta_bytes ?? 0
  const responseStatus = trace.response_status
  const responseBody = trace.response_body ?? undefined
  lines.push(`heap: ${fmtBytes(heapAfter)} (delta ${fmtSignedBytes(heapDelta)}; before ${fmtBytes(heapBefore)})`)

  if (responseStatus !== undefined) {
    lines.push(`response: ${responseStatus}`)
    if (responseStatus >= 400 && responseBody) {
      lines.push('response body:')
      for (const line of responseBody.split('\n')) {
        lines.push(truncate(line, detailWidth))
      }
    }
  }

  const errorMessage = trace.state?.Failed?.error
  const errorLines = errorMessage ? errorMessage.split('\n') : []
  if (errorMessage) {
    lines.push(`error: ${errorMessage}`)
  }

  return lines.join('\n')
}

function detailEntries(
  selected: { type: 'runtime' } | { type: 'request'; item: KnownRequest } | undefined,
  stats: RuntimeStats | null,
  workers: WorkerStats[],
): Array<{ text: string; isError: boolean }> {
  if (!selected || selected.type === 'runtime') {
    const totalRequests = stats?.metrics?.total_requests ?? 0
    const cacheHitRate = stats?.metrics?.cache_hit_rate ?? 0
    const evictions = stats?.metrics?.evictions ?? 0
    const activeIsolates = workers.reduce((sum, w) => sum + w.active_isolates, 0)
    const queued = workers.reduce((sum, w) => sum + w.queued_requests, 0)
    const avgLatency = workers.length
      ? workers.reduce((sum, w) => sum + w.avg_latency_ms, 0) / workers.length
      : 0
    const totalWorkers = stats?.config?.num_workers ?? workers.length
    const scheduler = stats?.config?.scheduler ?? 'n/a'
    const requestTimeout = stats?.config?.request_timeout_ms ?? 0
    const queueTimeout = stats?.config?.queue_timeout_ms ?? 0

    return [
      { text: 'runtime', isError: false },
      { text: `workers: ${totalWorkers}`, isError: false },
      { text: `requests: ${totalRequests}`, isError: false },
      { text: `cache hit rate: ${(cacheHitRate * 100).toFixed(1)}%`, isError: false },
      { text: `evictions: ${evictions}`, isError: false },
      { text: `active isolates: ${activeIsolates}`, isError: false },
      { text: `queued: ${queued}`, isError: false },
      { text: `avg latency: ${avgLatency.toFixed(2)} ms`, isError: false },
      { text: `scheduler: ${scheduler}`, isError: false },
      { text: `request timeout: ${requestTimeout} ms`, isError: false },
      { text: `queue timeout: ${queueTimeout} ms`, isError: false },
    ]
  }

  const trace = selected.item.trace
  if (!trace) {
    return [
      { text: selected.item.id, isError: false },
      { text: 'no request trace yet', isError: false },
    ]
  }

  const ops = trace.op_timings ?? []
  const totalMs = trace.total_time_us ? trace.total_time_us / 1000 : undefined
  const warmMs = trace.warm_time_us ? trace.warm_time_us / 1000 : undefined
  const queueMs = trace.queue_wait_ms ?? 0
  const heapBefore = trace.heap_before_bytes ?? 0
  const heapAfter = trace.heap_after_bytes ?? 0
  const heapDelta = trace.heap_delta_bytes ?? 0
  const responseStatus = trace.response_status
  const responseBody = trace.response_body ?? undefined
  const errorMessage = trace.state?.Failed?.error
  const errorLines = errorMessage ? errorMessage.split('\n') : []

  const entries: Array<{ text: string; isError: boolean }> = [
    { text: trace.id, isError: false },
    { text: `handler: ${trace.handler_name}`, isError: false },
    { text: `isolate: ${trace.isolate_id || 'n/a'}`, isError: false },
    { text: `worker: ${trace.worker_id}`, isError: false },
    { text: `state: ${formatRequestState(trace.state)}`, isError: false },
    { text: `started: ${formatClockTime(trace.started_at_ms)}`, isError: false },
  ]

  if (errorLines.length > 0) {
    entries.push({ text: 'error:', isError: true })
    for (const line of errorLines) {
      entries.push({ text: sanitizeErrorLine(line), isError: true })
    }
  }

  entries.push({ text: `queue: ${queueMs} ms`, isError: false })
  entries.push({ text: `warm: ${warmMs ? warmMs.toFixed(2) : '0.00'} ms`, isError: false })
  entries.push({ text: `total: ${totalMs ? totalMs.toFixed(2) : '0.00'} ms`, isError: false })
  entries.push({
    text: `heap: ${fmtBytes(heapAfter)} (delta ${fmtSignedBytes(heapDelta)}; before ${fmtBytes(heapBefore)})`,
    isError: false,
  })

  if (responseStatus !== undefined) {
    entries.push({ text: `response: ${responseStatus}`, isError: responseStatus >= 400 })
    if (responseStatus >= 400 && responseBody) {
      entries.push({ text: 'response body:', isError: true })
      for (const line of responseBody.split('\n')) {
        entries.push({ text: line, isError: true })
      }
    }
  }

  if (ops.length > 0) {
    entries.push({ text: 'ops:', isError: false })
    for (const op of ops) {
      const pct = totalMs ? ` (${((op.total_ms / totalMs) * 100).toFixed(1)}%)` : ''
      entries.push({
        text: `${op.name} ${op.count}x ${op.total_ms.toFixed(2)}ms avg ${op.avg_ms.toFixed(2)}ms${pct}`,
        isError: false,
      })
    }
  }

  return entries
}

function RuntimeView({
  stats,
  workers,
  maxWidth,
  maxLines,
}: {
  stats: RuntimeStats | null
  workers: WorkerStats[]
  maxWidth: number
  maxLines: number
}) {
  const totalRequests = stats?.metrics?.total_requests ?? 0
  const cacheHitRate = stats?.metrics?.cache_hit_rate ?? 0
  const evictions = stats?.metrics?.evictions ?? 0
  const activeIsolates = workers.reduce((sum, w) => sum + w.active_isolates, 0)
  const queued = workers.reduce((sum, w) => sum + w.queued_requests, 0)
  const avgLatency = workers.length
    ? workers.reduce((sum, w) => sum + w.avg_latency_ms, 0) / workers.length
    : 0
  const totalWorkers = stats?.config?.num_workers ?? workers.length

  const lines = [
    'runtime',
    `workers: ${totalWorkers}`,
    `requests: ${totalRequests}`,
    `cache hit rate: ${(cacheHitRate * 100).toFixed(1)}%`,
    `evictions: ${evictions}`,
    `active isolates: ${activeIsolates}`,
    `queued: ${queued}`,
    `avg latency: ${avgLatency.toFixed(2)} ms`,
    `scheduler: ${stats?.config?.scheduler ?? 'n/a'}`,
    `request timeout: ${stats?.config?.request_timeout_ms ?? 0} ms`,
    `queue timeout: ${stats?.config?.queue_timeout_ms ?? 0} ms`,
    `profiling: ${stats?.config?.introspect_profiling === false ? 'off' : 'on'}`,
  ]

  return (
    <Box flexDirection="column">
      {lines.slice(0, maxLines).map((line, idx) => (
        <Text key={`runtime-${idx}`} wrap="truncate">
          {truncate(line, maxWidth)}
        </Text>
      ))}
    </Box>
  )
}

function IntrospectApp({ runtime, archive }: { runtime: string; archive: boolean }) {
  const { exit } = useApp()
  const [stats, setStats] = useState<RuntimeStats | null>(null)
  const [workers, setWorkers] = useState<WorkerStats[]>([])
  const [requests, setRequests] = useState<Map<string, KnownRequest>>(new Map())
  const [selectedIndex, setSelectedIndex] = useState(0)
  const [focus, setFocus] = useState<'list' | 'detail' | 'repl'>('list')
  const [detailScroll, setDetailScroll] = useState(0)
  const [replInput, setReplInput] = useState('')
  const [replLines, setReplLines] = useState<ReplLine[]>([])
  const [replScroll, setReplScroll] = useState(0)
  const [error, setError] = useState<string | null>(null)
  const [notice, setNotice] = useState<string | null>(null)
  const { stdout } = useStdout()
  const cols = stdout?.columns ?? 80
  const rows = stdout?.rows ?? 24

  const listEntries = useMemo(() => {
    const items = Array.from(requests.values())
      .sort((a, b) => b.firstSeen - a.firstSeen)
    return [{ type: 'runtime' as const }, ...items.map((item) => ({ type: 'request' as const, item }))]
  }, [requests])

  const leftWidth = Math.max(24, Math.floor(cols * 0.33))
  const rightWidth = cols - leftWidth - 2
  const detailWidth = Math.max(10, rightWidth - 2)
  const listBoxHeight = rows - 3
  const rightHeight = listBoxHeight
  const detailBoxHeight = Math.max(6, Math.floor(rightHeight * 0.65))
  const replBoxHeight = Math.max(5, rightHeight - detailBoxHeight)
  const listContentHeight = Math.max(1, listBoxHeight - 2)
  const detailLines = Math.max(1, detailBoxHeight - 3)
  const replOutputLines = Math.max(1, replBoxHeight - 4)

  const selected = listEntries[selectedIndex]
  const requestEntries = listEntries.slice(1)
  const maxRequestRows = Math.max(0, listContentHeight - 1)
  const selectedRequestIndex = Math.max(0, selectedIndex - 1)
  const maxWindowStart = Math.max(0, requestEntries.length - maxRequestRows)
  const windowStart = Math.min(
    Math.max(0, selectedRequestIndex - (maxRequestRows - 1)),
    maxWindowStart,
  )
  const visibleRequests = requestEntries.slice(windowStart, windowStart + maxRequestRows)
  const detailAllEntries = detailEntries(selected, stats, workers)
  const maxDetailScroll = Math.max(0, detailAllEntries.length - detailLines)
  const detailScrollTop = Math.min(detailScroll, maxDetailScroll)
  const detailVisible = detailAllEntries.slice(detailScrollTop, detailScrollTop + detailLines)
  const maxReplScroll = Math.max(0, replLines.length - replOutputLines)
  const replScrollTop = Math.min(replScroll, maxReplScroll)
  const replVisible = replLines.slice(replScrollTop, replScrollTop + replOutputLines)

  const moveFocus = (next: 'list' | 'detail' | 'repl') => setFocus(next)

  useInput((input, key) => {
    const isMac = process.platform === 'darwin'
    const modifierPressed = isMac ? key.meta || key.shift : key.ctrl
    const normalized = input.toLowerCase()

    if (normalized === 'q' && modifierPressed) exit()

    if (input === '\t') {
      setFocus((prev) => {
        if (prev === 'list') return 'detail'
        if (prev === 'detail') return 'repl'
        return 'list'
      })
    }

    if (modifierPressed && key.leftArrow) {
      moveFocus('list')
    }
    if (modifierPressed && key.rightArrow && focus === 'list') {
      moveFocus('detail')
    }
    if (modifierPressed && key.downArrow) {
      setFocus((prev) => {
        if (prev === 'list') return 'detail'
        if (prev === 'detail') return 'repl'
        return 'repl'
      })
      return
    }
    if (modifierPressed && key.upArrow) {
      setFocus((prev) => {
        if (prev === 'repl') return 'detail'
        if (prev === 'detail') return 'list'
        return 'list'
      })
      return
    }

    if (focus !== 'repl') {
      if (normalized === 'y' && modifierPressed) {
        const text = detailText(selected, stats, workers)
        const result = copyToClipboard(text)
        setNotice(result.ok ? 'copied details' : `copy failed: ${result.message ?? 'unknown error'}`)
      }
      if (normalized === 'r') {
        moveFocus('repl')
      }
      if (normalized === 't' && modifierPressed) {
        setSelectedIndex(0)
        moveFocus('list')
      }
    }

    if (focus === 'list') {
      if (key.upArrow) {
        setSelectedIndex((prev) => Math.max(0, prev - 1))
      }
      if (key.downArrow) {
        setSelectedIndex((prev) => Math.min(listEntries.length - 1, prev + 1))
      }
      return
    }

    if (focus === 'detail') {
      if (key.upArrow) {
        setDetailScroll((prev) => Math.max(0, prev - 1))
      }
      if (key.downArrow) {
        setDetailScroll((prev) => prev + 1)
      }
      return
    }

    if (focus === 'repl') {
      if (key.upArrow) {
        setReplScroll((prev) => Math.max(0, prev - 1))
      }
      if (key.downArrow) {
        setReplScroll((prev) => prev + 1)
      }
      if (key.return) {
        const trimmed = replInput.trim()
        if (trimmed.length > 0) {
          void runReplCommand(trimmed)
        }
        setReplInput('')
      } else if (key.backspace || key.delete) {
        setReplInput((prev) => prev.slice(0, -1))
      } else if (input && !key.ctrl && !key.meta) {
        setReplInput((prev) => prev + input)
      }
    }
  })

  useEffect(() => {
    setDetailScroll(0)
  }, [selectedIndex])

  useEffect(() => {
    let cancelled = false
    const tick = async () => {
      try {
        const requestsUrl = joinUrl(runtime, '/_deka/debug/requests')
        requestsUrl.searchParams.set('limit', '200')
        if (archive) requestsUrl.searchParams.set('archive', 'true')

        const [statsData, workersData, requestData] = await Promise.all([
          fetchJson<RuntimeStats>(joinUrl(runtime, '/_deka/stats').toString()),
          fetchJson<WorkerStats[]>(joinUrl(runtime, '/_deka/debug/workers').toString()),
          fetchJson<RequestTrace[]>(requestsUrl.toString()),
        ])

        if (cancelled) return

        setStats(statsData)
        setWorkers(workersData)
        setRequests((prev) => {
          const next = new Map(prev)
          const now = Date.now()
          for (const trace of requestData) {
            const key = trace.id
            const existing = next.get(key)
            if (existing) {
              existing.lastSeen = now
              existing.trace = trace
              next.set(key, existing)
            } else {
              next.set(key, {
                id: trace.id,
                isolateId: trace.isolate_id,
                handler: trace.handler_name,
                firstSeen: now,
                lastSeen: now,
                trace,
              })
            }
          }
          return next
        })
        setError(null)
      } catch (err) {
        if (cancelled) return
        const message = err instanceof Error ? err.message : String(err)
        setError(message)
      }
    }

    tick()
    const timer = setInterval(tick, REFRESH_MS)
    return () => {
      cancelled = true
      clearInterval(timer)
    }
  }, [runtime])

  return (
    <Box flexDirection="column">
      <Box>
        <Text>deka introspect - {runtime}{archive ? ' (archive)' : ''}</Text>
        {error && <Text color="red">{`  error: ${error}`}</Text>}
      </Box>
      <Box flexDirection="row" height={listBoxHeight}>
        <Box
          flexDirection="column"
          width={leftWidth}
          borderStyle="round"
          borderColor={focus === 'list' ? 'green' : 'gray'}
        >
          <Text bold={focus === 'list'}>list</Text>
          {listEntries[0] && (
            <Text key="runtime" color={selectedIndex === 0 ? 'green' : undefined}>
              {selectedIndex === 0 ? '> ' : '  '}runtime
            </Text>
          )}
          {visibleRequests.map((entry, idx) => {
            const actualIndex = idx + 1 + windowStart
            const isSelected = actualIndex === selectedIndex
            if (entry.type !== 'request') return null
            const request = entry.item
            const isolateLabel = request.isolateId || 'pending'
            const shortId = isolateLabel.startsWith('isolate_')
              ? isolateLabel.slice('isolate_'.length)
              : isolateLabel
            const timeLabel = request.trace
              ? formatClockTime(request.trace.started_at_ms)
              : formatClockTime()
            const label = `${shortId} - ${timeLabel}`
            const isActive = request.trace ? formatRequestState(request.trace.state) === 'running' : false
            const trimmed = truncate(label, leftWidth - 4)
            return (
              <Text key={request.id} color={isSelected ? 'green' : undefined} bold={isActive}>
                {isSelected ? '> ' : '  '}
                {trimmed}
              </Text>
            )
          })}
        </Box>
        <Box
          flexDirection="column"
          width={rightWidth}
          marginLeft={2}
        >
          <Box
            flexDirection="column"
            height={detailBoxHeight}
            borderStyle="round"
            borderColor={focus === 'detail' ? 'green' : 'gray'}
          >
            <Text bold={focus === 'detail'}>details</Text>
            {detailVisible.map((entry, idx) => (
              <Text
                key={`detail-${idx}`}
                wrap="truncate"
                color={entry.isError ? 'red' : undefined}
              >
                {truncate(entry.text, detailWidth)}
              </Text>
            ))}
          </Box>
          <Box
            flexDirection="column"
            height={replBoxHeight}
            borderStyle="round"
            borderColor={focus === 'repl' ? 'green' : 'gray'}
          >
            <Text bold={focus === 'repl'}>repl</Text>
            {replVisible.map((entry, idx) => (
              <Text key={`repl-${idx}`} wrap="truncate" color={entry.color}>
                {truncate(entry.text, detailWidth)}
              </Text>
            ))}
            <Text wrap="truncate">{truncate(`> ${replInput}`, detailWidth)}</Text>
          </Box>
        </Box>
      </Box>
      <Box>
        <Text>up/down list  mod+up/down switch panes  mod+left list  mod+right detail  tab focus</Text>
        <Text>right panes: up/down scroll</Text>
        <Text>mod+t top  mod+y copy  mod+q quit</Text>
        <Text>mod=option/alt (mac; shift ok) / ctrl (linux/windows)</Text>
        {notice && <Text color="yellow">{`  ${notice}`}</Text>}
      </Box>
    </Box>
  )

  async function runReplCommand(line: string) {
    const parts = line.split(/\s+/).filter(Boolean)
    const command = parts[0]?.toLowerCase()
    const args = parts.slice(1)

    const appendLines = (lines: ReplLine[]) => {
      setReplLines((prev) => {
        const next = [...prev, ...lines].slice(-MAX_REPL_LINES)
        const nextLength = next.length
        const prevLength = prev.length
        setReplScroll((prevScroll) => {
          const prevMax = Math.max(0, prevLength - replOutputLines)
          const nextMax = Math.max(0, nextLength - replOutputLines)
          if (prevScroll >= prevMax) {
            return nextMax
          }
          return prevScroll
        })
        return next
      })
    }

    appendLines([{ text: `> ${line}` }])

    if (!command) return

    if (command === 'clear') {
      setReplLines([])
      setReplScroll(0)
      return
    }
    if (command === 'help') {
      appendLines([
        { text: 'commands:' },
        { text: '  help' },
        { text: '  stats' },
        { text: '  workers' },
        { text: '  requests [limit]' },
        { text: '  top [limit]' },
        { text: '  inspect <isolate_id>' },
        { text: '  kill <isolate_id>' },
        { text: '  clear' },
        { text: '  quit' },
      ])
      return
    }
    if (command === 'quit' || command === 'exit') {
      exit()
      return
    }

    try {
      if (command === 'stats') {
        const data = await fetchJson<RuntimeStats>(joinUrl(runtime, '/_deka/stats').toString())
        appendLines(JSON.stringify(data, null, 2).split('\n').map((text) => ({ text })))
        return
      }
      if (command === 'workers') {
        const data = await fetchJson<WorkerStats[]>(joinUrl(runtime, '/_deka/debug/workers').toString())
        appendLines(JSON.stringify(data, null, 2).split('\n').map((text) => ({ text })))
        return
      }
      if (command === 'requests') {
        const requestsUrl = joinUrl(runtime, '/_deka/debug/requests')
        if (args[0]) requestsUrl.searchParams.set('limit', args[0])
        if (archive) requestsUrl.searchParams.set('archive', 'true')
        const data = await fetchJson<RequestTrace[]>(requestsUrl.toString())
        appendLines(JSON.stringify(data, null, 2).split('\n').map((text) => ({ text })))
        return
      }
      if (command === 'top') {
        const topUrl = joinUrl(runtime, '/_deka/debug/top')
        if (args[0]) topUrl.searchParams.set('limit', args[0])
        const data = await fetchJson<Array<Record<string, unknown>>>(topUrl.toString())
        appendLines(JSON.stringify(data, null, 2).split('\n').map((text) => ({ text })))
        return
      }
      if (command === 'inspect') {
        const isolateId = args[0]
        if (!isolateId) {
          appendLines([{ text: 'inspect requires an isolate id', color: 'red' }])
          return
        }
        const url = joinUrl(runtime, `/_deka/debug/isolate/${encodeURIComponent(isolateId)}`)
        const data = await fetchJson<Record<string, unknown>>(url.toString())
        appendLines(JSON.stringify(data, null, 2).split('\n').map((text) => ({ text })))
        return
      }
      if (command === 'kill') {
        const isolateId = args[0]
        if (!isolateId) {
          appendLines([{ text: 'kill requires an isolate id', color: 'red' }])
          return
        }
        const url = joinUrl(runtime, `/_deka/debug/isolate/${encodeURIComponent(isolateId)}`)
        const response = await fetch(url.toString(), { method: 'DELETE' })
        const body = await response.text()
        if (!response.ok) {
          appendLines([{ text: `request failed (${response.status}) ${body}`, color: 'red' }])
          return
        }
        appendLines([{ text: body }])
        return
      }

      appendLines([{ text: `unknown command: ${command}`, color: 'red' }])
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err)
      appendLines([{ text: message, color: 'red' }])
    }
  }
}

export async function runIntrospectUi(runtime: string, archive = false) {
  const ink = withFullScreen(<IntrospectApp runtime={runtime} archive={archive} />)
  const hold = ink.waitUntilExit()
  ;(globalThis as { __dekaRuntimeHold?: Promise<void> }).__dekaRuntimeHold = hold
  await ink.start()
  await hold
}
