#!/usr/bin/env node
/** Three-way benchmark. Methodology and prerequisites: README.md. */
import { spawn } from "node:child_process";
import { Agent, get } from "node:http";
import { createServer as createSocketServer } from "node:net";
import { createHash } from "node:crypto";
import { withBrowser, payload, editLatency, findChrome, stopBrowsers } from "./lib/browser.mjs";

import {
  cpSync,
  existsSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, delimiter, join } from "node:path";
import { fileURLToPath } from "node:url";
import { ingest } from "./lib/ingest.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const toolchainDir = join(here, ".toolchain");
const RUNS = 5;
const ISLANDS_PATH = "/posts/islands-are-a-budget";
const DEKA_TOUCH = join(here, "deka-blog/src/ui/PostCard.dsx");
const VITE_TOUCH = join(here, "vite-blog/src/components/PostCard.tsx");

function which(bin) {
  const path = process.env.PATH || "";
  for (const dir of path.split(delimiter)) {
    const candidate = join(dir, bin);
    if (existsSync(candidate)) return candidate;
  }
  return null;
}

function toolchainEnv(extra = {}) {
  const homeBin = join(process.env.HOME || "", ".deka/bin");
  return {
    ...process.env,
    PATH: [toolchainDir, homeBin, process.env.PATH || ""].join(delimiter),
    NO_COLOR: "1",
    ...extra,
  };
}

function run(cmd, args, opts = {}) {
  return new Promise((resolve) => {
    const child = spawn(cmd, args, {
      cwd: opts.cwd || here,
      env: toolchainEnv(opts.env || {}),
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (d) => {
      stdout += d;
    });
    child.stderr.on("data", (d) => {
      stderr += d;
    });
    child.on("error", (err) => {
      resolve({ code: 1, stdout: "", stderr: err.message, combined: err.message });
    });
    child.on("close", (code) => {
      resolve({ code: code ?? 1, stdout, stderr, combined: stdout + stderr });
    });
  });
}

async function runOk(cmd, args, opts = {}) {
  const result = await run(cmd, args, opts);
  if (result.code !== 0) {
    throw new Error(`${cmd} ${args.join(" ")}\n${result.combined}`);
  }
  return result;
}

function nowMs() {
  const t = process.hrtime.bigint();
  return Number(t) / 1e6;
}

function median(values) {
  const xs = [...values].sort((a, b) => a - b);
  const mid = Math.floor(xs.length / 2);
  return xs.length % 2 ? xs[mid] : (xs[mid - 1] + xs[mid]) / 2;
}

function rmrf(path) {
  rmSync(path, { recursive: true, force: true });
}

function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}

function resolveDeka() {
  const pinned = join(toolchainDir, "deka");
  if (existsSync(pinned)) return pinned;
  throw new Error(
    "bench/.toolchain/deka is missing. Build the CLI from main once:\n" +
      "  cargo build --release -p cli --features dev-server\n" +
      "  mkdir -p bench/.toolchain\n" +
      "  cp target/release/cli bench/.toolchain/deka\n" +
      "  cp \"$HOME/.deka/bin/dsc\" bench/.toolchain/dsc\n" +
      "deka rows use that main-build pending the next release; Vite rows are unchanged.",
  );
}

function resolveDsc() {
  const pinned = join(toolchainDir, "dsc");
  if (existsSync(pinned)) return pinned;
  const home = join(process.env.HOME || "", ".deka/bin/dsc");
  if (existsSync(home)) return home;
  const onPath = which("dsc");
  if (onPath) return onPath;
  throw new Error("dsc is required (released 0.52.2). Put it at bench/.toolchain/dsc or ~/.deka/bin/dsc");
}

async function versionText(bin, args = ["--version"]) {
  const result = await run(bin, args);
  return (result.stdout || result.stderr).trim() || "unknown";
}

