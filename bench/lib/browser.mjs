import { spawn } from 'node:child_process';
import { existsSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, delimiter } from 'node:path';
import { gzipSync } from 'node:zlib';

const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const hostTime = () => performance.now();
const browserStops = new Set();
export async function stopBrowsers() { await Promise.all([...browserStops].map(stop => stop())); }
export function findChrome() {
  const paths = [process.env.CHROME_BIN,
    '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
    '/Applications/Chromium.app/Contents/MacOS/Chromium',
    ...['google-chrome', 'google-chrome-stable', 'chromium', 'chromium-browser'].flatMap(bin =>
      (process.env.PATH || '').split(delimiter).map(dir => join(dir, bin)))];
  const chrome = paths.find(p => p && existsSync(p));
  if (!chrome) throw new Error('Chrome/Chromium is required; set CHROME_BIN to its executable');
  return chrome;
}

export async function withBrowser(fn) {
  const profile = mkdtempSync(join(tmpdir(), 'deka-bench-chrome-'));
  const proc = spawn(findChrome(), ['--headless=new', '--disable-gpu', '--no-first-run',
    '--no-default-browser-check', '--disable-dev-shm-usage', '--password-store=basic',
    '--remote-debugging-port=0', `--user-data-dir=${profile}`, 'about:blank'], { stdio: ['ignore', 'pipe', 'pipe'] });
  const stop = async () => {
    if (proc.exitCode === null && proc.signalCode === null) {
      const exited = new Promise(resolve => proc.once('exit', resolve));
      proc.kill('SIGKILL'); await exited;
    }
    rmSync(profile, { recursive: true, force: true });
    browserStops.delete(stop);
  };
  browserStops.add(stop);
  let log = '';
  proc.stdout.on('data', d => { log += d; });
  proc.stderr.on('data', d => { log += d; });
  proc.on('error', err => { log += err.message; });
  let ws;
  const pending = new Map();
  try {
    const deadline = Date.now() + 20_000;
    let endpoint;
    while (Date.now() < deadline) {
      endpoint = log.match(/DevTools listening on (ws:\/\/[^\s]+)/)?.[1];
      if (endpoint) break;
      if (proc.exitCode !== null) throw new Error(log);
      await sleep(50);
    }
    if (!endpoint) throw new Error(`Chromium startup timeout: ${log}`);
    const origin = new URL(endpoint).origin.replace('ws:', 'http:');
    const targets = await (await fetch(`${origin}/json/list`)).json();
    ws = new WebSocket(targets.find(t => t.type === 'page').webSocketDebuggerUrl);
    await new Promise((resolve, reject) => {
      ws.addEventListener('open', resolve, { once: true });
      ws.addEventListener('error', reject, { once: true });
    });
    let id = 0;
    const listeners = new Set();
    ws.addEventListener('message', event => {
      const msg = JSON.parse(event.data);
      if (msg.id) {
        const p = pending.get(msg.id);
        if (p) { clearTimeout(p.timer); pending.delete(msg.id); msg.error ? p.reject(new Error(JSON.stringify(msg.error))) : p.resolve(msg.result); }
      } else for (const listener of listeners) listener(msg);
    });
    const send = (method, params = {}) => new Promise((resolve, reject) => {
      const key = ++id;
      const timer = setTimeout(() => { pending.delete(key); reject(new Error(`CDP timeout: ${method}`)); }, 60_000);
      pending.set(key, { resolve, reject, timer });
      ws.send(JSON.stringify({ id: key, method, params }));
    });
    const evaluate = async expression => {
      const r = await send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
      if (r.exceptionDetails) throw new Error(JSON.stringify(r.exceptionDetails));
      return r.result.value;
    };
    const until = async (expr, timeout = 60_000) => {
      const start = Date.now();
      while (Date.now() - start < timeout) {
        try { const value = await evaluate(expr); if (value) return value; } catch (err) {
          if (!/context|navigat/i.test(err.message)) throw err;
        }
        await sleep(50);
      }
      throw new Error(`Browser condition timed out: ${expr}`);
    };
    await send('Page.enable'); await send('Runtime.enable'); await send('Performance.enable', { timeDomain: 'timeTicks' }); await send('Network.enable', { maxTotalBufferSize: 100_000_000, maxResourceBufferSize: 20_000_000 });
    await send('Network.setCacheDisabled', { cacheDisabled: true });
    const browser = { send, evaluate, until, listeners,
      navigate: async url => {
        const r = await send('Page.navigate', { url });
        if (r.errorText) throw new Error(r.errorText);
        await until(`document.readyState === 'complete' && location.href === ${JSON.stringify(url)}`);
      } };
    return await fn(browser);
  } finally {
    for (const p of pending.values()) { clearTimeout(p.timer); p.reject(new Error('Browser closed')); }
    ws?.close();
    await stop();
  }
}

