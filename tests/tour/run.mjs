#!/usr/bin/env bun
// Compile every tests/tour lesson with the local CLI. Match by id in
// manifest.json, never by display name. See deka#292.

import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, isAbsolute, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(__dirname, "..", "..");
const manifestPath = join(__dirname, "manifest.json");
const scratchDir = join(__dirname, ".run-tmp");

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

function parseArgs(argv) {
  const args = { list: false, filter: null, help: false };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--list" || arg === "-l") args.list = true;
    else if (arg === "--filter" || arg === "-f") args.filter = argv[++i] || "";
    else if (arg === "--help" || arg === "-h") args.help = true;
  }
  return args;
}

function printUsage() {
  console.log(`usage: bun tests/tour/run.mjs [options]

options:
  -l, --list            List all lessons and exit
  -f, --filter <substr> Run only lessons whose id or title matches
  -h, --help            Show this help`);
}

function compileLesson(cliBinary, sourcePath, outPath) {
  const result = spawnSync(cliBinary, ["transpile", sourcePath, "--out", outPath], {
    encoding: "utf-8",
    timeout: 30_000,
    env: { ...process.env, DEKA_SECURITY_NO_PROMPT: "1" },
  });
  const combined = `${result.stderr ?? ""}\n${result.stdout ?? ""}`;
  return {
    ok: result.status === 0,
    output: combined,
  };
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.help) {
    printUsage();
    process.exit(0);
  }

  const manifest = JSON.parse(readFileSync(manifestPath, "utf-8"));
  const dsFiles = readdirSync(__dirname).filter((name) => name.endsWith(".ds"));
  const manifestIds = new Set(manifest.map((lesson) => lesson.id));
  const fileIds = new Set(dsFiles.map((name) => name.replace(/\.ds$/, "")));

  const missingFiles = [...manifestIds].filter((id) => !fileIds.has(id));
  const orphanFiles = [...fileIds].filter((id) => !manifestIds.has(id));
  if (missingFiles.length > 0 || orphanFiles.length > 0) {
    if (missingFiles.length > 0) {
      console.error(`error: manifest ids with no .ds file: ${missingFiles.join(", ")}`);
    }
    if (orphanFiles.length > 0) {
      console.error(`error: .ds files not in manifest.json: ${orphanFiles.join(", ")}`);
    }
    process.exit(1);
  }

  if (args.list) {
    for (const lesson of manifest) {
      console.log(`  ${lesson.id}  compile=${lesson.expectCompile}  ${lesson.title}`);
    }
    console.log(`\nTotal: ${manifest.length}`);
    process.exit(0);
  }

  const filtered = args.filter
    ? manifest.filter((lesson) => {
        const needle = args.filter.toLowerCase();
        return lesson.id.toLowerCase().includes(needle) || lesson.title.toLowerCase().includes(needle);
      })
    : manifest;

  if (filtered.length === 0) {
    console.error(`error: no lessons match filter "${args.filter}"`);
    process.exit(1);
  }

  const cliBinary = findCliBinary();
  if (!cliBinary) {
    console.error("error: could not find deka CLI (build with: cargo build --release -p cli)");
    process.exit(1);
  }

  mkdirSync(scratchDir, { recursive: true });
  let passed = 0;
  let failed = 0;

  console.log(`native CLI: ${cliBinary}`);
  console.log(`lessons: ${filtered.length}\n`);

  try {
    for (const lesson of filtered) {
      const sourcePath = join(__dirname, `${lesson.id}.ds`);
      const outPath = join(scratchDir, `${lesson.id}.js`);
      const compiled = compileLesson(cliBinary, sourcePath, outPath);
      try {
        rmSync(outPath, { force: true });
      } catch {}

      const reasons = [];
      if (compiled.ok !== lesson.expectCompile) {
        reasons.push(
          lesson.expectCompile
            ? `expected compile success, got:\n${compiled.output}`
            : "expected compile failure, but transpile succeeded"
        );
      } else if (!lesson.expectCompile && lesson.expectError) {
        if (!compiled.output.includes(lesson.expectError)) {
          reasons.push(
            `expected diagnostic containing ${JSON.stringify(lesson.expectError)}, got:\n${compiled.output}`
          );
        }
      }

      if (reasons.length === 0) {
        passed++;
        console.log(`✓ ${lesson.id}`);
      } else {
        failed++;
        console.log(`✗ ${lesson.id}`);
        for (const reason of reasons) {
          console.log(`    ${reason}`);
        }
      }
    }
  } finally {
    try {
      rmSync(scratchDir, { recursive: true, force: true });
    } catch {}
  }

  console.log("\n============================================================");
  console.log(` Passed: ${passed} | Failed: ${failed} | Total: ${filtered.length}`);
  console.log("============================================================\n");
  process.exit(failed === 0 ? 0 : 1);
}

main();
