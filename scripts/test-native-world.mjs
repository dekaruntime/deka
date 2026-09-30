// Usage: node scripts/test-native-world.mjs <wasm JS> <wasm binary> <native-world.json>
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { pathToFileURL } from 'node:url'
const { default: init, PortfolioWorld } = await import(pathToFileURL(resolve(process.argv[2])))
await init({ module_or_path: readFileSync(process.argv[3]) })
const golden = JSON.parse(readFileSync(process.argv[4], 'utf8'))
const world = new PortfolioWorld()
world.start()
function compare(a,b,path='root') {
  if (typeof a === 'number') { assert(Math.abs(a-b)<0.002, `${path}: ${a} vs ${b}`); return }
  if (a === null || typeof a !== 'object') { assert.equal(a,b,path); return }
  assert.deepEqual(Object.keys(a).sort(),Object.keys(b).sort(),path)
  for (const key of Object.keys(a)) compare(a[key],b[key],`${path}.${key}`)
}
try {
  for (let frame=0;frame<=240;frame++) {
    const key = ({0:['ArrowUp',true],90:['ArrowUp',false],91:['Enter',true],92:['Enter',false],160:['Enter',true],161:['Enter',false]})[frame]
    if (key) world.key(...key)
    const scene=JSON.parse(world.frame(960,640,frame*1000/120,false)), sounds=Array.from(world.sounds())
    const expected=golden.find(g=>g.frame===frame)
    if (expected) {
      // fontdue's native SIMD and WASM scalar coverage may differ by one alpha level.
      // Original sprite RGBA, RGB channels, geometry and simulation retain strict checks.
      for (let i=0;i<scene.images.length;i++) {
        const image=scene.images[i], reference=expected.scene.images[i]
        if (!image.id.startsWith('world-')) {
          for (let j=3;j<image.rgba.length;j+=4) {
            assert(Math.abs(image.rgba[j]-reference.rgba[j])<=1, `glyph alpha ${image.id}:${j}`)
            image.rgba[j]=reference.rgba[j]
          }
        }
      }
      compare(scene,expected.scene);compare(JSON.parse(world.snapshot()),expected.state);assert.deepEqual(sounds,expected.sounds)
    }
  }
  for(let id=0;id<3;id++){const pcm=PortfolioWorld.audio_samples(id);assert(pcm.length>1000);assert(pcm.some(s=>Math.abs(s)>0.01));assert(pcm.every(s=>Number.isFinite(s)&&Math.abs(s)<1))}
  console.log(`PASS: ${golden.length} native/WASM world scenes, movement, collision, room transitions, sound events and PCM.`)
} finally { world.free() }
