#!/usr/bin/env bun
import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { readdir, readFile, stat } from "node:fs/promises";
import path from "node:path";

const repoRoot = path.resolve(process.cwd());
const defaultSuite = "tests/phpx/conformance/corpus";

const COLOR = {
  reset: "\x1b[0m",
  green: "\x1b[32m",
  red: "\x1b[31m",
  yellow: "\x1b[33m",
};

function parseList(value) {
  return value
    .split(",")
    .map((part) => part.trim())
    .filter(Boolean);
}

function parseArgs(argv) {
  const args = {
    suite: defaultSuite,
    backends: parseList(process.env.PHPX_CONFORMANCE_BACKENDS || "v8"),
  };

  for (const arg of argv) {
    if (arg.startsWith("--backends=")) {
      args.backends = parseList(arg.slice("--backends=".length));
    } else if (arg.startsWith("--backend=")) {
      args.backends = [arg.slice("--backend=".length).trim()].filter(Boolean);
    } else if (!arg.startsWith("--") && args.suite === defaultSuite) {
      args.suite = arg;
    } else {
      throw new Error(`Unexpected argument: ${arg}`);
    }
  }

  if (args.backends.length === 0) {
    throw new Error("At least one backend is required");
  }

  return args;
}

function splitArgs(value) {
  if (!value) {
    return [];
  }
  return value.split(" ").map((arg) => arg.trim()).filter(Boolean);
}

function resolveBackend(name) {
  if (name === "v8") {
    const binary = process.env.PHPX_V8_BIN || process.env.PHPX_BIN || "target/release/cli";
    const args = process.env.PHPX_V8_ARGS
      ? splitArgs(process.env.PHPX_V8_ARGS)
      : process.env.PHPX_BIN_ARGS
        ? splitArgs(process.env.PHPX_BIN_ARGS)
        : ["run"];
    return {
      name,
      binary: path.resolve(repoRoot, binary),
      args,
    };
  }

  if (name === "native") {
    if (!process.env.PHPX_NATIVE_BIN) {
      throw new Error(
        "native backend selected but PHPX_NATIVE_BIN is not set. Set it to the real native PHPX runner when that backend exists."
      );
    }
    return {
      name,
      binary: path.resolve(repoRoot, process.env.PHPX_NATIVE_BIN),
      args: splitArgs(process.env.PHPX_NATIVE_ARGS || ""),
    };
  }

  const prefix = `PHPX_${name.toUpperCase().replace(/[^A-Z0-9]/g, "_")}`;
  const binary = process.env[`${prefix}_BIN`];
  if (!binary) {
    throw new Error(`Unknown backend '${name}'. Set ${prefix}_BIN and optional ${prefix}_ARGS to register it.`);
  }

  return {
    name,
    binary: path.resolve(repoRoot, binary),
    args: splitArgs(process.env[`${prefix}_ARGS`] || ""),
  };
}

async function exists(filePath) {
  try {
    await stat(filePath);
    return true;
  } catch {
    return false;
  }
}

async function collectPhpxFiles(entryPath) {
  const info = await stat(entryPath);
  if (info.isFile()) {
    if (!entryPath.endsWith(".phpx")) {
      throw new Error(`Suite file must be a .phpx file: ${entryPath}`);
    }
    return [entryPath];
  }

  const entries = await readdir(entryPath, { withFileTypes: true });
  let files = [];
  for (const entry of entries) {
    const child = path.join(entryPath, entry.name);
    if (entry.isDirectory()) {
      if (!entry.name.startsWith("_")) {
        files = files.concat(await collectPhpxFiles(child));
      }
    } else if (entry.isFile() && entry.name.endsWith(".phpx")) {
      files.push(child);
    }
  }
  return files.sort();
}

function sanitizeStream(text) {
  const normalizedRepo = repoRoot.replace(/\\/g, "/");
  return text
    .replace(/\r\n/g, "\n")
    .split("\n")
    .filter((line) => !line.startsWith("[PthreadsExtension]"))
    .filter((line) => !line.startsWith("    at ") && !line.startsWith("\tat "))
    .join("\n")
    .replaceAll(normalizedRepo, "<repo>")
    .replace(/\n{3,}/g, "\n\n")
    .trimEnd()
    .concat(text.endsWith("\n") ? "\n" : "");
}

