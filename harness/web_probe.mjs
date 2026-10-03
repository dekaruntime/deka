// Render through the browser package (wasm) in node: what text does the browser get?
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { pathToFileURL } from 'node:url'
const root = resolve(process.argv[2])
const { default: init, NativePreview } = await import(pathToFileURL(resolve(root, 'deka_native_web.js')))
await init({ module_or_path: readFileSync(resolve(root, 'deka_native_web_bg.wasm')) })
const rt = new NativePreview()
rt.compile('export fn App() { return (<view className="p-4 gap-2"><p>Hamburgefonstiv</p><p>日本語 🎉</p></view>); }', true)
for (const scale of [1, 2]) {
  const t = performance.now()
  const s = JSON.parse(rt.frame(400, 200, scale))
  const ms = performance.now() - t
  const nodes = s.nodes.filter(n => n.text).map(n => `${n.text}: ${n.layout_rect.width}x${n.layout_rect.height}`)
  const glyphs = s.paint.filter(p => p.image).length
  const ink = s.images.map(i => { let a = 0; for (let k = 3; k < i.rgba.length; k += 4) a += i.rgba[k]; return a })
  console.log(`@${scale}x ${ms.toFixed(1)} ms; ${nodes.join('; ')}; ${glyphs} glyph paints, ${s.images.length} images, all inked: ${ink.every(a => a > 0)}`)
}
rt.free()
