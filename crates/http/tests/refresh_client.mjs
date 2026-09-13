// Execute the shipped client with the real vendored refresh runtime. Only the
// browser globals are supplied by this Node harness; no compiler/runtime stub.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import Refresh from '../vendor/react/refresh-runtime.js';

const source = readFileSync(new URL('../src/hmr_client/refresh.js', import.meta.url), 'utf8');
// Put cache-busting query text in the fragment of inline fixture modules.
const moduleUrl = code => `data:text/javascript;base64,${Buffer.from(code).toString('base64')}#`;
const { applyJsUpdate } = await import(moduleUrl(`${source}\nexport { applyJsUpdate };`));
globalThis.window = { __deka_refresh_runtime: Refresh };
let reloads = 0;
globalThis.location = { reload() { reloads++; } };
Refresh.injectIntoGlobalHook(globalThis);

// The real WS payload supplied by the CLI integration test must reload before
// importing an unmounted document family (its server URL is not a Node URL).
if (process.argv[2]) {
  const payload = JSON.parse(process.argv[2]);
  applyJsUpdate(payload);
  assert.equal(reloads, 1, 'unregistered WS families must synchronously reload');
} else {
  function LabelV1() { return 'before'; }
  Refresh.register(LabelV1, 'Label.js Label');
  const runtimeUrl = new URL('../vendor/react/refresh-runtime.js', import.meta.url).href;
  const update = moduleUrl(`import Refresh from ${JSON.stringify(runtimeUrl)};
    globalThis.refreshImports = (globalThis.refreshImports || 0) + 1;
    export function Label() { return 'after'; }
    Refresh.register(Label, 'Label.js Label');
    export const __dekaRefreshBoundary = true;`);
  const known = { id: 'Label.js', families: ['Label.js Label'], url: update };
  const missing = { id: 'Page.dsx', families: ['Page.dsx Page'], url: update };
  // Check all modules before importing any, including a missing family after
  // a valid one and a mixed known/unknown family list inside one module.
  for (const modules of [[missing], [known, missing], [{ ...known, families: ['Label.js Label', 'Page.dsx Page'] }]]) {
    const before = reloads;
    applyJsUpdate({ modules });
    assert.equal(reloads, before + 1);
    await new Promise(setImmediate);
    assert.equal(globalThis.refreshImports, undefined);
  }
  const before = reloads;
  applyJsUpdate({ modules: [known] });
  for (let i = 0; i < 100 && Refresh.getFamilyByID('Label.js Label').current === LabelV1; i++) {
    await new Promise(resolve => setTimeout(resolve, 10));
  }
  assert.equal(reloads, before, 'registered client families must not reload');
  assert.equal(globalThis.refreshImports, 1);
  assert.equal(Refresh.getFamilyByID('Label.js Label').current(), 'after');
}
console.log('refresh client contract passed');
