// Browser Fast Refresh regression for deka#936.
// Edit a sibling component and require the loaded page to keep Counter state.
// Project files live under the worktree target dir — never /tmp.

import assert from 'node:assert/strict'
import { closeSync, existsSync, mkdirSync, openSync } from 'node:fs'
import { readFile, rm, writeFile } from 'node:fs/promises'
import net from 'node:net'
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

async function writeProject(project) {
  await writeFile(path.join(project, 'deka.json'), '{"name":"fast-refresh-e2e","security":{"prompt":false}}\n')
  await writeFile(
    path.join(project, 'index.js'),
    `globalThis.app = {
  async fetch() {
    const html = \`<!doctype html>
<html>
  <head><meta charset="utf-8"><title>fast-refresh</title></head>
  <body>
    <div id="root"></div>
    <script type="module">
      import { createElement as h } from "react";
      import { createRoot } from "react-dom/client";
      import { App } from "/_deka/hmr/module/App.js";
      createRoot(document.getElementById("root")).render(h(App));
    </script>
  </body>
</html>\`;
    return new Response(html, { headers: { "content-type": "text/html; charset=utf-8" } });
  },
};
`,
  )
  await writeFile(
    path.join(project, 'App.js'),
    `import { createElement as h } from "react";
import { Counter } from "./Counter.js";
import { Label } from "./Label.js";
export function App() {
  return h("div", { id: "app-root" }, h(Counter), h(Label));
}
`,
  )
  await writeFile(
    path.join(project, 'Counter.js'),
    `import { useState, createElement as h } from "react";
export function Counter() {
  const [n, setN] = useState(0);
  return h("button", { id: "counter", onClick: () => setN(n + 1) }, String(n));
}
`,
  )
  await writeFile(
    path.join(project, 'Label.js'),
    `import { createElement as h } from "react";
export function Label() {
  return h("p", { id: "label" }, "hello");
}
`,
  )
}

async function main() {
  assert.ok(existsSync(cli), `release CLI not found at ${cli}; build it with cargo build --release -p cli --features dev-server`)
  // Worktree-local and not ignored by the dev watcher (`target` / `.cache`
  // / `node_modules` segments are skipped). scripts/.run-tmp is gitignored.
  const project = path.join(repoRoot, 'scripts', '.run-tmp', 'fast-refresh-e2e', String(process.pid))
  mkdirSync(project, { recursive: true })
  const logPath = path.join(project, 'dev.log')
  const logFd = openSync(logPath, 'w')
  let server
  let browser

  try {
    await writeProject(project)
    const port = await freePort()
    server = spawn(cli, ['dev', '.', '--port', String(port), '--no-prompt'], {
      cwd: project,
      stdio: ['ignore', logFd, logFd],
    })
    const url = `http://127.0.0.1:${port}/`
    await waitFor('deka dev to serve the React page', async () => {
      const response = await fetch(url)
      const body = await response.text()
      return response.ok && body.includes('__deka_refresh_preamble') ? body : false
    })

    const moduleSource = await waitFor('refresh-wrapped Counter module', async () => {
      const response = await fetch(`${url}_deka/hmr/module/Counter.js`)
      const body = await response.text()
      return response.ok && body.includes('$RefreshReg$(Counter, "Counter")') ? body : false
    })
    assert.match(moduleSource, /__dekaRefreshBoundary = true/)

    // Exceed the default per-IP burst before the browser opens its HMR WS.
    // Vendor and module fan-out must not starve that handshake or the page.
    const fanOut = await Promise.all(Array.from({ length: 90 }, (_, i) =>
      fetch(`${url}_deka/${i % 2 ? 'react/react.js' : 'hmr/module/Counter.js'}`)))
    for (const response of fanOut) {
      assert.equal(response.status, 200, 'dev fan-out must bypass rate limiting')
      await response.arrayBuffer()
    }

    browser = await chromium.launch({ headless: true })
    const page = await browser.newPage()
    const pageErrors = []
    page.on('pageerror', (error) => pageErrors.push(error.message))
    let sawJsUpdate = false
    page.on('websocket', (socket) => {
      if (!socket.url().endsWith('/_deka/hmr')) return
      socket.on('framereceived', (frame) => {
        const payload = typeof frame.payload === 'string' ? frame.payload : ''
        if (payload.includes('"type":"js-update"') || payload.includes('"type": "js-update"')) {
          sawJsUpdate = true
        }
      })
    })

    await page.goto(url, { waitUntil: 'networkidle' })
    await page.waitForSelector('#counter')
    await page.waitForSelector('#label')
    assert.equal(await page.textContent('#counter'), '0')
    assert.equal(await page.textContent('#label'), 'hello')
    await page.click('#counter')
    await page.click('#counter')
    await page.click('#counter')
    assert.equal(await page.textContent('#counter'), '3', 'pre-edit clicks must stick in React state')

    let navigations = 0
    page.on('framenavigated', (frame) => {
      if (frame === page.mainFrame()) navigations += 1
    })

    await writeFile(
      path.join(project, 'Label.js'),
      `import { createElement as h } from "react";
export function Label() {
  return h("p", { id: "label" }, "hello world");
}
`,
    )

    await page.waitForFunction(
      () => document.querySelector('#label')?.textContent === 'hello world',
      undefined,
      { timeout: 45_000 },
    )
    assert.equal(
      await page.textContent('#counter'),
      '3',
      'Fast Refresh must preserve sibling Counter state across a Label JSX edit',
    )
    assert.equal(navigations, 0, 'a refreshable edit must not full-reload the page')
    assert.equal(pageErrors.join('\n'), '', `page errors: ${pageErrors.join('\n')}`)
    assert.ok(sawJsUpdate, 'the HMR websocket must push a js-update for the edited module')
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
console.log('browser Fast Refresh sibling-state regression passed')
