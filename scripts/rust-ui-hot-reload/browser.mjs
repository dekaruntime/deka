// Actual cargo-deka dev server, on-disk Rust saves, headless Chrome and pixels.
import { chromium } from '../rust-tour-browser/node_modules/playwright/index.mjs'
import { spawn, spawnSync } from 'node:child_process'
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs'
import { resolve } from 'node:path'
import assert from 'node:assert/strict'
import { createInterface } from 'node:readline'

const root = resolve(import.meta.dirname, '../..')
const source = resolve(root, 'crates/deka_ui_tour/examples/hot_reload_web.rs')
const original = readFileSync(source, 'utf8')
const logs = [], browserLog = []
const evidence = resolve(root, 'tasks/evidence/ui-hot-reload')
mkdirSync(evidence, { recursive: true })
const target = JSON.parse(spawnSync('cargo', ['metadata', '--format-version', '1', '--no-deps'], { cwd: root, encoding: 'utf8' }).stdout).target_directory
const build = spawnSync('cargo', ['build', '--locked', '-p', 'deka-ui-dev'], { cwd: root, encoding: 'utf8' })
assert.equal(build.status, 0, build.stderr)
let server, browser
const quiet = ms => new Promise(done => setTimeout(done, ms))
try {
  server = spawn(resolve(target, 'debug/cargo-deka'), ['dev', '--web', '--port', '0', '--locked', '-p', 'deka-ui-tour', '--example', 'hot_reload_web'], { cwd: root, stdio: ['ignore', 'pipe', 'pipe'] })
  const url = await new Promise((done, reject) => {
    const timeout = setTimeout(() => reject(new Error('server start timeout: ' + logs.slice(-20).join('\n'))), 300000)
    for (const stream of [server.stdout, server.stderr]) {
      createInterface({ input: stream }).on('line', line => {
        logs.push(line)
        const match = line.match(/browser at (http:\/\/[^;]+)/)
        if (match) { clearTimeout(timeout); done(match[1]) }
      })
    }
    server.on('exit', code => { clearTimeout(timeout); reject(new Error('server exited ' + code + ': ' + logs.slice(-20).join('\n'))) })
  })
  browser = await chromium.launch({ executablePath: process.env.RUST_TOUR_CHROME || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', headless: true })
  const page = await browser.newPage()
  page.setDefaultTimeout(10000)
  const errors = [], messages = []
  let navigations = 0
  page.on('pageerror', error => { errors.push(error.message); browserLog.push(error.message) })
  page.on('console', message => { browserLog.push(message.text()); messages.push(message.text()); if (message.type() === 'error') errors.push(message.text()) })
  page.on('framenavigated', frame => { if (frame === page.mainFrame()) navigations++ })
  await page.goto(url)
  const counter = () => page.getByRole('button', { name: /^(Count|Changed): / })
  await counter().focus()
  for (let i = 0; i < 3; i++) await page.keyboard.press('Enter')
  await assertText(page, 'Count: 3')
  const identity = await counter().evaluate(node => node)
  assert(identity)
  await counter().evaluate(node => { window.savedCounter = node })
  const canvas = page.locator('canvas')
  const pixels = await canvas.screenshot()
  const edited = original.replace('"Before"', '"After"').replace('title="Count"', 'title="Changed"').replace('<div id="left"><Counter title="Changed" seed=0/>{conditional}</div><div id="right"/>', '<div id="left"/><div id="right">{conditional}<Counter title="Changed" seed=0/></div>')
  assert.notEqual(edited, original)
  const started = performance.now()
  writeFileSync(source, edited)
  await assertText(page, 'Changed: 3')
  await assertText(page, 'After')
  const elapsed = performance.now() - started
  assert.equal(navigations, 1, 'markup must not navigate/restart the browser')
  assert(await counter().evaluate(node => node === window.savedCounter), 'component DOM identity changed')
  assert.notDeepEqual(await canvas.screenshot(), pixels, 'canvas pixels did not change')
  await page.getByRole('button', { name: 'Toggle', exact: true }).focus()
  await page.keyboard.press('Enter')
  await assertText(page, 'Conditional')
  await counter().focus(); await page.keyboard.press('Enter')
  await assertText(page, 'Changed: 4')
  const after = await canvas.screenshot()
  const messagesBefore = messages.length
  writeFileSync(source, edited.replace('class="p-4 gap-4"', 'class="p-bad"'))
  await page.waitForFunction(() => true)
  await waitUntil(() => messages.slice(messagesBefore).some(message => message.includes('keeping last good UI')), 'malformed diagnostic')
  assert.notEqual(messagesBefore, messages.length)
  await assertText(page, 'Changed: 4')
  assert.deepEqual(await canvas.screenshot(), after, 'malformed edit changed pixels')
  writeFileSync(source, edited.replace('"After"', '"Recovered"'))
  await assertText(page, 'Recovered')
  await assertText(page, 'Changed: 4')
  // A newly connected module starts with compiled templates and replays edits.
  const late = await browser.newPage()
  await late.goto(url)
  await assertText(late, 'Recovered')
  await assertText(late, 'Changed: 0')
  await late.close()
  writeFileSync(source, original)
  await assertText(page, 'Before')
  await assertText(page, 'Count: 4')
  // A markup-classified prop consumed by initialization reaches the browser's
  // runtime fallback and requests a real rebuild without a partial patch.
  const fallbackMessages = messages.length
  writeFileSync(source, original.replace('seed=0', 'seed=1').replace('"Before"', '"Fallback"'))
  await waitUntil(() => messages.slice(fallbackMessages).some(message => message.includes('prop `seed`') && message.includes('signal state resets')), 'runtime fallback diagnostic')
  await assertText(page, 'Before')
  await assertText(page, 'Count: 4')
  await assertText(page, 'Fallback', 120000)
  await assertText(page, 'Count: 1')
  assert.equal(navigations, 2, 'runtime fallback must replace the module once')
  // A compiled handler edit blocks a concurrent template edit and rebuilds.
  const rebuilt = original.replace('count += 1', 'count += 2').replace('"Before"', '"Rebuilt"')
  writeFileSync(source, rebuilt)
  await assertText(page, 'Rebuilt', 120000)
  await assertText(page, 'Count: 0')
  assert.equal(navigations, 3, 'compiled code must replace the module once')
  await counter().focus(); await page.keyboard.press('Enter')
  await assertText(page, 'Count: 2')
  assert.deepEqual(errors, [])
  console.log(`browser: source/prop/empty-slot patch in ${elapsed.toFixed(1)} ms; pixels changed; state 3 → retained handler → 4; malformed recovery, late connection, rollback, atomic runtime fallback and compiled-code reload passed`)
} finally {
  if (browserLog.length) console.log(browserLog.join("\n"))
  await browser?.close()
  if (server) {
    server.kill('SIGINT')
    await Promise.race([new Promise(done => server.once('exit', done)), quiet(10000)])
    if (server.exitCode === null) server.kill('SIGKILL')
  }
  writeFileSync(source, original)
  writeFileSync(resolve(evidence, 'phase2-browser.log'), logs.join('\n'))
  writeFileSync(resolve(evidence, 'phase2-browser-console.log'), browserLog.join('\n'))
}
async function assertText(page, text, timeout = 10000) {
  await page.waitForFunction(text => [...document.querySelectorAll('[aria-label]')].some(node => node.getAttribute('aria-label') === text) || [...document.querySelectorAll('span')].some(node => node.textContent === text), text, { timeout })
}
async function waitUntil(test, label) {
  const deadline = performance.now() + 10000
  while (!test()) { assert(performance.now() < deadline, label + ' timed out'); await quiet(25) }
}