export async function payload(browser, url, interactive) {
  const responses = new Map();
  const failed = [];
  const inFlight = new Set();
  const finished = new Set();
  let lastActivity = Date.now();
  const listener = msg => {
    if (msg.method === 'Network.requestWillBeSent') { inFlight.add(msg.params.requestId); lastActivity = Date.now(); }
    if (msg.method === 'Network.loadingFailed') { failed.push(msg.params); inFlight.delete(msg.params.requestId); lastActivity = Date.now(); }
    if (msg.method === 'Network.responseReceived') {
      const { requestId, type, response } = msg.params;
      responses.set(requestId, { type, ...response }); lastActivity = Date.now();
    }
    if (msg.method === 'Network.loadingFinished') { finished.add(msg.params.requestId); inFlight.delete(msg.params.requestId); lastActivity = Date.now(); }
  };
  browser.listeners.add(listener);
  try {
    await browser.navigate(url);
    await browser.until("!!document.querySelector('article .prose')");
    const deadline = Date.now() + 20_000;
    while (Date.now() < deadline && (inFlight.size > 0 || Date.now() - lastActivity < 1000)) await sleep(100);
    if (Date.now() >= deadline) throw new Error('Payload network did not become quiet');
    if (failed.length) throw new Error(`Payload network failure: ${JSON.stringify(failed)}`);
    browser.listeners.delete(listener); // freeze the initial-visit observation window
    const assets = [];
    const excluded = [];
    for (const [requestId, response] of responses) {
      const { type, mimeType, url: assetUrl, status } = response;
      if (status >= 400 && !assetUrl.endsWith('/favicon.ico')) throw new Error(`Payload HTTP ${status}: ${assetUrl}`);
      if (!finished.has(requestId)) throw new Error(`Incomplete request: ${assetUrl}`);
      const kind = type === 'Document' ? 'html' : /javascript/.test(mimeType) ? 'js' : mimeType === 'text/css' ? 'css' : /text\/x-component/.test(mimeType) ? 'rsc' : null;
      if (!kind) { excluded.push({ url: assetUrl, type, mimeType, status }); continue; }
      const result = await browser.send('Network.getResponseBody', { requestId });
      const body = Buffer.from(result.body, result.base64Encoded ? 'base64' : 'utf8');
      assets.push({ url: assetUrl, kind, status, raw: body.length, gzip: gzipSync(body).length });
    }
    const sum = (kind, key) => assets.filter(a => a.kind === kind).reduce((n, a) => n + a[key], 0);
    const result = { htmlRaw: sum('html', 'raw'), htmlGzip: sum('html', 'gzip'), jsRaw: sum('js', 'raw'), jsGzip: sum('js', 'gzip'), cssGzip: sum('css', 'gzip'), rscGzip: sum('rsc', 'gzip'), totalGzip: assets.reduce((n, a) => n + a.gzip, 0), assets, excluded };
    if (interactive) {
      await browser.until("!!document.querySelector('#theme-toggle') && !!document.querySelector('#newsletter')");
      await sleep(400);
      const before = await browser.evaluate("document.querySelector('#theme-toggle').getAttribute('data-theme')");
      const backgroundBefore = await browser.evaluate('getComputedStyle(document.body).backgroundColor');
      await browser.evaluate("document.querySelector('#theme-toggle').click()");
      const after = await browser.until(`(() => {const t = document.querySelector('#theme-toggle').getAttribute('data-theme'); return t !== ${JSON.stringify(before)} && t;})()`);
      await browser.until(`document.documentElement.getAttribute('data-theme') === ${JSON.stringify(after)}`);
      const backgroundAfter = await browser.until(`getComputedStyle(document.body).backgroundColor !== ${JSON.stringify(backgroundBefore)} && getComputedStyle(document.body).backgroundColor`);
      await browser.evaluate(`(() => {
        const input = document.querySelector('#nl-email');
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(input, 'ava@deka.gg');
        input.dispatchEvent(new Event('input', { bubbles: true }));
        input.dispatchEvent(new Event('change', { bubbles: true }));
      })()`);
      await browser.evaluate("document.querySelector('#newsletter').requestSubmit()");
      const status = await browser.until("document.querySelector('#nl-status').textContent.includes('ava@deka.gg') && document.querySelector('#nl-status').textContent");
      result.widgets = { theme: `${before}->${after}`, backgroundBefore, backgroundAfter, newsletter: status };
    }
    return result;
  } finally { browser.listeners.delete(listener); }
}