async function machine() {
  const uname = await run("uname", ["-m"]);
  const arch = (uname.stdout || "").trim() || process.arch;
  if (process.platform === "darwin") {
    const sw = await run("sw_vers");
    const cpu = await run("sysctl", ["-n", "machdep.cpu.brand_string"]);
    const mem = await run("sysctl", ["-n", "hw.memsize"]);
    const bytes = Number((mem.stdout || "").trim());
    const swText = sw.stdout || "";
    const product = (swText.match(/ProductName:\s*(.+)/) || [])[1];
    const version = (swText.match(/ProductVersion:\s*(.+)/) || [])[1];
    return {
      os: product && version ? `${product.trim()} ${version.trim()}` : swText.replace(/\s+/g, " ").trim() || "darwin",
      arch,
      cpu: (cpu.stdout || "").trim() || "unknown",
      memoryGiB: Number.isFinite(bytes) ? Math.round(bytes / 1024 / 1024 / 1024) : 0,
      node: process.version,
    };
  }
  if (process.platform === "linux") {
    let os = "Linux";
    const lsb = await run("lsb_release", ["-ds"]);
    if (lsb.code === 0 && (lsb.stdout || "").trim()) {
      os = lsb.stdout.trim().replace(/^"|"$/g, "");
    } else {
      try {
        const text = readFileSync("/etc/os-release", "utf8");
        const pretty = (text.match(/^PRETTY_NAME=(.*)$/m) || [])[1];
        if (pretty) os = pretty.replace(/^"|"$/g, "");
      } catch {
        /* keep Linux */
      }
    }
    let cpu = "unknown";
    try {
      const info = readFileSync("/proc/cpuinfo", "utf8");
      cpu = ((info.match(/^model name\s*:\s*(.+)$/m) || [])[1] || "unknown").trim();
    } catch {
      /* keep unknown */
    }
    let memoryGiB = 0;
    try {
      const mem = readFileSync("/proc/meminfo", "utf8");
      const kb = Number((mem.match(/^MemTotal:\s+(\d+)/m) || [])[1]);
      if (Number.isFinite(kb)) memoryGiB = Math.round(kb / 1024 / 1024);
    } catch {
      /* keep 0 */
    }
    return { os, arch, cpu, memoryGiB, node: process.version };
  }
  return { os: process.platform, arch, cpu: "unknown", memoryGiB: 0, node: process.version };
}

async function timeCommand(cmd, args, cwd, env) {
  const start = nowMs();
  const result = await runOk(cmd, args, { cwd, env });
  return { ms: nowMs() - start, log: result.combined };
}

function dekaClean(root = join(here, "deka-blog")) {
  rmrf(join(root, "dist"));
  rmrf(join(root, ".cache"));
  rmrf(join(root, "ds_modules/.cache"));
  rmrf(join(root, ".deka-dist-stage"));
}

function viteClean() {
  const root = join(here, "vite-blog");
  rmrf(join(root, "dist"));
  rmrf(join(root, "node_modules/.vite"));
}

async function series(label, n, fn) {
  const samples = [];
  for (let i = 0; i < n; i++) {
    process.stderr.write(`  ${label} ${i + 1}/${n}\n`);
    samples.push(await fn(i));
  }
  return { samples, median: median(samples) };
}

async function waitHttp(url, timeoutMs = 60_000, server) {
  const start = Date.now();
  let last = "";
  while (Date.now() - start < timeoutMs) {
    if (server?.exited()) throw new Error(`Server exited before readiness: ${server.log()}`);
    try {
      const res = await fetch(url);
      await res.arrayBuffer();
      if (res.ok) return;
      last = `status ${res.status}`;
    } catch (err) {
      last = err instanceof Error ? err.message : String(err);
    }
    await sleep(100);
  }
  throw new Error(`server did not come up: ${url}${last ? ` (${last})` : ""}`);
}

