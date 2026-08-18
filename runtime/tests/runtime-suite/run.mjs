#!/usr/bin/env bun
// Headless runtime execution harness for DekaScript.
// Compiles every fixture through both the native CLI and the browser WASM
// compiler, runs the emitted JS against the Deka runtime globals, and asserts
// on stdout / compile diagnostics.

import { readFile, mkdtemp, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";
import vm from "node:vm";
import { createRuntimeGlobals } from "./runtime-globals.mjs";

const require = createRequire(import.meta.url);
const __dirname = dirname(fileURLToPath(import.meta.url));
const fixturesDir = join(__dirname, "fixtures");
const fixturesJsonPath = join(__dirname, "fixtures.json");

function findCliBinary() {
  const candidates = [
    join(__dirname, "..", "..", "target", "release", "cli"),
    join(__dirname, "..", "..", "target", "debug", "cli"),
  ];
  for (const candidate of candidates) {
    try {
      const stat = require("node:fs").statSync(candidate);
      if (stat.isFile() && (stat.mode & 0o111)) return candidate;
    } catch {}
  }
  return null;
}

function findWasmArtifact() {
  const candidates = [
    join(__dirname, "..", "..", "target", "wasm32-unknown-unknown", "release", "phpx_compiler_wasm.wasm"),
    join(__dirname, "..", "..", "dist", "deka-compiler-wasm", "deka_compiler.wasm"),
  ];
  let newest = null;
  let newestMtime = 0;
  for (const candidate of candidates) {
    try {
      const stat = require("node:fs").statSync(candidate);
      if (stat.isFile() && stat.mtimeMs > newestMtime) {
        newest = candidate;
        newestMtime = stat.mtimeMs;
      }
    } catch {}
  }
  return newest;
}

function compileNative(sourcePath, cliBinary) {
  const outPath = join(tmpdir(), `deka-runtime-test-${Date.now()}-${Math.random().toString(36).slice(2)}.js`);
  const result = spawnSync(cliBinary, ["transpile", sourcePath, "--out", outPath], {
    encoding: "utf-8",
    timeout: 60_000,
  });
  if (result.status !== 0) {
    return { ok: false, error: result.stderr || result.stdout || "native transpile failed" };
  }
  try {
    const code = require("node:fs").readFileSync(outPath, "utf-8");
    require("node:fs").unlinkSync(outPath);
    return { ok: true, code };
  } catch (error) {
    return { ok: false, error: String(error) };
  }
}

async function loadWasmArtifact(artifactPath) {
  const bytes = await readFile(artifactPath);
  const { instance } = await WebAssembly.instantiate(bytes);
  return instance.exports;
}

function compileWasm(source, filename, wasmExports) {
  const e = wasmExports;

  function write(value) {
    const bytes = new TextEncoder().encode(value);
    const ptr = e.deka_compiler_alloc(bytes.length);
    new Uint8Array(e.memory.buffer, ptr, bytes.length).set(bytes);
    return [ptr, bytes.length];
  }

  const [sourcePtr, sourceLen] = write(source);
  const [filenamePtr, filenameLen] = write(filename);
  const [modePtr, modeLen] = write("deka");
  const resultPtr = e.deka_compiler_compile(sourcePtr, sourceLen, filenamePtr, filenameLen, modePtr, modeLen);
  const header = new DataView(e.memory.buffer, resultPtr, 8);
  const jsonPtr = header.getUint32(0, true);
  const jsonLen = header.getUint32(4, true);
  const response = JSON.parse(new TextDecoder().decode(new Uint8Array(e.memory.buffer, jsonPtr, jsonLen)));
  e.deka_compiler_free(sourcePtr, sourceLen);
  e.deka_compiler_free(filenamePtr, filenameLen);
  e.deka_compiler_free(modePtr, modeLen);
  e.deka_compiler_free(resultPtr, 8 + jsonLen);
  return response;
}

function stripModuleArtifacts(code) {
  return code
    .replace(/^export const \w+ = [^;]+;\n?/gm, "")
    .replace(/^export async function \w+[\s\S]*$/m, "")
    .replace(/^export function \w+[\s\S]*$/m, "")
    .replace(/^import\s+\{\s*jsx,\s*jsxs\s*\}\s+from\s+['"]component\/core['"];?\n?/gm, "");
}

async function runCompiledJs(code) {
  const cleaned = stripModuleArtifacts(code);
  const { globals, output, errorOutput } = createRuntimeGlobals();
  const context = vm.createContext(globals);
  const script = new vm.Script(`(async () => { ${cleaned} })()`);
  const promise = script.runInContext(context, { timeout: 10_000 });
  if (promise && typeof promise.then === "function") {
    await promise;
  }
  // JSX/PHPX templates write rendered output to __phpxCurrentResponse.body.
  const templateBody = context.__phpxCurrentResponse?.body;
  return {
    stdout: output.join("") || (typeof templateBody === "string" ? templateBody : ""),
    stderr: errorOutput.join(""),
  };
}

function normalizeWhitespace(value) {
  return value.replace(/\r\n/g, "\n");
}

async function runFixture(fixture, cliBinary, wasmExports) {
  const sourcePath = join(fixturesDir, fixture.file);
  const source = await readFile(sourcePath, "utf-8");

  const results = {
    name: fixture.name,
    file: fixture.file,
    xfail: fixture.xfail || null,
    native: { passed: false, error: null, stdout: "", stderr: "" },
    wasm: { passed: false, error: null, stdout: "", stderr: "" },
  };

  // Native path
  const nativeCompile = compileNative(sourcePath, cliBinary);
  if (!nativeCompile.ok) {
    results.native.error = nativeCompile.error;
    if (fixture.expectCompile) {
      results.native.error = `compile failed: ${nativeCompile.error}`;
    } else {
      const msg = nativeCompile.error;
      if (msg.includes(fixture.expectError)) {
        results.native.passed = true;
      } else {
        results.native.error = `expected diagnostic containing "${fixture.expectError}", got: ${msg}`;
      }
    }
  } else if (!fixture.expectCompile) {
    results.native.error = `expected compile failure containing "${fixture.expectError}", but it succeeded`;
  } else {
    try {
      const run = await runCompiledJs(nativeCompile.code);
      results.native.stdout = run.stdout;
      results.native.stderr = run.stderr;
      if (normalizeWhitespace(run.stdout) === normalizeWhitespace(fixture.expectStdout)) {
        results.native.passed = true;
      } else {
        results.native.error = `stdout mismatch:\n  expected: ${JSON.stringify(fixture.expectStdout)}\n  actual:   ${JSON.stringify(run.stdout)}`;
      }
    } catch (error) {
      results.native.error = `runtime error: ${error.message || error}`;
    }
  }

  // WASM path
  const wasmResponse = compileWasm(source, fixture.file, wasmExports);
  if (!wasmResponse.ok) {
    if (fixture.expectCompile) {
      results.wasm.error = `compile failed: ${JSON.stringify(wasmResponse.diagnostics)}`;
    } else {
      const diagnostic = (wasmResponse.diagnostics || []).map((d) => d.message).join("\n");
      if (diagnostic.includes(fixture.expectError)) {
        results.wasm.passed = true;
      } else {
        results.wasm.error = `expected diagnostic containing "${fixture.expectError}", got: ${diagnostic || JSON.stringify(wasmResponse)}`;
      }
    }
  } else if (!fixture.expectCompile) {
    results.wasm.error = `expected compile failure containing "${fixture.expectError}", but it succeeded`;
  } else {
    try {
      const run = await runCompiledJs(wasmResponse.output.code);
      results.wasm.stdout = run.stdout;
      results.wasm.stderr = run.stderr;
      if (normalizeWhitespace(run.stdout) === normalizeWhitespace(fixture.expectStdout)) {
        results.wasm.passed = true;
      } else {
        results.wasm.error = `stdout mismatch:\n  expected: ${JSON.stringify(fixture.expectStdout)}\n  actual:   ${JSON.stringify(run.stdout)}`;
      }
    } catch (error) {
      results.wasm.error = `runtime error: ${error.message || error}`;
    }
  }

  return results;
}

function printResults(results) {
  console.log("\n============================================================");
  console.log(" DekaScript Runtime Execution Test Suite");
  console.log("============================================================\n");

  let passed = 0;
  let failed = 0;
  let xfailed = 0;
  let xpassed = 0;

  for (const result of results) {
    const nativeIcon = result.native.passed ? "✓" : "✗";
    const wasmIcon = result.wasm.passed ? "✓" : "✗";
    const ok = result.native.passed && result.wasm.passed;
    const xfail = result.xfail;

    if (xfail) {
      if (ok) {
        xpassed++;
        console.log(`! native  ! wasm   ${result.name} (UNEXPECTED PASS — xfail: ${xfail})`);
      } else {
        xfailed++;
        console.log(`~ native  ~ wasm   ${result.name} (expected failure: ${xfail})`);
      }
      continue;
    }

    if (ok) {
      passed++;
      console.log(`${nativeIcon} native  ${wasmIcon} wasm   ${result.name}`);
    } else {
      failed++;
      console.log(`${nativeIcon} native  ${wasmIcon} wasm   ${result.name}`);
      if (!result.native.passed) {
        console.log(`  native: ${result.native.error}`);
      }
      if (!result.wasm.passed) {
        console.log(`  wasm:   ${result.wasm.error}`);
      }
    }
  }

  console.log("\n============================================================");
  console.log(` Passed: ${passed} | Failed: ${failed} | Expected failures: ${xfailed} | Unexpected passes: ${xpassed} | Total: ${results.length}`);
  console.log("============================================================\n");

  return failed === 0 && xpassed === 0;
}

function parseArgs(argv) {
  const args = { list: false, filter: null, help: false };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--list" || arg === "-l") {
      args.list = true;
    } else if (arg === "--filter" || arg === "-f") {
      args.filter = argv[++i] || "";
    } else if (arg === "--help" || arg === "-h") {
      args.help = true;
    }
  }
  return args;
}

function printUsage() {
  console.log(`usage: bun run.mjs [options]

options:
  -l, --list           List all fixtures and exit
  -f, --filter <glob>  Run only fixtures whose name matches the substring
  -h, --help           Show this help

examples:
  bun tests/runtime-suite/run.mjs
  bun tests/runtime-suite/run.mjs --list
  bun tests/runtime-suite/run.mjs --filter structs`);
}

function listFixtures(fixtures) {
  console.log("\nDekaScript runtime fixtures:");
  for (const fixture of fixtures) {
    const xfail = fixture.xfail ? " [xfail]" : "";
    console.log(`  - ${fixture.name}${xfail}`);
  }
  console.log(`\nTotal: ${fixtures.length}`);
}

async function main() {
  const args = parseArgs(process.argv.slice(2));

  if (args.help) {
    printUsage();
    process.exit(0);
  }

  const fixtures = JSON.parse(await readFile(fixturesJsonPath, "utf-8"));

  if (args.list) {
    listFixtures(fixtures);
    process.exit(0);
  }

  const filtered = args.filter
    ? fixtures.filter((f) => f.name.toLowerCase().includes(args.filter.toLowerCase()))
    : fixtures;

  if (filtered.length === 0) {
    console.error(`error: no fixtures match filter "${args.filter}"`);
    process.exit(1);
  }

  const cliBinary = findCliBinary();
  if (!cliBinary) {
    console.error("error: could not find deka CLI binary (build with: cd runtime && cargo build --release -p cli)");
    process.exit(1);
  }

  const wasmArtifact = findWasmArtifact();
  if (!wasmArtifact) {
    console.error("error: could not find deka-compiler.wasm artifact (build with: cd runtime && scripts/build-deka-compiler-wasm.sh)");
    process.exit(1);
  }

  const wasmExports = await loadWasmArtifact(wasmArtifact);

  // Validate required ABI exports.
  for (const name of ["deka_compiler_alloc", "deka_compiler_compile", "deka_compiler_free"]) {
    if (!(name in wasmExports)) {
      console.error(`error: wasm artifact missing required export: ${name}`);
      process.exit(1);
    }
  }

  const results = [];
  for (const fixture of filtered) {
    results.push(await runFixture(fixture, cliBinary, wasmExports));
  }

  const ok = printResults(results);
  process.exit(ok ? 0 : 1);
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