// CDP Performance.Timestamp and Tracing user-timing events share Chromium's
// monotonic clock, including across document reloads. Calibrate the host write
// to that clock, then use the trace timestamp of a DOM-observer performance mark.
export async function editLatency(browser, file, source, expected, selector) {
  let calibration;
  for (let i = 0; i < 7; i++) {
    const before = hostTime();
    const { metrics } = await browser.send('Performance.getMetrics');
    const after = hostTime();
    const timestamp = metrics.find(m => m.name === 'Timestamp').value * 1000;
    const probe = { offsetMs: timestamp - (before + after) / 2, roundTripMs: after - before };
    if (!calibration || probe.roundTripMs < calibration.roundTripMs) calibration = probe;
  }
  const token = `bench-applied-${Date.now()}-${Math.random()}`;
  const frames = [];
  let appliedTimestamp;
  let complete;
  const tracingComplete = new Promise(resolve => { complete = resolve; });
  const listener = msg => {
    if (msg.method === 'Network.webSocketFrameReceived') frames.push({ cdpTimestamp: msg.params.timestamp, data: msg.params.response.payloadData.slice(0, 500) });
    if (msg.method === 'Tracing.dataCollected') {
      for (const event of msg.params.value) {
        if (event.name === token && event.cat?.includes('blink.user_timing')) appliedTimestamp ??= event.ts / 1000;
      }
    }
    if (msg.method === 'Tracing.tracingComplete') complete();
  };
  browser.listeners.add(listener);
  await browser.send('Tracing.start', { categories: 'blink.user_timing', transferMode: 'ReportEvents' });
  let tracing = true;
  const observer = `(() => {
    const check = () => {
      if (document.querySelector(${JSON.stringify(selector)})?.textContent.includes(${JSON.stringify(expected)})) {
        performance.mark(${JSON.stringify(token)});
        window.__benchApplied = { origin: performance.timeOrigin, token: ${JSON.stringify(token)} };
        watcher.disconnect();
      }
    };
    const watcher = new MutationObserver(check);
    watcher.observe(document, { childList: true, subtree: true, characterData: true });
    check();
  })()`;
  const { identifier } = await browser.send('Page.addScriptToEvaluateOnNewDocument', { source: observer });
  const origin = await browser.evaluate('performance.timeOrigin');
  await browser.evaluate(observer);
  try {
    const saveStart = hostTime() + calibration.offsetMs;
    writeFileSync(file, source);
    const saveEnd = hostTime() + calibration.offsetMs;
    const applied = await browser.until(`window.__benchApplied?.token === ${JSON.stringify(token)} && window.__benchApplied`);
    await browser.send('Tracing.end'); tracing = false;
    let timer;
    try {
      await Promise.race([tracingComplete, new Promise((_, reject) => { timer = setTimeout(() => reject(new Error('CDP tracing completion timed out')), 20_000); })]);
    } finally { clearTimeout(timer); }
    if (!Number.isFinite(appliedTimestamp)) throw new Error('DOM update has no CDP trace timestamp');
    const ms = appliedTimestamp - saveEnd;
    if (ms < 0) throw new Error('Invalid HMR clock alignment');
    return { ms, saveStart, saveEnd, applied: appliedTimestamp, traceMark: token, calibration, frames, fullReload: origin !== applied.origin };
  } finally {
    if (tracing) await browser.send('Tracing.end');
    browser.listeners.delete(listener);
    await browser.send('Page.removeScriptToEvaluateOnNewDocument', { identifier });
  }
}
