#!/usr/bin/env bun
// Native Hats runner. Loads tests/testsuite/<category>/<name>/ and executes
// each fixture with `deka run` (target/release/cli or DEKA_NATIVE).
// Browser/WASM stays the live playground on testsuite.deka.gg; this command
// is the language gate. See deka#292.

import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync, chmodSync, cpSync, copyFileSync } from "node:fs";
import { dirname, isAbsolute, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import os from "node:os";

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(__dirname, "..", "..");
// Fixture discovery root. Defaults to this directory; --root overrides it so
// a hermetic corpus (and other repos, e.g. the testsuite website) can run
// through the same gate without touching the committed fixtures (deka#539).
let testsRoot = __dirname;
const scratchRoot = join(__dirname, ".run-tmp");

// Fixtures known to fail, loaded from tests/testsuite/expected-failures.txt.
// A ratchet, not a suppression list: a listed fixture that starts passing is a
// hard error, so the list can only shrink (deka#503).
function loadExpectedFailures() {
  const file = join(testsRoot, "expected-failures.txt");
  if (!existsSync(file)) return new Set();
  return new Set(
    readFileSync(file, "utf8")
      .split("\n")
      .map((line) => line.trim())
      .filter((line) => line && !line.startsWith("#"))
  );
}

const NL = String.fromCharCode(10);

const DEFAULT_DEKA_JSON = {
  name: "conformance-fixture",
  security: {
    allow: {
      read: ["./"],
      write: [".cache"],
    },
    prompt: false,
  },
};

const PACKAGE_DEKA_JSON = {
  name: "conformance-fixture",
  security: {
    allow: {
      read: ["./"],
      write: [".cache", "php_modules", "ds_modules"],
    },
    prompt: false,
  },
};

function fixtureImportsIo(test) {
  const blobs = [test.source, ...Object.values(test.files ?? {})];
  return blobs.some((s) => /\bfrom\s+["']io["']/.test(s));
}

function packagesFor(test) {
  const packages = [...(test.packages ?? [])];
  if (fixtureImportsIo(test) && !packages.some((p) => p === "io" || p === "@deka/io")) {
    packages.push("io");
  }
  return packages;
}

function findCliBinary() {
  if (process.env.DEKA_NATIVE) {
    const resolved = isAbsolute(process.env.DEKA_NATIVE)
      ? process.env.DEKA_NATIVE
      : resolve(process.cwd(), process.env.DEKA_NATIVE);
    if (!existsSync(resolved)) {
      throw new Error(`DEKA_NATIVE is set to ${process.env.DEKA_NATIVE} but that file does not exist`);
    }
    return resolved;
  }
  const candidate = join(repoRoot, "target", "release", "cli");
  try {
    const stat = statSync(candidate);
    if (stat.isFile() && (stat.mode & 0o111)) return candidate;
  } catch {}
  return null;
}

function parseStatusFromFilename(filename) {
  if (filename.endsWith(".pass.ds") || filename.endsWith(".pass.dsx")) return "pass";
  if (filename.endsWith(".fail.ds") || filename.endsWith(".fail.dsx")) return "fail";
  return null;
}

function baseNameFromFilename(filename) {
  return filename.replace(/\.(pass|fail)\.dsx?$/, "");
}

function slugFromParts(category, name) {
  return `${category}-${name}`.toLowerCase().replace(/[^a-z0-9]+/g, "-");
}

function collectDsFiles(dir, relativeTo) {
  const results = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    const rel = relative(relativeTo, full).replace(/\\/g, "/");
    if (entry.isDirectory()) {
      results.push(...collectDsFiles(full, relativeTo));
    } else if (entry.isFile() && (entry.name.endsWith(".ds") || entry.name.endsWith(".dsx"))) {
      results.push(rel);
    }
  }
  return results;
}

function parseHosts(raw) {
  if (!Array.isArray(raw)) return ["native", "browser"];
  const hosts = raw.filter((item) => item === "native" || item === "browser");
  return hosts.length > 0 ? hosts : ["native", "browser"];
}

function parsePackages(raw) {
  if (!Array.isArray(raw)) return undefined;
  const names = raw.filter((item) => typeof item === "string" && item.trim().length > 0);
  return names.length > 0 ? names : undefined;
}

function currentCompiler() {
  const value = process.env.DEKA_COMPILER;
  if (typeof value === "string" && value.trim().toLowerCase() === "v1") {
    return "v1";
  }
  return "v2";
}

function parseCompiler(raw) {
  if (raw === "v1" || raw === "v2") return raw;
  return undefined;
}

function readMaybe(dir, filename) {
  const filePath = join(dir, filename);
  if (!existsSync(filePath)) return undefined;
  return readFileSync(filePath, "utf-8");
}

function readMetadata(dir, name) {
  const jsonPath = join(dir, `${name}.json`);
  if (!existsSync(jsonPath)) return {};
  try {
    const raw = JSON.parse(readFileSync(jsonPath, "utf-8"));
    return {
      title: typeof raw.title === "string" ? raw.title : undefined,
      stage: ["parse", "typecheck", "run"].includes(raw.stage) ? raw.stage : undefined,
      hosts: parseHosts(raw.hosts),
      expectedStdoutNative:
        typeof raw.expectedStdoutNative === "string" ? raw.expectedStdoutNative : undefined,
      expectedDiagnosticContains:
        typeof raw.expectedDiagnosticContains === "string" ? raw.expectedDiagnosticContains : undefined,
      dekaJson:
        raw.dekaJson && typeof raw.dekaJson === "object" && !Array.isArray(raw.dekaJson)
          ? raw.dekaJson
          : undefined,
      packages: parsePackages(raw.packages),
      compiler: parseCompiler(raw.compiler),
      notes: typeof raw.notes === "string" ? raw.notes : undefined,
    };
  } catch {
    return {};
  }
}

function loadAllTests() {
  const categories = [];
  for (const categoryEntry of readdirSync(testsRoot, { withFileTypes: true })) {
    if (!categoryEntry.isDirectory()) continue;
    if (categoryEntry.name.startsWith(".")) continue;
    const categoryName = categoryEntry.name;
    const categoryDir = join(testsRoot, categoryName);
    const tests = [];
    for (const testEntry of readdirSync(categoryDir, { withFileTypes: true })) {
      if (!testEntry.isDirectory()) continue;
      const testName = testEntry.name;
      const testDir = join(categoryDir, testName);
      const dsFiles = collectDsFiles(testDir, testDir);
      const entryFile = dsFiles.find((f) => parseStatusFromFilename(f) && !f.includes("/"));
      if (!entryFile) continue;

      const status = parseStatusFromFilename(entryFile);
      const name = baseNameFromFilename(entryFile);
      const source = readMaybe(testDir, entryFile);
      if (source === undefined) continue;

      const metadata = readMetadata(testDir, name);
      const expectedStdout = readMaybe(testDir, `${name}.stdout`);
      const extraDsFiles = dsFiles.filter((f) => f !== entryFile);
      const files =
        extraDsFiles.length > 0
          ? Object.fromEntries(
              extraDsFiles
                .map((f) => [f, readMaybe(testDir, f)])
                .filter(([, content]) => content !== undefined)
            )
          : undefined;

      tests.push({
        slug: slugFromParts(categoryName, testName),
        category: categoryName,
        status,
        name: testName,
        dir: testDir,
        source,
        files,
        entryPath: entryFile,
        title: metadata.title ?? testName.replace(/_/g, " "),
        stage: metadata.stage ?? "run",
        hosts: metadata.hosts ?? ["native", "browser"],
        expectedStdout,
        expectedStdoutNative: metadata.expectedStdoutNative,
        expectedDiagnosticContains: metadata.expectedDiagnosticContains,
        dekaJson: metadata.dekaJson,
        packages: metadata.packages,
        compiler: metadata.compiler,
        notes: metadata.notes,
      });
    }
    if (tests.length > 0) {
      tests.sort((a, b) => a.name.localeCompare(b.name));
      categories.push({ name: categoryName, tests });
    }
  }
  categories.sort((a, b) => a.name.localeCompare(b.name));
  return categories;
}

function parseNativeDiagnostics(stderr) {
  const diagnostics = [];
  const lines = stderr.split("\n");
  let line;
  let column;

  for (const current of lines) {
    const headerMatch = current.match(/^┌─\s+\S+:(\d+):(\d+)\s*$/);
    if (headerMatch) {
      line = Number(headerMatch[1]);
      column = Number(headerMatch[2]);
      continue;
    }
    const messageMatch = current.match(/\^\s+(.+)$/);
    if (messageMatch) {
      const message = messageMatch[1].trim();
      if (message) {
        diagnostics.push({ severity: "error", message, line, column });
      }
      line = undefined;
      column = undefined;
    }
  }

  if (diagnostics.length === 0) {
    const firstLine = lines.find((l) => {
      const trimmed = l.trim();
      return (
        trimmed.length > 0 &&
        !trimmed.startsWith("[") &&
        !trimmed.startsWith("Validation") &&
        !trimmed.startsWith("❌")
      );
    });
    if (firstLine) {
      diagnostics.push({ severity: "error", message: firstLine.trim() });
    }
  }

  return diagnostics;
}

function writeProjectFiles(tmpDir, entryPath, source, files) {
  const isProject = files && Object.keys(files).length > 0;
  if (!isProject) {
    const ext = String(entryPath || "test.ds").endsWith(".dsx") ? ".dsx" : ".ds";
    writeFileSync(join(tmpDir, `test${ext}`), source);
    return { isProject: false, ext };
  }
  mkdirSync(dirname(join(tmpDir, entryPath)), { recursive: true });
  writeFileSync(join(tmpDir, entryPath), source);
  for (const [filePath, content] of Object.entries(files)) {
    const fullPath = join(tmpDir, filePath);
    mkdirSync(dirname(fullPath), { recursive: true });
    writeFileSync(fullPath, content);
  }
  return { isProject: true, ext: ".ds" };
}

// A cache hit restores ds_modules/ and deka.lock but not the manifest, so the
// fixture ended up with packages installed and never declared -- exactly the
// shape the project gate rejects (deka#403, deka#430). Derive the dependency
// block from what was actually restored, so it does not matter which runner
// filled the shared cache.
function declareRestoredModules(tmpDir) {
  const manifestPath = join(tmpDir, "deka.json");
  const manifest = existsSync(manifestPath)
    ? JSON.parse(readFileSync(manifestPath, "utf-8"))
    : {};
  const deps = { ...(manifest.dependencies ?? {}) };
  for (const modulesDir of ["ds_modules", "php_modules"]) {
    const scope = join(tmpDir, modulesDir, "@deka");
    if (!existsSync(scope)) continue;
    for (const name of readdirSync(scope)) {
      const pkg = "@deka/" + name;
      if (deps[pkg]) continue;
      const pkgManifest = join(scope, name, "deka.json");
      let version = "*";
      if (existsSync(pkgManifest)) {
        try {
          version = JSON.parse(readFileSync(pkgManifest, "utf-8")).version ?? "*";
        } catch {
          version = "*";
        }
      }
      deps[pkg] = version;
    }
  }
  manifest.dependencies = deps;
  writeFileSync(manifestPath, JSON.stringify(manifest, null, 2) + NL);
}

function restoreCachedModules(cacheDir, tmpDir) {
  for (const name of ["ds_modules", "php_modules"]) {
    const cached = join(cacheDir, name);
    if (existsSync(cached)) {
      cpSync(cached, join(tmpDir, name), { recursive: true });
    }
  }
}

function lockMatchesFixture(tmpDir, cacheDir) {
  const tmpLock = join(tmpDir, "deka.lock");
  const cachedLock = join(cacheDir, "deka.lock");
  if (!existsSync(tmpLock) || !existsSync(cachedLock)) return false;
  try {
    return readFileSync(tmpLock, "utf-8") === readFileSync(cachedLock, "utf-8");
  } catch {
    return false;
  }
}

/// Derive the package-cache key from the RESOLVED versions in the fixture's
/// lock, not just the requested names (deka#535). A name-only key served
/// pre-release package bytes to a post-release compiler: the failure blamed
/// the package source and read as a compiler bug. Keying on name@version
/// means a version change misses and reinstalls — no invalidation logic —
/// and entries written under the old name-only format are never looked up.
///
/// Returns null when the lock carries no resolved packages — an unlocked
/// fixture before its first resolution. Nothing resolved means there is no
/// version to key on, so the cache is bypassed entirely (install fresh,
/// do not serve, do not populate) rather than falling back to a name-only
/// key that would serve stale bytes forever.
export function cacheKeyFromLock(lockSource, packages) {
  let lock = null;
  try {
    lock = JSON.parse(lockSource);
  } catch {
    return null;
  }
  const entries = Object.entries(lock?.packages ?? {});
  if (entries.length === 0) return null;
  return entries
    .map(([name, value]) => {
      // Lock entries are ["name@version", source, {gitRef, ...}, ""].
      const version = Array.isArray(value)
        ? String(value[0] ?? "").slice(String(value[0] ?? "").lastIndexOf("@") + 1)
        : "";
      return version ? `${name}@${version}` : name;
    })
    .sort()
    .join("+");
}

function installPackages(cliPath, tmpDir, packages, locked, verbose) {
  // The fixture lock is already in tmpDir (runNative copies or writes it
  // before installing), so resolved versions are available for the key.
  let lockSource = "";
  try {
    lockSource = readFileSync(join(tmpDir, "deka.lock"), "utf-8");
  } catch {
    // No lock yet: the cache is bypassed below.
  }
  const requestedKey = cacheKeyFromLock(lockSource, packages);
  const cacheDir =
    requestedKey === null
      ? null
      : join(repoRoot, ".cache", "deka-packages", requestedKey);
  const cachedLock = cacheDir === null ? "" : join(cacheDir, "deka.lock");
  const hasCachedModules =
    cacheDir !== null &&
    (existsSync(join(cacheDir, "ds_modules")) || existsSync(join(cacheDir, "php_modules")));

  if (
    requestedKey !== null &&
    existsSync(cachedLock) &&
    hasCachedModules &&
    (!locked || lockMatchesFixture(tmpDir, cacheDir))
  ) {
    if (verbose) console.error(`[deka-packages cache hit] ${requestedKey}`);
    restoreCachedModules(cacheDir, tmpDir);
    copyFileSync(cachedLock, join(tmpDir, "deka.lock"));
    declareRestoredModules(tmpDir);
    return { ok: true, stderr: "" };
  }

  const cliArgs = ["add", ...packages, "--yes"];
  if (locked) cliArgs.push("--locked");
  const spawned = spawnSync(cliPath, cliArgs, {
    cwd: tmpDir,
    encoding: "utf-8",
    timeout: 120000,
    env: { ...process.env, DEKA_SECURITY_NO_PROMPT: "1" },
  });
  const stderr = spawned.stderr ?? "";
  if (spawned.status !== 0 || spawned.error) {
    return {
      ok: false,
      error:
        spawned.error?.message ??
        stderr
          .split("\n")
          .map((line) => line.trim())
          .find((line) => line.length > 0) ??
        `deka add ${packages.join(" ")} failed`,
      stderr,
    };
  }

  // Populate under the versions the install ACTUALLY resolved, read back from
  // the post-add lock — not the (possibly empty) pre-add lock. First-run
  // fixtures then land under their versioned key and hit on the next run.
  const lockPath = join(tmpDir, "deka.lock");
  let finalKey = requestedKey;
  if (existsSync(lockPath)) {
    let finalLock = "";
    try {
      finalLock = readFileSync(lockPath, "utf-8");
    } catch {
      finalLock = "";
    }
    finalKey = cacheKeyFromLock(finalLock, packages) ?? requestedKey;
  }
  if (finalKey !== null) {
    const finalDir = join(repoRoot, ".cache", "deka-packages", finalKey);
    mkdirSync(finalDir, { recursive: true });
    for (const name of ["ds_modules", "php_modules"]) {
      const dir = join(tmpDir, name);
      if (existsSync(dir)) {
        cpSync(dir, join(finalDir, name), { recursive: true });
      }
    }
    if (existsSync(lockPath)) {
      copyFileSync(lockPath, join(finalDir, "deka.lock"));
    }
    if (verbose) console.error(`[deka-packages cache populate] ${finalKey}`);
  }
  return { ok: true, stderr };
}


function runNative(cliPath, test, locked, verbose) {
  mkdirSync(scratchRoot, { recursive: true });
  const tmpDir = mkdtempSync(join(scratchRoot, "case-"));
  chmodSync(tmpDir, 0o700);
  const packages = packagesFor(test);

  try {
    const { isProject, ext } = writeProjectFiles(tmpDir, test.entryPath ?? "test.ds", test.source, test.files);
    const fixtureLockPath = join(test.dir, "deka.lock");
    const fixtureLockExists = existsSync(fixtureLockPath);
    if (fixtureLockExists) {
      copyFileSync(fixtureLockPath, join(tmpDir, "deka.lock"));
    } else if (locked) {
      return {
        ok: false,
        stdout: "",
        stderr: "",
        error: `locked run requires deka.lock in ${test.dir}`,
        transpileFailed: true,
        diagnostics: [{ severity: "error", message: `locked run requires deka.lock in ${test.dir}` }],
      };
    }
    const dekaJson = test.dekaJson ?? (packages.length > 0 ? PACKAGE_DEKA_JSON : DEFAULT_DEKA_JSON);
    writeFileSync(join(tmpDir, "deka.json"), JSON.stringify(dekaJson, null, 2) + "\n");

    // The runtime project gate requires a deka.lock whenever deka.json exists.
    // Fixtures without package dependencies carry an empty lock; fixtures with
    // packages use the resolved lock produced by deka add.
    const tmpLockPath = join(tmpDir, "deka.lock");
    if (!existsSync(tmpLockPath)) {
      writeFileSync(
        tmpLockPath,
        '{\n  "lockfileVersion": 1,\n  "packages": {}\n}\n'
      );
    }

    if (packages.length > 0) {
      const installed = installPackages(cliPath, tmpDir, packages, locked, verbose);
      if (!installed.ok) {
        return {
          ok: false,
          stdout: "",
          stderr: installed.stderr,
          error: installed.error,
          transpileFailed: true,
          diagnostics: installed.error ? [{ severity: "error", message: installed.error }] : [],
        };
      }
    }

    if (!fixtureLockExists) {
      const tmpLock = join(tmpDir, "deka.lock");
      if (existsSync(tmpLock)) {
        copyFileSync(tmpLock, fixtureLockPath);
      }
    }

    const entryRel = isProject ? `./${test.entryPath ?? "main.ds"}` : `./test${ext ?? ".ds"}`;
    const spawned = spawnSync(cliPath, ["run", entryRel], {
      cwd: tmpDir,
      encoding: "utf-8",
      timeout: 30000,
      env: { ...process.env, DEKA_SECURITY_NO_PROMPT: "1" },
    });

    const stdout = spawned.stdout ?? "";
    const rawStderr = spawned.stderr ?? "";
    const stderr = rawStderr
      .split("\n")
      .filter((line) => !line.startsWith("[security]"))
      .join("\n");
    const failed = spawned.status !== 0 || spawned.error !== undefined;
    const ranInIsolate = rawStderr.includes("Run failed:") || stdout.length > 0;
    const diagnostics = failed ? parseNativeDiagnostics(stderr || rawStderr) : [];
    const firstError =
      diagnostics[0]?.message ??
      stderr
        .split("\n")
        .map((line) => line.trim())
        .find((line) => line.length > 0) ??
      spawned.error?.message ??
      (failed ? "deka run failed" : undefined);

    return {
      ok: !failed,
      stdout,
      stderr,
      error: failed ? firstError : undefined,
      transpileFailed: failed && !ranInIsolate,
      diagnostics,
    };
  } finally {
    try {
      rmSync(tmpDir, { recursive: true, force: true });
    } catch {}
  }
}

function nativeStage(result) {
  if (result.ok) return "run";
  return result.transpileFailed ? "parse" : "run";
}

function stageMatches(expected, actual) {
  if (expected === actual) return true;
  // Native isolate does not distinguish typecheck from parse on compile failure.
  if (
    (expected === "parse" || expected === "typecheck") &&
    (actual === "parse" || actual === "typecheck")
  ) {
    return true;
  }
  return false;
}

function expectedStdoutForNative(test) {
  if (test.expectedStdoutNative !== undefined) return test.expectedStdoutNative;
  return test.expectedStdout;
}

function evaluate(test, result) {
  const stage = nativeStage(result);
  const reasons = [];
  const actualStatus = result.ok ? "pass" : "fail";
  if (actualStatus !== test.status) {
    reasons.push(`status: want ${test.status}, got ${actualStatus}${result.error ? ` (${result.error})` : ""}`);
  }
  if (!stageMatches(test.stage, stage)) {
    reasons.push(`stage: want ${test.stage}, got ${stage}`);
  }
  const expectedStdout = expectedStdoutForNative(test);
  if (expectedStdout !== undefined && result.stdout !== expectedStdout) {
    reasons.push(
      `stdout:\n  expected: ${JSON.stringify(expectedStdout)}\n  actual:   ${JSON.stringify(result.stdout)}`
    );
  }
  if (test.expectedDiagnosticContains) {
    const needle = test.expectedDiagnosticContains.toLowerCase();
    const hay = [
      ...result.diagnostics.map((d) => d.message),
      result.stderr,
      result.error ?? "",
    ]
      .join("\n")
      .toLowerCase();
    if (!hay.includes(needle)) {
      reasons.push(
        `diagnostic: expected to contain ${JSON.stringify(test.expectedDiagnosticContains)}, got ${JSON.stringify(result.error ?? result.stderr)}`
      );
    }
  }
  return { matched: reasons.length === 0, stage, reasons };
}

function parseArgs(argv) {
  const args = {
    list: false,
    json: false,
    filter: null,
    help: false,
    locked: false,
    root: "",
    verbose: false,
    jobs: Math.min(8, os.availableParallelism?.() || 4),
  };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--list" || arg === "-l") args.list = true;
    else if (arg === "--json") args.json = true;
    else if (arg === "--filter" || arg === "-f") args.filter = argv[++i] || "";
    else if (arg === "--jobs" || arg === "-j") args.jobs = Number(argv[++i] || args.jobs);
    else if (arg === "--locked") args.locked = true;
    else if (arg === "--root") args.root = argv[++i] || "";
    else if (arg === "--verbose" || arg === "-v") args.verbose = true;
    else if (arg === "--help" || arg === "-h") args.help = true;
  }
  if (args.root) testsRoot = resolve(process.cwd(), args.root);
  return args;
}

function printUsage() {
  console.log(`usage: bun tests/testsuite/run.mjs [options]

options:
  -l, --list                 List all fixtures and exit
  --json                     Output a JSON array of per-fixture results
  -f, --filter <substr>      Run only fixtures whose slug or title matches
  -j, --jobs <n>             Parallel native runs (default: min(8, CPUs))
  --locked                   Use existing deka.lock; fail if missing or stale
  --root <dir>               Fixture corpus root (default: tests/testsuite)
  -v, --verbose              Log package-cache hits and populates to stderr
  -h, --help                 Show this help

Native isolate only (\`deka run\`). Uses target/release/cli or DEKA_NATIVE.
Browser/WASM is the live playground on testsuite.deka.gg.

examples:
  cargo build --release -p cli
  bun tests/testsuite/run.mjs
  bun tests/testsuite/run.mjs --filter json
  DEKA_COMPILER=v2 bun tests/testsuite/run.mjs --json`);
}

async function mapPool(items, limit, fn) {
  const ret = new Array(items.length);
  let next = 0;
  async function worker() {
    while (true) {
      const idx = next++;
      if (idx >= items.length) return;
      ret[idx] = await fn(items[idx], idx);
    }
  }
  const n = Math.max(1, Math.min(limit, items.length));
  await Promise.all(Array.from({ length: n }, () => worker()));
  return ret;
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.help) {
    printUsage();
    process.exit(0);
  }

  const categories = loadAllTests();
  const all = categories.flatMap((c) => c.tests);

  if (args.list) {
    for (const test of all) {
      const hosts = test.hosts.join(",");
      console.log(`  ${test.slug}  ${test.status}@${test.stage}  [${hosts}]  ${test.title}`);
    }
    console.log(`\nTotal: ${all.length}`);
    process.exit(0);
  }

  const filtered = args.filter
    ? all.filter((t) => {
        const needle = args.filter.toLowerCase();
        return (
          t.slug.toLowerCase().includes(needle) ||
          t.title.toLowerCase().includes(needle) ||
          t.name.toLowerCase().includes(needle) ||
          t.category.toLowerCase().includes(needle)
        );
      })
    : all;

  if (filtered.length === 0) {
    console.error(`error: no fixtures match filter "${args.filter}"`);
    process.exit(1);
  }

  const cliBinary = findCliBinary();
  if (!cliBinary) {
    console.error("error: could not find deka CLI (build with: cargo build --release -p cli)");
    process.exit(1);
  }

  if (!args.json) {
    console.log(`native CLI: ${cliBinary}`);
    console.log(`fixtures: ${filtered.length}  jobs: ${args.jobs}`);
  }

  const activeCompiler = currentCompiler();

  // Browser-only fixtures are not this gate's population -- they are the dump's.
  // They are not "skipped": a fixture that never runs natively was never a
  // native test. Group membership is decided by `hosts` (see TESTING.md).
  const browserOnly = filtered.filter((t) => !t.hosts.includes("native"));
  const pool = filtered.filter((t) => t.hosts.includes("native"));
  const isNativeOnly = (t) => !t.hosts.includes("browser");

  const results = await mapPool(pool, args.jobs, async (test) => {
    if (test.compiler && test.compiler !== activeCompiler) {
      return { test, skipped: true, reason: `compiler mismatch: fixture requires ${test.compiler}, running ${activeCompiler}` };
    }
    const native = runNative(cliBinary, test, args.locked, args.verbose);
    const evaled = evaluate(test, native);
    return { test, skipped: false, native, ...evaled };
  });

  const expectedFailures = loadExpectedFailures();
  // Two groups run here: native-only and shared. Each is self-contained --
  // `pass + fail == group total` -- and there is no skip bucket anywhere.
  const groups = {
    "native-only": { passed: 0, failed: 0, known: 0, total: 0 },
    shared: { passed: 0, failed: 0, known: 0, total: 0 },
  };
  let failed = 0;
  const unexpectedlyPassing = [];

  // Gate computation runs in BOTH output modes. --json used to print and
  // exit(0) here, before the ratchet and reconciliation checks — a silent
  // bypass that reported success while fixtures were mismatched (deka#539).
  // JSON mode now emits the same per-fixture array after the gates and exits
  // with the same status as human-readable mode.
  if (!args.json) console.log("");
  for (const result of results) {
    const g = groups[isNativeOnly(result.test) ? "native-only" : "shared"];
    g.total++;
    if (result.matched) {
      // A listed fixture that passes must be removed from the list; leaving it
      // would let a real regression hide behind a stale entry.
      if (expectedFailures.has(result.test.slug)) {
        unexpectedlyPassing.push(result.test.slug);
      }
      g.passed++;
      continue;
    }
    if (expectedFailures.has(result.test.slug)) {
      g.known++;
      continue;
    }
    g.failed++;
    failed++;
    if (!args.json) {
      console.log(`✗ ${result.test.slug}`);
      for (const reason of result.reasons) {
        console.log(`    ${reason}`);
      }
    }
  }

  if (unexpectedlyPassing.length > 0 && !args.json) {
    console.log("");
    console.log("These fixtures are listed in expected-failures.txt but PASSED.");
    console.log("Delete their lines -- a stale entry can hide a real regression:");
    for (const slug of unexpectedlyPassing) console.log(`    ${slug}`);
  }

  // Every fixture must land in exactly one bucket. If this identity ever fails
  // a fixture has fallen between the cases and is owned by nothing (deka#503).
  // Each group must account for every fixture in it. Nothing is skipped, so a
  // shortfall means a fixture fell out of the run entirely (deka#503).
  for (const [name, g] of Object.entries(groups)) {
    const accounted = g.passed + g.failed + g.known;
    if (accounted !== g.total) {
      console.error(
        `\nreconciliation failed in ${name}: ${accounted} accounted for, ${g.total} fixtures.`
      );
      process.exit(1);
    }
  }
  if (groups["native-only"].total + groups.shared.total !== pool.length) {
    console.error("\nreconciliation failed: group totals do not sum to the population.");
    process.exit(1);
  }

  if (args.json) {
    const output = results.map((r) => ({
      slug: r.test.slug,
      category: r.test.category,
      name: r.test.name,
      expectedStatus: r.test.status,
      expectedStage: r.test.stage,
      group: r.test.hosts.includes("browser") ? "shared" : "native-only",
      matched: r.matched ?? false,
      actualStatus: r.native ? (r.native.ok ? "pass" : "fail") : undefined,
      actualStage: r.stage,
      stdout: r.native?.stdout,
      stderr: r.native?.stderr,
      error: r.native?.error,
      reasons: r.reasons,
    }));
    console.log(JSON.stringify(output, null, 2));
  } else {
    const line = (name, g) =>
      ` ${name.padEnd(13)} ${String(g.passed).padStart(4)} pass · ${String(
        g.failed
      ).padStart(3)} fail${g.known ? ` · ${g.known} known` : ""}   (${g.total})`;

    console.log("\n============================================================");
    console.log(line("native-only", groups["native-only"]));
    console.log(line("shared", groups.shared));
    console.log(
      ` ${"browser-only".padEnd(13)} ${String(browserOnly.length).padStart(
        4
      )} not this gate's population — run tests/dump`
    );
    console.log("============================================================\n");
  }

  process.exit(failed === 0 && unexpectedlyPassing.length === 0 ? 0 : 1);
}

// Run only when executed directly, not when imported for its exported
// helpers (e.g. cacheKeyFromLock in tests).
if (import.meta.main) {
  main().catch((error) => {
    console.error(error);
    process.exit(1);
  });
}
