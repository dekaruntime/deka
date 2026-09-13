#!/usr/bin/env node
/**
 * deka-bench phase 1 runner.
 *
 * Times cold + incremental production builds (median of 5) and measures
 * two gzipped payload stories: a post page with no islands, and a post
 * page with hydrated Theme + Newsletter islands. HMR, TTFB, and Next.js
 * are not measured here — see README.md.
 */
import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { gzipSync } from "node:zlib";
import {
  cpSync,
  existsSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  rmSync,
  statSync,
  utimesSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, extname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { ingest } from "./lib/ingest.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const toolchainDir = join(here, ".toolchain");
const RUNS = 5;
const ZERO_JS_PATH = "/posts/zero-js-by-default";
const ISLANDS_PATH = "/posts/islands-are-a-budget";
const DEKA_TOUCH = join(here, "deka-blog/src/ui/PostCard.dsx");
const VITE_TOUCH = join(here, "vite-blog/src/components/PostCard.tsx");

function which(bin) {
  const path = process.env.PATH || "";
  for (const dir of path.split(":")) {
    const candidate = join(dir, bin);
    if (existsSync(candidate)) return candidate;
  }
  return null;
}

function toolchainEnv(extra = {}) {
  const homeBin = join(process.env.HOME || "", ".deka/bin");
  return {
    ...process.env,
    PATH: `${toolchainDir}:${homeBin}:${process.env.PATH || ""}`,
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

function touch(path) {
  const now = new Date();
  utimesSync(path, now, now);
}

function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}

function resolveDeka() {
  const pinned = join(toolchainDir, "deka");
  if (existsSync(pinned)) return pinned;
  throw new Error(
    "bench/.toolchain/deka is missing. Build the CLI from main once:\n" +
      "  cargo build --release -p cli\n" +
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
    samples.push(await fn());
  }
  return { samples, median: median(samples) };
}

async function waitHttp(url, timeoutMs = 60_000) {
  const start = Date.now();
  let last = "";
  while (Date.now() - start < timeoutMs) {
    try {
      const res = await fetch(url);
      if (res.ok || res.status === 404) return;
      last = `status ${res.status}`;
    } catch (err) {
      last = err instanceof Error ? err.message : String(err);
    }
    await sleep(100);
  }
  throw new Error(`server did not come up: ${url}${last ? ` (${last})` : ""}`);
}

function contentTypeFor(file) {
  switch (extname(file)) {
    case ".html":
      return "text/html; charset=utf-8";
    case ".js":
      return "text/javascript; charset=utf-8";
    case ".css":
      return "text/css; charset=utf-8";
    case ".svg":
      return "image/svg+xml";
    default:
      return "application/octet-stream";
  }
}

function startStatic(root, port) {
  const server = createServer((req, res) => {
    const url = new URL(req.url || "/", `http://127.0.0.1:${port}`);
    let rel = decodeURIComponent(url.pathname);
    if (rel.endsWith("/")) rel += "index.html";
    const file = join(root, rel.replace(/^\/+/, ""));
    const index = join(root, "index.html");
    let path = file;
    if (!existsSync(path) || statSync(path).isDirectory()) {
      if (existsSync(index)) path = index;
      else {
        res.writeHead(404);
        res.end("not found");
        return;
      }
    }
    const body = readFileSync(path);
    res.writeHead(200, { "content-type": contentTypeFor(path) });
    res.end(body);
  });
  return new Promise((resolve) => {
    server.listen(port, "127.0.0.1", () => resolve(server));
  });
}

function collectAssetUrls(html, base) {
  const urls = new Set();
  const re = /<(?:script|link)[^>]+(?:src|href)=["']([^"']+)["']/gi;
  let m;
  while ((m = re.exec(html))) {
    const href = m[1];
    if (href.startsWith("data:") || href.startsWith("http://") || href.startsWith("https://")) continue;
    if (href.endsWith(".css") || href.endsWith(".js") || href.includes("/assets/")) {
      urls.add(new URL(href, base).href);
    }
  }
  return [...urls];
}

function jsAssetRefs(jsText, base) {
  const urls = [];
  const re = /["'](\/?assets\/[^"']+\.js)["']|["'](\.\/[^"']+\.js)["']/g;
  let m;
  while ((m = re.exec(jsText))) {
    const spec = m[1] || m[2];
    if (!spec) continue;
    urls.push(new URL(spec.startsWith("/") ? spec : "/assets/" + spec.replace(/^\.\//, ""), base).href);
  }
  return urls;
}

async function measurePayload(base, path) {
  const pageUrl = new URL(path, base).href;
  const htmlRes = await fetch(pageUrl);
  const htmlBuf = Buffer.from(await htmlRes.arrayBuffer());
  const html = htmlBuf.toString("utf8");
  const pending = collectAssetUrls(html, pageUrl);
  const seen = new Set();
  const assets = [];
  while (pending.length) {
    const url = pending.pop();
    if (seen.has(url)) continue;
    seen.add(url);
    const res = await fetch(url);
    if (!res.ok) continue;
    const buf = Buffer.from(await res.arrayBuffer());
    const kind = url.endsWith(".css") ? "css" : url.endsWith(".js") ? "js" : "other";
    if (kind === "other") continue;
    assets.push({ url, kind, raw: buf.length, gzip: gzipSync(buf).length });
    if (kind === "js") {
      for (const next of jsAssetRefs(buf.toString("utf8"), pageUrl)) {
        if (/AboutPage|TagPage/.test(next) && path.includes("/posts/")) continue;
        pending.push(next);
      }
    }
  }
  const htmlGzip = gzipSync(htmlBuf).length;
  const js = assets.filter((a) => a.kind === "js").reduce((n, a) => n + a.gzip, 0);
  const css = assets.filter((a) => a.kind === "css").reduce((n, a) => n + a.gzip, 0);
  return {
    htmlRaw: htmlBuf.length,
    htmlGzip,
    jsGzip: js,
    cssGzip: css,
    totalGzip: htmlGzip + js + css,
    hasIslandMarker: html.includes("data-deka-island"),
    hasIslandsScript: /assets\/islands(?:\.[^/]+)?\.js/.test(html),
    assets,
  };
}

function fmtMs(ms) {
  return `${ms.toFixed(0)} ms`;
}

function fmtBytes(n) {
  return `${n} B`;
}

function materializeZeroJsBlog(dest) {
  const src = join(here, "deka-blog");
  rmrf(dest);
  mkdirSync(dest, { recursive: true });
  cpSync(src, dest, {
    recursive: true,
    filter: (from) => {
      const rel = from.slice(src.length);
      return !rel.split(/[/\\]/).some((p) =>
        p === "dist" || p === ".cache" || p === ".deka-dist-stage" || p === "node_modules",
      );
    },
  });
  const shellPath = join(dest, "src/ui/Shell.dsx");
  const shell = readFileSync(shellPath, "utf8").replace(/[ \t]+client:load/g, "");
  if (shell.includes("client:load") || shell.includes("client:idle") || shell.includes("client:visible")) {
    throw new Error("zero-JS copy still contains a client: directive");
  }
  writeFileSync(shellPath, shell);
}

function spawnDekaServe(dekaBin, dscBin, cwd, port) {
  const proc = spawn(dekaBin, ["serve", ".", "--port", String(port), "--no-prompt"], {
    cwd,
    env: toolchainEnv({ DEKA_DSC: dscBin }),
    stdio: ["ignore", "pipe", "pipe"],
  });
  let log = "";
  proc.stderr.on("data", (d) => {
    log += d;
  });
  proc.stdout.on("data", (d) => {
    log += d;
  });
  return {
    proc,
    log: () => log,
    async stop() {
      if (proc.exitCode !== null || proc.signalCode !== null) return;
      proc.kill("SIGTERM");
      await Promise.race([
        new Promise((resolve) => proc.once("exit", resolve)),
        sleep(5_000),
      ]);
    },
  };
}

function findChrome() {
  const candidates = [
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    "/usr/bin/google-chrome",
    "/usr/bin/google-chrome-stable",
    "/usr/bin/chromium",
    "/usr/bin/chromium-browser",
    which("google-chrome"),
    which("google-chrome-stable"),
    which("chromium"),
    which("chromium-browser"),
  ];
  for (const p of candidates) {
    if (p && existsSync(p)) return p;
  }
  return null;
}

function cdpSend(ws, id, method, params = {}) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`cdp timeout: ${method}`)), 20_000);
    const onMessage = (event) => {
      const raw = typeof event.data === "string" ? event.data : event.data.toString();
      let msg;
      try {
        msg = JSON.parse(raw);
      } catch {
        return;
      }
      if (msg.id !== id) return;
      clearTimeout(timer);
      ws.removeEventListener("message", onMessage);
      if (msg.error) reject(new Error(`${method}: ${JSON.stringify(msg.error)}`));
      else resolve(msg.result);
    };
    ws.addEventListener("message", onMessage);
    ws.send(JSON.stringify({ id, method, params }));
  });
}