const NEXT_ROOT = join(here, 'next-blog');
const NEXT_TOUCH = join(NEXT_ROOT, 'src/components/PostCard.tsx');
const POST_PATH = ISLANDS_PATH;
const env = { NEXT_TELEMETRY_DISABLED: '1' };
const resultsDir = join(here, 'results');
const cleanup = new Set();
const restoreFiles = new Map();
const buildLogs = {};

function startProcess(cmd, args, cwd, extraEnv = {}) {
  const proc = spawn(cmd, args, { cwd, env: toolchainEnv({ ...env, ...extraEnv }), stdio: ['ignore', 'pipe', 'pipe'], detached: process.platform !== 'win32' });
  let log = '';
  proc.stdout.on('data', d => { log += d; }); proc.stderr.on('data', d => { log += d; });
  proc.on('error', err => { log += err.message; });
  const kill = signal => {
    try { process.platform === 'win32' ? proc.kill(signal) : process.kill(-proc.pid, signal); } catch (err) { if (err.code !== 'ESRCH') throw err; }
  };
  const server = { log: () => log, exited: () => proc.exitCode !== null || proc.signalCode !== null, async stop() {
    cleanup.delete(server);
    if (proc.exitCode === null && proc.signalCode === null) {
      const exited = new Promise(resolve => proc.once('exit', resolve));
      kill('SIGTERM');
      await Promise.race([exited, sleep(3000)]);
      kill('SIGKILL');
      await exited;
    }
  } };
  cleanup.add(server);
  return server;
}

async function serving(command, cwd, port, extraEnv, fn) {
  await new Promise((resolve, reject) => {
    const probe = createSocketServer(); probe.once('error', reject);
    probe.listen(port, '127.0.0.1', () => probe.close(resolve));
  });
  const server = startProcess(command[0], command.slice(1), cwd, extraEnv);
  try {
    await waitHttp(`http://127.0.0.1:${port}/`, 120_000, server);
    return await fn(`http://127.0.0.1:${port}`);
  } catch (err) { throw new Error(`${err.stack}\nServer: ${command.join(' ')}\n${server.log()}`); }
  finally { await server.stop(); }
}

async function ttfb(url) {
  const agent = new Agent({ keepAlive: true, maxSockets: 1 });
  const sample = () => new Promise((resolve, reject) => {
    const start = nowMs();
    const req = get(url, { agent, headers: { 'Accept-Encoding': 'identity' } }, res => {
      const ms = nowMs() - start; // complete response headers, before reading body
      res.resume();
      res.on('end', () => res.statusCode === 200 ? resolve(ms) : reject(new Error(`TTFB status ${res.statusCode}`)));
      res.on('error', reject);
    });
    req.setTimeout(20_000, () => req.destroy(new Error('TTFB timeout'))); req.on('error', reject);
  });
  try {
    for (let i = 0; i < 5; i++) await sample();
    return await series('warm TTFB', 25, sample);
  } finally { agent.destroy(); }
}

async function builds(name, command, cwd, file, clean, extraEnv) {
  const original = readFileSync(file, 'utf8');
  restoreFiles.set(file, original);
  if (original.split('Read post</span>').length !== 2) throw new Error(`Expected one PostCard edit literal: ${file}`);
  const timed = async () => {
    const result = await timeCommand(command[0], command.slice(1), cwd, { ...env, ...extraEnv });
    (buildLogs[name] ||= []).push(result.log);
    return result.ms;
  };
  try {
    const cold = await series(`${name} cold`, RUNS, async () => { clean(); return timed(); });
    const incremental = await series(`${name} incremental`, RUNS, async i => {
      writeFileSync(file, original.replace("Read post</span>", `Read post ${i + 1}</span>`));
      return timed();
    });
    return { cold, incremental };
  } finally { writeFileSync(file, original); restoreFiles.delete(file); }
}

