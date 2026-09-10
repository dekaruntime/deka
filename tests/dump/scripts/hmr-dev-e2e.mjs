// Browser-level regression for deka#779. A fresh HTTP request can observe an
// edited source even when the HMR client fails to parse, so this deliberately
// holds one loaded page open and waits for that page to update.

import assert from 'node:assert/strict'
import { closeSync, existsSync, openSync } from 'node:fs'
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import net from 'node:net'
import os from 'node:os'
import path from 'node:path'
import { spawn } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { chromium } from 'playwright'

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..', '..')
const cli = process.env.DEKA_NATIVE || path.join(repoRoot, 'target', 'release', 'cli')

function freePort() {
  return new Promise((resolve, reject) => {
    const listener = net.createServer()
    listener.once('error', reject)
    listener.listen(0, '127.0.0.1', () => {
      const address = listener.address()
      if (!address || typeof address === 'string') {
        listener.close()
        reject(new Error('could not reserve an IPv4 port'))
        return
      }
      listener.close((error) => (error ? reject(error) : resolve(address.port)))
    })
  })
}

async function waitFor(label, probe, timeoutMs = 45_000) {
  const deadline = Date.now() + timeoutMs
  let lastError = ''
  while (Date.now() < deadline) {
    try {
      const value = await probe()
      if (value) return value
    } catch (error) {
      lastError = error instanceof Error ? error.message : String(error)
    }
    await Bun.sleep(100)
  }
  throw new Error(`timed out waiting for ${label}${lastError ? `: ${lastError}` : ''}`)
}

function handlerSource(status) {
  const html = `<!doctype html><html><body><div id="app"><p data-deka-id="status">${status}</p></div></body></html>`
  return `globalThis.app = {
  async fetch() {
    return new Response(${JSON.stringify(html)}, {
      headers: { "content-type": "text/html; charset=utf-8" },
    });
  },
};
`
}

function writeHandler(project, status) {
  return writeFile(path.join(project, 'index.js'), handlerSource(status))
}

async function main() {
  assert.ok(existsSync(cli), `release CLI not found at ${cli}; build it with cargo build --release -p cli`)
  const project = await mkdtemp(path.join(os.tmpdir(), 'deka-hmr-browser-'))
  const logPath = path.join(project, 'dev.log')
  const logFd = openSync(logPath, 'w')
  let server
  let browser

  try {
    await writeFile(project + '/deka.json', '{"name":"hmr-browser","security":{"prompt":false}}\n')
    await writeHandler(project, 'HMR initial page')

    const port = await freePort()
    server = spawn(cli, ['dev', '.', '--port', String(port), '--no-prompt'], {
      cwd: project,
      env: { ...process.env, DEKA_RATE_LIMIT_DISABLED: '1' },
      stdio: ['ignore', logFd, logFd],
    })
    const url = `http://127.0.0.1:${port}/`
    const initialHtml = await waitFor('deka dev to serve the initial page', async () => {
      const response = await fetch(url)
      const body = await response.text()
      return response.ok && body.includes('HMR initial page') ? body : false
    })

    browser = await chromium.launch({ headless: true })
    const page = await browser.newPage()
    const pageErrors = []
    const consoleErrors = []
    page.on('pageerror', (error) => pageErrors.push(error.message))
    page.on('console', (message) => {
      if (message.type() === 'error') consoleErrors.push(message.text())
    })
    let resolveFirstHmrSocket
    const firstHmrSocket = new Promise((resolve) => {
      resolveFirstHmrSocket = resolve
    })
    page.on('websocket', (socket) => {
      if (socket.url().endsWith('/_deka/hmr')) {
        resolveFirstHmrSocket(socket)
      }
    })

    await page.goto(url, { waitUntil: 'domcontentloaded' })
    await page.waitForFunction(() => document.querySelector('#app')?.textContent?.includes('HMR initial page'))
    try {
      await Promise.race([
        firstHmrSocket,
        Bun.sleep(10_000).then(() => {
          throw new Error('the loaded page never opened the HMR WebSocket')
        }),
      ])
    } catch (error) {
      const clientSource = await page.evaluate(
        () => document.querySelector('#__deka_hmr_client')?.textContent || '(not found)',
      )
      throw new Error(
        `${error instanceof Error ? error.message : String(error)}\n` +
          `page errors: ${pageErrors.join('\n') || '(none)'}\n` +
          `console errors: ${consoleErrors.join('\n') || '(none)'}\n` +
          `injected client:\n${clientSource.slice(0, 2_000)}`,
      )
    }
    assert.deepEqual(pageErrors, [], `the HMR client must parse without page errors: ${pageErrors.join('\n')}`)
    assert.equal(
      (initialHtml.match(/<script id="__deka_hmr_client" type="module">/g) || []).length,
      1,
      `deka dev must inject exactly one HMR module script opening:\n${initialHtml}`,
    )

    let navigations = 0
    page.on('framenavigated', (frame) => {
      if (frame === page.mainFrame()) navigations += 1
    })

    // The first update seeds the server snapshot; the second verifies the
    // per-node diff that follows it. Neither may navigate the loaded page.
    await writeHandler(project, 'HMR snapshot initialized')
    await page.waitForFunction(
      () => document.querySelector('#app')?.textContent?.includes('HMR snapshot initialized'),
      undefined,
      { timeout: 45_000 },
    )

    await writeHandler(project, 'HMR patched in the loaded page')

    // Do not make another HTTP request here. The assertion is against the
    // already-loaded browser document; this second edit must use a patch,
    // rather than navigate after the initial snapshot has been established.
    await page.waitForFunction(
      () => document.querySelector('#app')?.textContent?.includes('HMR patched in the loaded page'),
      undefined,
      { timeout: 45_000 },
    )
    assert.equal(navigations, 0, 'a diffable source edit must patch the loaded page without navigation')

    const log = await readFile(logPath, 'utf8')
    assert.match(
      log,
      /\[watch\] evicted [1-9]\d*/,
      `the source edit must evict the serving pool (#731), not rely on a cache-key accident:\n${log}`,
    )
  } finally {
    if (browser) await browser.close()
    if (server && server.exitCode === null && server.signalCode === null) {
      server.kill()
      await Promise.race([
        new Promise((resolve) => server.once('exit', resolve)),
        Bun.sleep(5_000),
      ])
    }
    closeSync(logFd)
    await rm(project, { recursive: true, force: true })
  }
}

await main()
console.log('browser HMR source-edit regression passed')
