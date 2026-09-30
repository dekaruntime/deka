// Usage: node scripts/test-native-layout.mjs <wasm-bindgen web output directory> <native-scenes.json>
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { resolve, join } from 'node:path';
import { pathToFileURL } from 'node:url';
const directory = resolve(process.argv[2]);
const native = JSON.parse(readFileSync(process.argv[3], 'utf8'));
const cases = JSON.parse(readFileSync(new URL('../examples/native/layout-cases.json', import.meta.url), 'utf8'));
const { default: init, NativePreview } = await import(pathToFileURL(join(directory, 'deka_native_web.js')));
await init({ module_or_path: readFileSync(join(directory, 'deka_native_web_bg.wasm')) });
function compare(a, b, path) {
  if (typeof a === 'number' && typeof b === 'number') {
    assert(Math.abs(a - b) <= 0.0001, `${path}: ${a} != ${b}`);
  } else if (a && b && typeof a === 'object' && typeof b === 'object') {
    assert.deepEqual(Object.keys(a).sort(), Object.keys(b).sort(), path);
    for (const key of Object.keys(a)) compare(a[key], b[key], `${path}.${key}`);
  } else assert.equal(a, b, path);
}
let count = 0;
for (const fixture of cases) {
  for (const scale of [1, 2]) {
    const preview = new NativePreview();
    try {
      preview.compile(fixture.source, false);
      let scene;
      for (const [index, step] of (fixture.steps || [{}]).entries()) {
        if (step.click) {
          const r = scene.targets[0].rect;
          assert(preview.pointer(r.x + 4, r.y + 4));
        }
        const expected = native.find(n => n.name === fixture.name && n.scale === scale && n.step === index);
        assert(expected, `Missing native scene: ${fixture.name}/${scale}/${index}`);
        scene = JSON.parse(step.time === undefined ? preview.frame(fixture.width, fixture.height, scale)
          : preview.frame_at(fixture.width, fixture.height, scale, step.time, !!step.reduced));
        compare(scene, expected.scene, fixture.name);
        count++;
      }
    } finally { preview.free(); }
  }
}
assert.equal(native.length, count);
console.log(`PASS: ${count} native/WASM scenes agree (layout, clips, paint, glyph pixels and targets).`);
