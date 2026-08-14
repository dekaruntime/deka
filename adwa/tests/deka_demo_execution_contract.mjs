import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { createDekaDemoConsumer, renderDekaDemoOutcome } from "../website/core/deka_demo_consumer.js";

const wasmPath = process.argv[2];
if (!wasmPath) throw new Error("usage: node deka_demo_execution_contract.mjs <deka_compiler.wasm>");
const wasmBytes = await readFile(wasmPath);
let loads = 0;
const consumer = createDekaDemoConsumer({
  wasmUrl: "https://demo.invalid/deka_compiler.wasm",
  fetchImpl: async () => {
    loads += 1;
    return new Response(wasmBytes, { status: 200 });
  },
});

const hello = await readFile(new URL("../ds-demo/src/hello.ds", import.meta.url), "utf8");
const success = await consumer.run({ filename: "hello.ds", source: hello });
assert.equal(success.kind, "success");
assert.deepEqual(success.results, ["Hello from DekaScript"]);
assert.equal(success.metadata.language, "deka");

const broken = await readFile(new URL("../ds-demo/src/broken.ds", import.meta.url), "utf8");
const diagnostic = await consumer.run({ filename: "broken.ds", source: broken });
assert.equal(diagnostic.kind, "diagnostic");
assert.equal(diagnostic.diagnostics[0].filename, "broken.ds");
const diagnosticView = { dataset: {}, textContent: "" };
renderDekaDemoOutcome(diagnosticView, diagnostic);
assert.equal(diagnosticView.dataset.dekaOutcome, "diagnostic");
assert.match(diagnosticView.textContent, /broken\.ds:/);

const runtimeFailure = await consumer.run({ filename: "runtime-failure.ds", source: "runtime.missing();" });
assert.equal(runtimeFailure.kind, "runtime_error");
const runtimeView = { dataset: {}, textContent: "" };
renderDekaDemoOutcome(runtimeView, runtimeFailure);
assert.equal(runtimeView.dataset.dekaOutcome, "runtime_error");
assert.match(runtimeView.textContent, /^Runtime error:/);

await assert.rejects(() => consumer.run({ filename: "legacy.phpx", source: hello }), /named .ds files/);
await assert.rejects(() => consumer.run({ filename: "project-bundle", source: hello }), /named .ds files/);
assert.equal(loads, 1, "compiler WASM must initialize only once");
console.log("Deka demo execution contract passed");