function zeroNextCopy(dest) {
  mkdirSync(dest, { recursive: true });
  for (const entry of readdirSync(NEXT_ROOT)) {
    if (['node_modules', '.next', '.bench-static'].includes(entry)) continue;
    cpSync(join(NEXT_ROOT, entry), join(dest, entry), { recursive: true });
  }
  // The copy lives inside next-blog; normal Node resolution finds its parent's
  // locked node_modules. No install or framework configuration differs.
  const layout = join(dest, 'src/app/layout.tsx');
  writeFileSync(layout, readFileSync(layout, 'utf8').replace('../components/Theme', '../components/StaticWidgets').replace('../components/Newsletter', '../components/StaticWidgets'));
  writeFileSync(join(dest, 'src/components/StaticWidgets.tsx'), `import type { ReactNode } from 'react';
export function ThemeProvider({ children }: { children: ReactNode }) { return children; }
export function ThemeToggle() { return <button type="button" id="theme-toggle" className="theme-toggle" data-theme="light" aria-label="Toggle color theme">Dark</button>; }
export function NewsletterSignup() { return <form className="nl-form" id="newsletter"><input id="nl-email" className="nl-input" type="email" name="email" placeholder="you@example.com" defaultValue="" /><button className="nl-btn" type="submit" id="nl-submit">Subscribe</button><p className="nl-status" id="nl-status">No tracking pixels. This form stays on the page.</p></form>; }
`);
}

function zeroDekaCopy(dest) {
  const src = join(here, 'deka-blog');
  cpSync(src, dest, { recursive: true, filter: from => !from.slice(src.length).split(/[/\\]/).some(p => ['dist', '.cache', '.deka-dist-stage', 'node_modules'].includes(p)) });
  const file = join(dest, 'src/ui/Shell.dsx');
  writeFileSync(file, readFileSync(file, 'utf8').replace(/[ \t]+client:load/g, ''));
}

async function hmr(name, command, root, file, port, extraEnv) {
  const original = readFileSync(file, 'utf8');
  restoreFiles.set(file, original);
  try {
    return await serving(command, root, port, extraEnv, base => withBrowser(async browser => {
      await browser.navigate(`${base}/`);
      await browser.until("document.querySelector('[data-bench-edit=card]')?.textContent === 'Read post'");
      await sleep(1500); // socket connection / initial route compilation outside timing
      const samples = [];
      for (let i = 0; i < RUNS; i++) {
        process.stderr.write(`  ${name} HMR ${i + 1}/${RUNS}\n`);
        const label = `Read post ${i + 1}`;
        samples.push(await editLatency(browser, file, original.replace('Read post</span>', `${label}</span>`), label, '[data-bench-edit=card]'));
        await sleep(500);
      }
      return { median: median(samples.map(s => s.ms)), samples, fullReloads: samples.filter(s => s.fullReload).length };
    }));
  } finally { writeFileSync(file, original); restoreFiles.delete(file); }
}