function indent(text) {
  if (text === "") {
    return "    (empty)";
  }
  return text
    .split(/\r?\n/)
    .map((line) => `    ${line}`)
    .join("\n");
}

async function runBackend(backend, scriptPath) {
  if (!existsSync(backend.binary)) {
    throw new Error(`${backend.name} backend binary not found: ${backend.binary}`);
  }

  return new Promise((resolve, reject) => {
    const proc = spawn(backend.binary, [...backend.args, scriptPath], {
      cwd: repoRoot,
      env: { ...process.env },
      stdio: ["ignore", "pipe", "pipe"],
    });

    let stdout = "";
    let stderr = "";
    proc.stdout.on("data", (chunk) => {
      stdout += chunk.toString();
    });
    proc.stderr.on("data", (chunk) => {
      stderr += chunk.toString();
    });
    proc.on("error", reject);
    proc.on("close", (code) => {
      resolve({
        code: code ?? 0,
        stdout: sanitizeStream(stdout),
        stderr: sanitizeStream(stderr),
      });
    });
  });
}

async function loadExpected(scriptPath) {
  const base = scriptPath.replace(/\.phpx$/, "");
  const outPath = `${base}.out`;
  if (!(await exists(outPath))) {
    return null;
  }
  return sanitizeStream(await readFile(outPath, "utf8"));
}

function sameResult(a, b) {
  return a.code === b.code && a.stdout === b.stdout && a.stderr === b.stderr;
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const suitePath = path.resolve(repoRoot, args.suite);
  const backends = args.backends.map(resolveBackend);
  const files = await collectPhpxFiles(suitePath);

  console.log(`PHPX conformance corpus: ${path.relative(repoRoot, suitePath)}`);
  console.log(`Backends: ${backends.map((backend) => backend.name).join(", ")}`);

  let failures = 0;
  for (const scriptPath of files) {
    const rel = path.relative(repoRoot, scriptPath);
    const results = [];
    for (const backend of backends) {
      results.push([backend.name, await runBackend(backend, scriptPath)]);
    }

    const [baselineName, baseline] = results[0];
    const expectedOut = await loadExpected(scriptPath);
    const expectedOk = expectedOut === null || baseline.stdout === expectedOut;
    const differentialOk = results.every(([, result]) => sameResult(result, baseline));

    if (expectedOk && differentialOk) {
      console.log(`${COLOR.green}ok${COLOR.reset} ${rel}`);
      continue;
    }

    failures += 1;
    console.log(`${COLOR.red}FAILED${COLOR.reset} ${rel}`);
    if (!expectedOk) {
      console.log(`  ${baselineName} stdout did not match ${path.basename(scriptPath, ".phpx")}.out`);
      console.log("  expected:");
      console.log(indent(expectedOut ?? ""));
      console.log("  actual:");
      console.log(indent(baseline.stdout));
    }
    if (!differentialOk) {
      for (const [name, result] of results.slice(1)) {
        if (sameResult(result, baseline)) {
          continue;
        }
        console.log(`  backend mismatch: ${baselineName} vs ${name}`);
        console.log(`  ${baselineName}: code=${baseline.code}`);
        console.log(indent(baseline.stdout));
        console.log(`  ${name}: code=${result.code}`);
        console.log(indent(result.stdout));
        if (baseline.stderr !== result.stderr) {
          console.log(`  ${baselineName} stderr:`);
          console.log(indent(baseline.stderr));
          console.log(`  ${name} stderr:`);
          console.log(indent(result.stderr));
        }
      }
    }
  }

  const passed = files.length - failures;
  const color = failures === 0 ? COLOR.green : COLOR.red;
  console.log(`\n${color}Summary: ${passed}/${files.length} conformance fixtures passed.${COLOR.reset}`);
  if (backends.length === 1) {
    console.log(`${COLOR.yellow}Note: only one backend selected; differential comparison activates when a second real backend is configured.${COLOR.reset}`);
  }
  if (failures > 0) {
    process.exit(1);
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
