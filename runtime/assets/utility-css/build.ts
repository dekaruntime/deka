#!/usr/bin/env bun
/**
 * Bundle the shared utility-CSS generator for embedding in the Rust runtime.
 *
 * The runtime uses `include_str!` to load `bundle.js` and executes it in a
 * deno_core isolate. The bundle exposes `globalThis.__dekaGenerateUtilityCss`.
 */

import { build } from 'bun'
import { writeFileSync, unlinkSync } from 'fs'
import { join, dirname } from 'path'

const root = dirname(import.meta.path)

const result = await build({
  entrypoints: [join(root, 'index.ts')],
  outdir: root,
  naming: '[name].js',
  format: 'iife',
  target: 'browser',
  minify: false,
})

if (!result.success) {
  console.error(result.logs)
  process.exit(1)
}

// Bun writes `index.js` for IIFE output. Rename it to `bundle.js` and inject a
// global alias so the Rust side can call a stable name regardless of exports.
const bundled = result.outputs[0]
const source = await Bun.file(bundled.path).text()

const withGlobalAlias =
  source +
  `\n;globalThis.__dekaGenerateUtilityCss = (html, registryJson, options) => {` +
  ` return globalThis.dekaUtilityCss.injectUtilityCss(html, registryJson, options);` +
  `};\n`

writeFileSync(join(root, 'bundle.js'), withGlobalAlias)
// Remove the intermediate Bun output so only `bundle.js` is committed.
if (bundled.path.endsWith('index.js')) {
  unlinkSync(bundled.path)
}
console.log('Wrote bundle.js')