function table(report) {
  const { results, machine: m } = report;
  const rows = Object.entries(results);
  const ms = n => `${n.toFixed(2)} ms`;
  let out = `# deka-bench phase 2 results\n\nGenerated: ${report.generatedAt}\n\nMachine: ${m.os}, ${m.arch}, ${m.cpu}, ${m.memoryGiB} GiB, Node ${m.node}.\n\n`;
  out += `Toolchains (exact output and binary hashes in JSON):\n\n\`\`\`json\n${JSON.stringify(report.toolchain, null, 2)}\n\`\`\`\n\n`;
  out += '| stack | cold build (median 5) | incremental build (median 5) | warm TTFB (median 25) | component update (median 5) | full reloads |\n| --- | ---: | ---: | ---: | ---: | ---: |\n';
  for (const [name, r] of rows) out += `| ${name} | ${ms(r.build.cold.median)} | ${ms(r.build.incremental.median)} | ${ms(r.ttfb.median)} | ${ms(r.hmr.median)} | ${r.hmr.fullReloads}/5 |\n`;
  for (const [story, label] of [['static', 'A — no application client islands: Deka zero JS; Next static-page framework JS floor; Vite CSR'], ['interactive', 'B — Theme + Newsletter: Deka hydrated islands; Next SSG + client components; Vite CSR']]) {
    out += `\n## Payload story ${label}\n\nSame URL: \`${POST_PATH}\`. Fresh browser/cache; gzip-normalized bodies, not wire compression. RSC includes default Next link prefetches observed through network idle.\n\n| stack | HTML gzip | JS gzip | CSS gzip | RSC gzip | total gzip |\n| --- | ---: | ---: | ---: | ---: | ---: |\n`;
    for (const [name, r] of rows) { const p = r[story]; out += `| ${name} | ${p.htmlGzip} B | ${p.jsGzip} B | ${p.cssGzip} B | ${p.rscGzip} B | ${p.totalGzip} B |\n`; }
  }
  out += '\n## Chromium widget checks\n\n';
  for (const [name, r] of rows) out += `- ${name}: ${r.interactive.widgets.theme}; ${r.interactive.widgets.newsletter}\n`;
  out += '\nHMR uses the actual PostCard boundary each framework ships; any full reload is disclosed above. Content-edit HMR remains outside this phase’s component-edit task. Reproduce: `node bench/run.mjs`. See README for clock calibration, cold-cache definition, and serve commands.\n';
  return out;
}