async function verifyToggleAndSubmit(pageUrl) {
  const chrome = findChrome();
  if (!chrome) return { ok: false, skipped: true, detail: "no chrome/chromium found" };
  const port = 9333;
  const profile = mkdtempSync(join(tmpdir(), "deka-bench-chrome-"));
  const proc = spawn(
    chrome,
    [
      "--headless=new",
      "--disable-gpu",
      "--no-first-run",
      "--no-default-browser-check",
      "--disable-dev-shm-usage",
      "--password-store=basic",
      `--remote-debugging-port=${port}`,
      `--user-data-dir=${profile}`,
      "about:blank",
    ],
    { stdio: ["ignore", "pipe", "pipe"] },
  );
  let ws;
  try {
    await waitHttp(`http://127.0.0.1:${port}/json/version`, 20_000);
    const listed = await fetch(`http://127.0.0.1:${port}/json/list`);
    const targets = await listed.json();
    const page = (Array.isArray(targets) ? targets : []).find((t) => t.webSocketDebuggerUrl && t.type === "page")
      || (Array.isArray(targets) ? targets : []).find((t) => t.webSocketDebuggerUrl);
    if (!page) throw new Error("chrome has no debuggable page target");
    ws = new WebSocket(page.webSocketDebuggerUrl);
    await new Promise((resolve, reject) => {
      ws.addEventListener("open", resolve);
      ws.addEventListener("error", () => reject(new Error("cdp websocket failed")));
    });
    await cdpSend(ws, 1, "Page.enable");
    await cdpSend(ws, 2, "Runtime.enable");
    const loadFired = new Promise((resolve) => {
      const timer = setTimeout(resolve, 15_000);
      const onMessage = (event) => {
        let msg;
        try {
          msg = JSON.parse(typeof event.data === "string" ? event.data : event.data.toString());
        } catch {
          return;
        }
        if (msg.method === "Page.loadEventFired") {
          clearTimeout(timer);
          ws.removeEventListener("message", onMessage);
          resolve();
        }
      };
      ws.addEventListener("message", onMessage);
    });
    await cdpSend(ws, 3, "Page.navigate", { url: pageUrl });
    await loadFired;
    let evalId = 20;
    async function evalExpr(expr) {
      const id = ++evalId;
      const result = await cdpSend(ws, id, "Runtime.evaluate", {
        expression: expr,
        awaitPromise: true,
        returnByValue: true,
      });
      if (result.exceptionDetails) {
        throw new Error(result.exceptionDetails.text || expr);
      }
      return result.result ? result.result.value : undefined;
    }
    let ready = false;
    for (let i = 0; i < 80; i++) {
      ready = Boolean(
        await evalExpr("!!(document.querySelector('#theme-toggle') && document.querySelector('#newsletter'))"),
      );
      if (ready) break;
      await sleep(200);
    }
    if (!ready) throw new Error("theme/newsletter widgets did not render");
    // module islands/SPA bundles evaluate around loadEventFired; give hydrateRoot a beat
    await sleep(400);
    const before = await evalExpr("document.querySelector('#theme-toggle').getAttribute('data-theme')");
    await evalExpr(
      "document.querySelector('#theme-toggle').dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true, view: window })); true",
    );
    let theme = before;
    for (let i = 0; i < 50; i++) {
      theme = await evalExpr("document.querySelector('#theme-toggle').getAttribute('data-theme')");
      if (theme && theme !== before) break;
      await sleep(100);
    }
    if (theme === before) throw new Error(`theme did not toggle (still ${String(theme)})`);
    await evalExpr(`(() => {
      const input = document.querySelector('#nl-email');
      const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
      setter.call(input, 'ava@deka.gg');
      input.dispatchEvent(new Event('input', { bubbles: true }));
      input.dispatchEvent(new Event('change', { bubbles: true }));
      return input.value;
    })()`);
    await evalExpr(`(() => {
      const form = document.querySelector('#newsletter');
      if (form.requestSubmit) form.requestSubmit();
      else document.querySelector('#nl-submit').click();
      return true;
    })()`);
    let status = "";
    for (let i = 0; i < 50; i++) {
      status = (await evalExpr("document.querySelector('#nl-status')?.textContent || ''")) || "";
      if (status.includes("ava@deka.gg")) break;
      await sleep(100);
    }
    if (!status.includes("ava@deka.gg")) throw new Error(`newsletter did not update: ${status}`);
    return { ok: true, skipped: false, detail: `theme ${before}->${theme}; ${status.trim()}` };
  } catch (err) {
    return { ok: false, skipped: false, detail: err instanceof Error ? err.message : String(err) };
  } finally {
    try {
      if (ws && ws.readyState === WebSocket.OPEN) ws.close();
    } catch {
      /* ignore */
    }
    if (proc.exitCode === null && proc.signalCode === null) proc.kill("SIGKILL");
    await sleep(300);
    try {
      rmrf(profile);
    } catch {
      /* chrome may still hold the profile directory */
    }
  }
}

