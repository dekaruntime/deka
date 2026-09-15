// Browser server-HTML Fast Refresh regression for deka#956.
// Edit a server-rendered heading while an island counter is live; the
// heading must update, the counter must keep its state, a form input
// outside the island must keep its value, and the page must not navigate.
// Editing the island module itself must full-reload.

import assert from 'node:assert/strict'
import { closeSync, existsSync, mkdirSync, openSync } from 'node:fs'
import { cp, rm, writeFile } from 'node:fs/promises'
import net from 'node:net'
import path from 'node:path'
import { spawn } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { chromium } from 'playwright'

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..', '..')
const cli = process.env.DEKA_NATIVE || path.join(repoRoot, 'target', 'release', 'cli')
const fixture = path.join(repoRoot, 'crates', 'cli', 'tests', 'fixtures', 'server-fast-refresh')

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

async function waitFor(label, probe, timeoutMs = 90_000) {
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

const pageSource = (title) => `export fn Page() ReactNode {
  return (
    <section>
      <h1 id="server-title">${title}</h1>
      <form>
        <input id="outside-input" name="note" value="initial" />
      </form>
      <div id="spacer" class="spacer"></div>
    </section>
  )
}
`

async function main() {
  assert.ok(
    existsSync(cli),
    `release CLI not found at ${cli}; build it with cargo build --release -p cli --features dev-server`,
  )
  const project = path.join(repoRoot, 'scripts', '.run-tmp', 'server-fast-refresh-e2e', String(process.pid))
  mkdirSync(project, { recursive: true })
  const logPath = path.join(project, 'dev.log')
  const logFd = openSync(logPath, 'w')
  let server
  let browser

  try {
    await cp(fixture, project, { recursive: true })
    const port = await freePort()
    const env = { ...process.env, NO_COLOR: '1', FORCE_COLOR: '0' }
    server = spawn(cli, ['dev', '.', '--port', String(port), '--no-prompt'], {
      cwd: project,
      stdio: ['ignore', logFd, logFd],
      env,
    })
    const url = `http://127.0.0.1:${port}/`
    const html = await waitFor('deka dev to SSR the server-refresh page', async () => {
      const response = await fetch(url)
      const body = await response.text()
      return response.ok && body.includes('hello server') && body.includes('__deka_hmr_client')
        ? body
        : false
    })
    assert.match(html, /id="counter"|data-deka-island="Counter"/)

    browser = await chromium.launch({ headless: true })
    const page = await browser.newPage()
    const pageErrors = []
    page.on('pageerror', (error) => pageErrors.push(error.message))
    await page.goto(url, { waitUntil: 'networkidle' })
    await page.waitForSelector('#server-title')
    await page.waitForSelector('#counter')
    assert.equal(await page.textContent('#server-title'), 'hello server')

    const clickDeadline = Date.now() + 30_000
    while (Date.now() < clickDeadline) {
      await page.click('#counter')
      if ((await page.textContent('#counter')) === '1') break
      await Bun.sleep(50)
    }
    while ((await page.textContent('#counter')) !== '3' && Date.now() < clickDeadline) {
      await page.click('#counter')
      await Bun.sleep(30)
    }
    assert.equal(await page.textContent('#counter'), '3', 'pre-edit clicks must stick in island state')

    await page.fill('#outside-input', 'kept')
    await page.evaluate(() => window.scrollTo(0, 480))
    const scrollBefore = await page.evaluate(() => window.scrollY || window.pageYOffset || 0)

    let navigations = 0
    page.on('framenavigated', (frame) => {
      if (frame === page.mainFrame()) navigations += 1
    })

    await writeFile(path.join(project, 'app', 'page.dsx'), pageSource('hello refreshed'))
    await page.waitForFunction(
      () => document.querySelector('#server-title')?.textContent === 'hello refreshed',
      undefined,
      { timeout: 45_000 },
    )
    assert.equal(await page.textContent('#counter'), '3', 'morph must preserve hydrated island state')
    assert.equal(await page.inputValue('#outside-input'), 'kept', 'form input outside the island must keep its value')
    const scrollAfter = await page.evaluate(() => window.scrollY || window.pageYOffset || 0)
    assert.ok(Math.abs(scrollBefore - scrollAfter) < 80, `scroll survived morph: ${scrollBefore} -> ${scrollAfter}`)
    assert.equal(navigations, 0, 'a server-text edit must not full-reload the page')
    assert.equal(pageErrors.join('\n'), '', `page errors: ${pageErrors.join('\n')}`)

    await writeFile(
      path.join(project, 'src', 'ui', 'Counter.dsx'),
      `export fn Counter() ReactNode {
  const pair = useState(10)
  const n = pair[0]
  const setN = pair[1]
  return (
    <button type="button" id="counter" onClick={fn() void {
      setN(n + 1)
    }}>{n}</button>
  )
}
`,
    )
    const navDeadline = Date.now() + 45_000
    while (Date.now() < navDeadline && navigations === 0) {
      await Bun.sleep(50)
    }
    assert.ok(navigations >= 1, 'editing the island module must full-reload')
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
console.log('browser server Fast Refresh morph + island-reload regression passed')