async function main() {
  const dekaBin = resolveDeka(), dscBin = resolveDsc(); findChrome();
  const counts = ingest();
  for (const app of ['vite-blog', 'next-blog']) {
    if (!existsSync(join(here, app, 'node_modules'))) await runOk('npm', ['ci'], { cwd: join(here, app) });
  }
  const installed = {};
  for (const app of ['vite-blog', 'next-blog']) installed[app] = JSON.parse((await runOk('npm', ['ls', '--json', '--depth=0'], { cwd: join(here, app) })).stdout);
  const nextCli = join(NEXT_ROOT, 'node_modules/next/dist/bin/next');
  const viteCli = join(here, 'vite-blog/node_modules/vite/bin/vite.js');
  const dekaEnv = { DEKA_DSC: dscBin };
  const stacks = {
    deka: { root: join(here, 'deka-blog'), file: DEKA_TOUCH, build: [dekaBin, 'build', '--no-prompt'], clean: dekaClean, env: dekaEnv,
      prod: [dekaBin, 'serve', 'dist/server/serve-entry.js', '--port', '8760', '--no-prompt'], dev: [dekaBin, 'dev', '.', '--port', '8770', '--no-prompt'], port: 8760 },
    vite: { root: join(here, 'vite-blog'), file: VITE_TOUCH, build: [process.execPath, viteCli, 'build'], clean: viteClean,
      prod: [process.execPath, viteCli, 'preview', '--host', '127.0.0.1', '--port', '8761', '--strictPort'], dev: [process.execPath, viteCli, '--host', '127.0.0.1', '--port', '8771', '--strictPort'], port: 8761 },
    next: { root: NEXT_ROOT, file: NEXT_TOUCH, build: [process.execPath, nextCli, 'build'], clean: () => rmrf(join(NEXT_ROOT, '.next')),
      prod: [process.execPath, nextCli, 'start', '--hostname', '127.0.0.1', '--port', '8763'], dev: [process.execPath, nextCli, 'dev', '--hostname', '127.0.0.1', '--port', '8773'], port: 8763 },
  };
  const report = { generatedAt: '', machine: await machine(), content: counts, path: POST_PATH, toolchain: {
    note: 'Release build with dev-server from this PR, including app-router Content-Type and server-component reload fixes; not a released CLI.',
    deka: await versionText(dekaBin, ['--version', '--verbose']), dsc: await versionText(dscBin),
    chrome: await versionText(findChrome()), node: process.version, installed,
    vite: JSON.parse(readFileSync(join(here, 'vite-blog/package.json'))), next: JSON.parse(readFileSync(join(NEXT_ROOT, 'package.json'))),
    dekaSha256: createHash('sha256').update(readFileSync(dekaBin)).digest('hex'),
    dscSha256: createHash('sha256').update(readFileSync(dscBin)).digest('hex'),
    sourceCommit: (await runOk('git', ['rev-parse', 'HEAD'])).stdout.trim(),
  }, commands: {}, results: {} };
  for (const [name, stack] of Object.entries(stacks)) {
    report.commands[name] = { build: stack.build, prod: stack.prod, dev: stack.dev, cwd: stack.root };
    process.stderr.write(`${name}: production builds\n`);
    const build = await builds(name, stack.build, stack.root, stack.file, stack.clean, stack.env);
    // Incremental samples edited the source; serve a rebuilt committed baseline.
    await runOk(stack.build[0], stack.build.slice(1), { cwd: stack.root, env: { ...env, ...stack.env } });
    const measured = await serving(stack.prod, stack.root, stack.port, stack.env, async base => ({
      ttfb: await ttfb(`${base}${POST_PATH}`),
      interactive: await withBrowser(browser => payload(browser, `${base}${POST_PATH}`, true)),
    }));
    report.results[name] = { build, ...measured };
    if (name === 'vite') report.results[name].static = await serving(stack.prod, stack.root, stack.port, stack.env,
      base => withBrowser(browser => payload(browser, `${base}${POST_PATH}`, false)));
  }
  const zeroDeka = mkdtempSync(join(tmpdir(), 'deka-bench-static-'));
  const zeroNext = join(NEXT_ROOT, '.bench-static');
  try {
    zeroDekaCopy(zeroDeka);
    await runOk(dekaBin, ['build', '--no-prompt'], { cwd: zeroDeka, env: dekaEnv });
    report.results.deka.static = await serving([dekaBin, 'serve', 'dist/server/serve-entry.js', '--port', '8762', '--no-prompt'], zeroDeka, 8762, dekaEnv,
      base => withBrowser(browser => payload(browser, `${base}${POST_PATH}`, false)));
    rmrf(zeroNext); zeroNextCopy(zeroNext);
    await runOk(process.execPath, [nextCli, 'build'], { cwd: zeroNext, env });
    report.results.next.static = await serving([process.execPath, nextCli, 'start', '--hostname', '127.0.0.1', '--port', '8764'], zeroNext, 8764, env,
      base => withBrowser(browser => payload(browser, `${base}${POST_PATH}`, false)));
  } finally { rmrf(zeroDeka); rmrf(zeroNext); }
  if (report.results.deka.static.jsGzip !== 0) throw new Error('Deka no-island story unexpectedly shipped JS');
  if (report.results.next.static.jsGzip === 0) throw new Error('Next static-page JS floor was not captured');
  for (const [name, stack] of Object.entries(stacks)) {
    stack.clean(); // dev must not serve the preceding production artifact
    report.results[name].hmr = await hmr(name, stack.dev, stack.root, stack.file, stack.port + 10, stack.env);
  }
  report.generatedAt = new Date().toISOString();
  mkdirSync(resultsDir, { recursive: true });
  writeFileSync(join(here, 'last-results.json'), JSON.stringify(report, null, 2) + '\n');
  writeFileSync(join(resultsDir, 'phase2.json'), JSON.stringify(report, null, 2) + '\n');
  writeFileSync(join(resultsDir, 'phase2.md'), table(report));
  writeFileSync(join(resultsDir, 'build-logs.json'), JSON.stringify(buildLogs, null, 2) + '\n');
  process.stdout.write(table(report));
}

for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, async () => {
  for (const server of cleanup) await server.stop();
  for (const [file, original] of restoreFiles) writeFileSync(file, original);
  await stopBrowsers();
  process.exit(1);
});
main().catch(async err => {
  for (const server of cleanup) await server.stop();
  process.stderr.write(`${err.stack || err}\n`); process.exitCode = 1;
});