async function main() {
  const dekaBin = resolveDeka();
  const dscBin = resolveDsc();
  process.stderr.write("ingest content\n");
  const counts = ingest();
  const specs = await machine();
  const dekaVer = await versionText(dekaBin, ["--version", "--verbose"]);
  const dscVer = (await versionText(dscBin)).split("\n")[0];
  const npm = which("npm");
  if (!npm) throw new Error("npm is required to install vite-blog dependencies");

  if (!existsSync(join(here, "vite-blog/node_modules"))) {
    process.stderr.write("npm install (vite-blog, untimed)\n");
    await runOk("npm", ["install"], { cwd: join(here, "vite-blog") });
  }

  const dekaEnv = { DEKA_DSC: dscBin };

  process.stderr.write("deka cold builds\n");
  const dekaCold = await series("deka cold", RUNS, async () => {
    dekaClean();
    const t = await timeCommand(dekaBin, ["build", "--no-prompt"], join(here, "deka-blog"), dekaEnv);
    return t.ms;
  });

  process.stderr.write("deka incremental builds\n");
  const dekaInc = await series("deka incremental", RUNS, async () => {
    touch(DEKA_TOUCH);
    const t = await timeCommand(dekaBin, ["build", "--no-prompt"], join(here, "deka-blog"), dekaEnv);
    return t.ms;
  });

  process.stderr.write("vite cold builds\n");
  const viteCold = await series("vite cold", RUNS, async () => {
    viteClean();
    const t = await timeCommand("npx", ["vite", "build"], join(here, "vite-blog"));
    return t.ms;
  });

  process.stderr.write("vite incremental builds\n");
  const viteInc = await series("vite incremental", RUNS, async () => {
    touch(VITE_TOUCH);
    const t = await timeCommand("npx", ["vite", "build"], join(here, "vite-blog"));
    return t.ms;
  });

  process.stderr.write("payload: deka islands app\n");
  const dekaPort = 8760;
  const vitePort = 8761;
  const zeroPort = 8762;
  const islandsServe = spawnDekaServe(dekaBin, dscBin, join(here, "deka-blog"), dekaPort);
  try {
    await waitHttp(`http://127.0.0.1:${dekaPort}/`);
  } catch (err) {
    throw new Error(`${err.message}\n${islandsServe.log()}`);
  }
  const dekaIslands = await measurePayload(`http://127.0.0.1:${dekaPort}`, ISLANDS_PATH);
  process.stderr.write("chromium: deka islands\n");
  const dekaBrowser = await verifyToggleAndSubmit(`http://127.0.0.1:${dekaPort}${ISLANDS_PATH}`);
  await islandsServe.stop();
  if (!dekaBrowser.skipped && !dekaBrowser.ok) {
    throw new Error(`deka Chromium islands check failed: ${dekaBrowser.detail}`);
  }

  process.stderr.write("payload: deka zero-JS copy (client: directives stripped)\n");
  const zeroRoot = join(tmpdir(), `deka-bench-zero-${process.pid}`);
  materializeZeroJsBlog(zeroRoot);
  const zeroServe = spawnDekaServe(dekaBin, dscBin, zeroRoot, zeroPort);
  let dekaZero;
  try {
    try {
      await waitHttp(`http://127.0.0.1:${zeroPort}/`);
    } catch (err) {
      throw new Error(`${err.message}\n${zeroServe.log()}`);
    }
    dekaZero = await measurePayload(`http://127.0.0.1:${zeroPort}`, ZERO_JS_PATH);
  } finally {
    await zeroServe.stop();
    rmrf(zeroRoot);
  }

  process.stderr.write("payload: vite static\n");
  const viteServer = await startStatic(join(here, "vite-blog/dist"), vitePort);
  const viteZero = await measurePayload(`http://127.0.0.1:${vitePort}`, ZERO_JS_PATH);
  const viteIslands = await measurePayload(`http://127.0.0.1:${vitePort}`, ISLANDS_PATH);
  process.stderr.write("chromium: vite csr\n");
  const viteBrowser = await verifyToggleAndSubmit(`http://127.0.0.1:${vitePort}${ISLANDS_PATH}`);
  viteServer.close();
  if (!viteBrowser.skipped && !viteBrowser.ok) {
    throw new Error(`vite Chromium check failed: ${viteBrowser.detail}`);
  }

  const report = {
    generatedAt: new Date().toISOString(),
    machine: specs,
    toolchain: {
      deka: dekaVer,
      dsc: dscVer,
      node: process.version,
      dekaBinary: dekaBin,
      dscBinary: dscBin,
      note: "deka rows run on a main-build pending the next release; Vite rows use the pinned npm toolchain",
    },
    content: counts,
    paths: { zeroJs: ZERO_JS_PATH, islands: ISLANDS_PATH },
    chromium: { deka: dekaBrowser, vite: viteBrowser },
    results: {
      deka: {
        coldBuildMs: dekaCold.median,
        incrementalBuildMs: dekaInc.median,
        zeroJs: dekaZero,
        islands: dekaIslands,
      },
      vite: {
        coldBuildMs: viteCold.median,
        incrementalBuildMs: viteInc.median,
        zeroJs: viteZero,
        islands: viteIslands,
      },
    },
    samples: {
      dekaCold: dekaCold.samples,
      dekaIncremental: dekaInc.samples,
      viteCold: viteCold.samples,
      viteIncremental: viteInc.samples,
    },
  };

  writeFileSync(join(here, "last-results.json"), JSON.stringify(report, null, 2) + "\n");

  const table = `
# deka-bench phase 1 results

Generated: ${report.generatedAt}

## Machine

- os: ${specs.os}
- arch: ${specs.arch}
- cpu: ${specs.cpu}
- memory: ${specs.memoryGiB} GiB
- node: ${specs.node}
- deka: main-build (pending next release)
- deka --version --verbose:
${dekaVer.split("\n").map((l) => `  ${l}`).join("\n")}
- dsc: ${dscVer}
- deka binary: ${dekaBin}
- dsc binary: ${dscBin}

## Build (median of ${RUNS}, production)

| stack | cold build | incremental build |
| --- | ---: | ---: |
| deka | ${fmtMs(dekaCold.median)} | ${fmtMs(dekaInc.median)} |
| Vite + React 19.1.1 | ${fmtMs(viteCold.median)} | ${fmtMs(viteInc.median)} |
| Next.js App Router | TODO (phase 2) | TODO (phase 2) |

## Payload story A — zero-JS static page (\`${ZERO_JS_PATH}\`)

deka ships no client JS. Vite is a CSR SPA on the same URL.

| stack | HTML gzip | JS gzip | CSS gzip | total gzip |
| --- | ---: | ---: | ---: | ---: |
| deka (no islands, no client JS) | ${fmtBytes(dekaZero.htmlGzip)} | ${fmtBytes(dekaZero.jsGzip)} | ${fmtBytes(dekaZero.cssGzip)} | ${fmtBytes(dekaZero.totalGzip)} |
| Vite + React 19.1.1 (CSR SPA) | ${fmtBytes(viteZero.htmlGzip)} | ${fmtBytes(viteZero.jsGzip)} | ${fmtBytes(viteZero.cssGzip)} | ${fmtBytes(viteZero.totalGzip)} |
| Next.js App Router | TODO (phase 2) | TODO (phase 2) | TODO (phase 2) | TODO (phase 2) |

## Payload story B — hydrated islands (\`${ISLANDS_PATH}\`)

deka: per-page HTML + cached shared islands runtime (production React + Theme + Newsletter). Vite: CSR SPA. This is not a "27x smaller" claim.

| stack | HTML gzip | JS gzip | CSS gzip | total gzip |
| --- | ---: | ---: | ---: | ---: |
| deka (HTML + shared islands runtime) | ${fmtBytes(dekaIslands.htmlGzip)} | ${fmtBytes(dekaIslands.jsGzip)} | ${fmtBytes(dekaIslands.cssGzip)} | ${fmtBytes(dekaIslands.totalGzip)} |
| Vite + React 19.1.1 (CSR SPA) | ${fmtBytes(viteIslands.htmlGzip)} | ${fmtBytes(viteIslands.jsGzip)} | ${fmtBytes(viteIslands.cssGzip)} | ${fmtBytes(viteIslands.totalGzip)} |
| Next.js App Router | TODO (phase 2) | TODO (phase 2) | TODO (phase 2) | TODO (phase 2) |

## Chromium (toggle + submit)

| stack | result |
| --- | --- |
| deka islands | ${dekaBrowser.skipped ? `skipped (${dekaBrowser.detail})` : dekaBrowser.ok ? `ok — ${dekaBrowser.detail}` : `FAIL — ${dekaBrowser.detail}`} |
| Vite + React 19.1.1 | ${viteBrowser.skipped ? `skipped (${viteBrowser.detail})` : viteBrowser.ok ? `ok — ${viteBrowser.detail}` : `FAIL — ${viteBrowser.detail}`} |

## Not measured yet

| metric | status |
| --- | --- |
| HMR (component edit) | TODO (phase 2) — CDP timestamps |
| HMR (content edit) | TODO (phase 2) — CDP timestamps |
| TTFB | TODO (phase 2) |
| Next.js subject app | TODO (phase 2) |

Reproduce: \`node bench/run.mjs\`
`;

  process.stdout.write(table.trim() + "\n");
}

main().catch((err) => {
  process.stderr.write(String(err.stack || err) + "\n");
  process.exit(1);
});
