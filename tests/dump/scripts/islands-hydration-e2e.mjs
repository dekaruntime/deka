// Browser islands hydration regression for deka#946.
// Serve the Theme + Newsletter DSX islands and require React state to
// drive the DOM: click theme → attribute/label change; submit newsletter →
// the compiled component's status text renders.

import assert from 'node:assert/strict'
import { closeSync, existsSync, mkdirSync, openSync } from 'node:fs'
import { cp, rm } from 'node:fs/promises'
import net from 'node:net'
import path from 'node:path'
import { spawn } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { chromium } from 'playwright'

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..', '..')
const cli = process.env.DEKA_NATIVE || path.join(repoRoot, 'target', 'release', 'cli')
const fixture = path.join(repoRoot, 'crates', 'cli', 'tests', 'fixtures', 'islands-hydration')

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

async function waitFor(label, probe, timeoutMs = 60_000) {
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

async function main() {
  assert.ok(existsSync(cli), `release CLI not found at ${cli}; build it with cargo build --release -p cli`)
  const project = path.join(repoRoot, 'scripts', '.run-tmp', 'islands-hydration-e2e', String(process.pid))
  mkdirSync(project, { recursive: true })
  const logPath = path.join(project, 'serve.log')
  const logFd = openSync(logPath, 'w')
  let server
  let browser

  try {
    await cp(fixture, project, { recursive: true })
    const port = await freePort()
    const env = { ...process.env, NO_COLOR: '1' }
    server = spawn(cli, ['serve', '.', '--port', String(port), '--no-prompt'], {
      cwd: project,
      stdio: ['ignore', logFd, logFd],
      env,
    })
    const url = `http://127.0.0.1:${port}/`
    const html = await waitFor('deka serve to SSR the islands page', async () => {
      const response = await fetch(url)
      const body = await response.text()
      return response.ok && body.includes('data-deka-island="ThemeToggle"') ? body : false
    })
    assert.match(html, /data-deka-island="NewsletterSignup"/)
    assert.match(html, /src="\/assets\/islands\.js"/)

    const bundle = await waitFor('shared islands bundle', async () => {
      const response = await fetch(`${url}assets/islands.js`)
      const body = await response.text()
      return response.ok && body.includes('function ThemeToggle') ? body : false
    })
    assert.match(bundle, /function NewsletterSignup/)
    assert.match(bundle, /hydrateRoot/)
    assert.doesNotMatch(bundle, /react-dom-client\.development\.js/)

    browser = await chromium.launch({ headless: true })
    const page = await browser.newPage()
    const pageErrors = []
    page.on('pageerror', (error) => pageErrors.push(error.message))
    await page.goto(url, { waitUntil: 'networkidle' })
    await page.waitForSelector('#theme-toggle')
    await page.waitForSelector('#newsletter')
    assert.equal(await page.getAttribute('#theme-toggle', 'data-theme'), 'light')
    assert.equal(await page.textContent('#theme-toggle'), 'Dark')

    await page.click('#theme-toggle')
    await page.waitForFunction(
      () => document.querySelector('#theme-toggle')?.getAttribute('data-theme') === 'dark',
      undefined,
      { timeout: 10_000 },
    )
    assert.equal(await page.textContent('#theme-toggle'), 'Light', 'theme island must re-render from React state')

    await page.fill('#nl-email', 'ava@deka.gg')
    await page.click('#nl-submit')
    await page.waitForFunction(
      () => (document.querySelector('#nl-status')?.textContent || '').includes('ava@deka.gg'),
      undefined,
      { timeout: 10_000 },
    )
    assert.equal(pageErrors.join('\n'), '', `page errors: ${pageErrors.join('\n')}`)
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
console.log('browser islands hydration (theme + newsletter) passed')
