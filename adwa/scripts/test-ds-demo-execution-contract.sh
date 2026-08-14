#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="$(mktemp -d)"
trap 'rm -rf "$OUT_DIR"' EXIT

ADWA_DEMO_OUT_DIR="$OUT_DIR/package" "$ROOT_DIR/scripts/build-demo.sh"
cmp "$ROOT_DIR/website/core/deka_demo_consumer.js" "$OUT_DIR/package/consumer/deka_demo_consumer.js"
cmp "$ROOT_DIR/ds-demo/deka.json" "$OUT_DIR/package/demo/deka.json"
cmp "$ROOT_DIR/ds-demo/src/hello.ds" "$OUT_DIR/package/demo/src/hello.ds"
cmp "$ROOT_DIR/ds-demo/src/broken.ds" "$OUT_DIR/package/demo/src/broken.ds"

node - "$OUT_DIR/package" <<'EOF2'
const fs = require("node:fs");
const path = require("node:path");

const packageDir = process.argv[2];
const manifest = JSON.parse(fs.readFileSync(path.join(packageDir, "deka-demo-package.json"), "utf8"));
const expected = {
  schema_version: 1,
  compiler: "compiler/deka_compiler.wasm",
  compiler_metadata: "compiler/deka_compiler.wasm.metadata.json",
  compiler_checksum: "compiler/deka_compiler.wasm.sha256",
  demo_config: "demo/deka.json",
  consumer: "consumer/deka_demo_consumer.js",
};
if (JSON.stringify(manifest) !== JSON.stringify(expected)) throw new Error("unexpected Deka demo package manifest");
for (const artifact of Object.values(expected).filter((value) => typeof value === "string")) {
  if (!fs.statSync(path.join(packageDir, artifact)).isFile()) throw new Error(`missing packaged artifact: ${artifact}`);
}
const files = fs.readdirSync(path.join(packageDir, "demo", "src")).sort();
if (JSON.stringify(files) !== JSON.stringify(["broken.ds", "hello.ds"])) throw new Error("demo package must contain only DS inputs");
EOF2

node "$ROOT_DIR/tests/deka_demo_execution_contract.mjs" "$OUT_DIR/package/compiler/deka_compiler.wasm"
