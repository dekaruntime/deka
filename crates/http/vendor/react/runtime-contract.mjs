// Node-side contract: the vendored refresh runtime registers families and
// exposes performReactRefresh. Not a DOM state-preservation test — that lives
// in tests/dump/scripts/fast-refresh-e2e.mjs.
import assert from "node:assert/strict";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const Refresh = await import(pathToFileURL(join(here, "refresh-runtime.js")).href);
const React = await import(pathToFileURL(join(here, "react.js")).href);

const runtime = Refresh.default;
assert.equal(typeof runtime.injectIntoGlobalHook, "function");
assert.equal(typeof runtime.register, "function");
assert.equal(typeof runtime.performReactRefresh, "function");
assert.equal(typeof runtime.setSignature, "function");
assert.equal(typeof runtime.isLikelyComponentType, "function");
assert.equal(React.default.version, "19.1.1");

runtime.injectIntoGlobalHook(globalThis);
function LabelV1() {
  return "hello";
}
function LabelV2() {
  return "hello world";
}
runtime.register(LabelV1, "fixture Label");
runtime.register(LabelV2, "fixture Label");
const family = runtime.getFamilyByID("fixture Label");
assert.ok(family, "register must create a family id");
assert.equal(family.current, LabelV1);
assert.equal(runtime.getFamilyByType(LabelV1), family);
assert.equal(runtime.getFamilyByType(LabelV2), family);
assert.equal(runtime.isLikelyComponentType(LabelV1), true);
assert.equal(runtime.isLikelyComponentType(function formatCount() {}), false);
runtime.performReactRefresh();
console.log("refresh runtime contract passed");
