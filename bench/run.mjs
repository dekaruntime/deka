#!/usr/bin/env node
/**
 * deka-bench phase 1 runner.
 *
 * Times cold + incremental production builds (median of 5) and measures
 * gzipped HTML+JS+CSS bytes for one post page. HMR, TTFB, and Next.js are
 * not measured here — see README.md.
 */
import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { gzipSync } from "node:zlib";
import {
  existsSync,
  mkdtempSync,
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
const RUNS = 5;
const PAYLOAD_PATH = "/posts/zero-js-by-default";
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

function run(cmd, args, opts = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(cmd, args, {
      cwd: opts.cwd || here,
      env: { ...process.env, PATH: `${process.env.HOME}/.deka/bin:${process.env.PATH || ""}`, ...(opts.env || {}) },
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
    child.on("error", reject);
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

async function versionOf(bin, args = ["--version"]) {
  const found = which(bin) || join(process.env.HOME || "", ".deka/bin", bin);
  if (!existsSync(found) && !which(bin)) return "missing";
  const cmd = which(bin) || found;
  const result = await run(cmd, args);
  return (result.stdout || result.stderr).trim().split("\n")[0] || "unknown";
}

async function machine() {
  const uname = await run("uname", ["-m"]);
  const sw = await run("sw_vers");
  const cpu = await run("sysctl", ["-n", "machdep.cpu.brand_string"]);
  const mem = await run("sysctl", ["-n", "hw.memsize"]);
  const bytes = Number((mem.stdout || "").trim());
  const swText = sw.stdout || "";
  const product = (swText.match(/ProductName:\s*(.+)/) || [])[1];
  const version = (swText.match(/ProductVersion:\s*(.+)/) || [])[1];
  return {
    os: product && version ? `${product.trim()} ${version.trim()}` : swText.replace(/\s+/g, " ").trim() || process.platform,
    arch: (uname.stdout || "").trim() || process.arch,
    cpu: (cpu.stdout || "").trim() || "unknown",
    memoryGiB: Number.isFinite(bytes) ? Math.round(bytes / 1024 / 1024 / 1024) : 0,
    node: process.version,
  };
}

async function timeCommand(cmd, args, cwd) {
  const start = nowMs();
  const result = await runOk(cmd, args, { cwd });
  return { ms: nowMs() - start, log: result.combined };
}

function dekaClean() {
  const root = join(here, "deka-blog");
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

async function waitHttp(url, timeoutMs = 15000) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    try {
      const res = await fetch(url);
      if (res.ok || res.status === 404) return;
    } catch {
      /* retry */
    }
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error(`server did not come up: ${url}`);
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
    assets,
  };
}

function fmtMs(ms) {
  return `${ms.toFixed(0)} ms`;
}

function fmtBytes(n) {
  return `${n} B`;
}

async function main() {
  const dekaHome = join(process.env.HOME || "", ".deka/bin");
  process.env.PATH = `${dekaHome}:${process.env.PATH || ""}`;
  process.stderr.write("ingest content\n");
  const counts = ingest();
  const dekaBin = which("deka") || join(process.env.HOME || "", ".deka/bin/deka");
  const dscBin = which("dsc") || join(process.env.HOME || "", ".deka/bin/dsc");
  if (!existsSync(dekaBin) || !existsSync(dscBin)) {
    throw new Error("deka 0.52.0 and dsc 0.52.2 must be on PATH (see bench/README.md)");
  }
  const specs = await machine();
  const dekaVer = await versionOf("deka");
  const dscVer = await versionOf("dsc");
  const npm = which("npm");
  if (!npm) throw new Error("npm is required to install vite-blog dependencies");

  if (!existsSync(join(here, "vite-blog/node_modules"))) {
    process.stderr.write("npm install (vite-blog, untimed)\n");
    await runOk("npm", ["install"], { cwd: join(here, "vite-blog") });
  }

  process.stderr.write("deka cold builds\n");
  const dekaCold = await series("deka cold", RUNS, async () => {
    dekaClean();
    const t = await timeCommand(dekaBin, ["build", "--no-prompt"], join(here, "deka-blog"));
    return t.ms;
  });

  process.stderr.write("deka incremental builds\n");
  const dekaInc = await series("deka incremental", RUNS, async () => {
    touch(DEKA_TOUCH);
    const t = await timeCommand(dekaBin, ["build", "--no-prompt"], join(here, "deka-blog"));
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

  process.stderr.write("payload: deka serve + vite static\n");
  const dekaPort = 8760;
  const vitePort = 8761;
  const dekaProc = spawn(dekaBin, ["serve", "--port", String(dekaPort), "--no-prompt"], {
    cwd: join(here, "deka-blog"),
    env: { ...process.env, PATH: `${process.env.HOME}/.deka/bin:${process.env.PATH || ""}` },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let dekaLog = "";
  dekaProc.stderr.on("data", (d) => {
    dekaLog += d;
  });
  dekaProc.stdout.on("data", (d) => {
    dekaLog += d;
  });
  await waitHttp(`http://127.0.0.1:${dekaPort}/`);
  const dekaPayload = await measurePayload(`http://127.0.0.1:${dekaPort}`, PAYLOAD_PATH);
  dekaProc.kill("SIGTERM");

  const viteServer = await startStatic(join(here, "vite-blog/dist"), vitePort);
  const vitePayload = await measurePayload(`http://127.0.0.1:${vitePort}`, PAYLOAD_PATH);
  viteServer.close();

  const report = {
    generatedAt: new Date().toISOString(),
    machine: specs,
    toolchain: { deka: dekaVer, dsc: dscVer, node: process.version },
    content: counts,
    payloadPath: PAYLOAD_PATH,
    results: {
      deka: {
        coldBuildMs: dekaCold.median,
        incrementalBuildMs: dekaInc.median,
        payload: dekaPayload,
      },
      vite: {
        coldBuildMs: viteCold.median,
        incrementalBuildMs: viteInc.median,
        payload: vitePayload,
      },
    },
    samples: {
      dekaCold: dekaCold.samples,
      dekaIncremental: dekaInc.samples,
      viteCold: viteCold.samples,
      viteIncremental: viteInc.samples,
    },
  };

  const outDir = mkdtempSync(join(tmpdir(), "deka-bench-"));
  const jsonPath = join(here, "last-results.json");
  writeFileSync(jsonPath, JSON.stringify(report, null, 2) + "\n");

  const table = `
# deka-bench phase 1 results

Generated: ${report.generatedAt}

## Machine

- os: ${specs.os}
- arch: ${specs.arch}
- cpu: ${specs.cpu}
- memory: ${specs.memoryGiB} GiB
- node: ${specs.node}
- deka: ${dekaVer}
- dsc: ${dscVer}

## Build (median of ${RUNS}, production)

| stack | cold build | incremental build |
| --- | ---: | ---: |
| deka | ${fmtMs(dekaCold.median)} | ${fmtMs(dekaInc.median)} |
| Vite + React 19.1.1 | ${fmtMs(viteCold.median)} | ${fmtMs(viteInc.median)} |
| Next.js App Router | TODO (phase 2) | TODO (phase 2) |

## Payload for \`${PAYLOAD_PATH}\` (gzipped HTML+JS+CSS)

| stack | HTML gzip | JS gzip | CSS gzip | total gzip |
| --- | ---: | ---: | ---: | ---: |
| deka | ${fmtBytes(dekaPayload.htmlGzip)} | ${fmtBytes(dekaPayload.jsGzip)} | ${fmtBytes(dekaPayload.cssGzip)} | ${fmtBytes(dekaPayload.totalGzip)} |
| Vite + React 19.1.1 | ${fmtBytes(vitePayload.htmlGzip)} | ${fmtBytes(vitePayload.jsGzip)} | ${fmtBytes(vitePayload.cssGzip)} | ${fmtBytes(vitePayload.totalGzip)} |
| Next.js App Router | TODO (phase 2) | TODO (phase 2) | TODO (phase 2) | TODO (phase 2) |

## Not measured yet

| metric | status |
| --- | --- |
| HMR (component edit) | TODO (phase 2) — CDP timestamps |
| HMR (content edit) | TODO (phase 2) — CDP timestamps |
| TTFB | TODO (phase 2) |
| Next.js subject app | TODO (phase 2) |

Reproduce: \`node bench/run.mjs\`
scratch: ${outDir}
`;

  process.stdout.write(table.trim() + "\n");
  void dekaLog;
}

main().catch((err) => {
  process.stderr.write(String(err.stack || err) + "\n");
  process.exit(1);
});
