// Platform adapter reused from the VM-era tour: Rust owns scenes and hit testing.
import { WebGLRenderer } from './webgl.js'
export function mount(app, canvas) {
  if (!(canvas instanceof HTMLCanvasElement)) { app.free(); throw new TypeError('launch requires an HTML canvas') }
  let renderer
  try { renderer = new WebGLRenderer(canvas) } catch (error) { app.free(); throw error }
  let disposed = false, request = 0, scene, fixedClock = new URLSearchParams(location.search).has('inspect') ? 0 : undefined
  const motion = matchMedia('(prefers-reduced-motion: reduce)')
  const inputs = new Map()
  const parent = canvas.parentElement
  const fail = error => canvas.dispatchEvent(new CustomEvent('deka:error', { detail: String(error), bubbles: true }))
  const syncInputs = () => {
    const active = new Set()
    for (const control of JSON.parse(app.inputs())) {
      const node = scene.nodes.find(node => node.id === control.id)
      if (!node || node.rect.width <= 0 || node.rect.height <= 0) continue
      active.add(control.id)
      let input = inputs.get(control.id)
      if (!input) {
        input = document.createElement('input'); input.setAttribute('aria-label', 'Deka text input')
        input.style.cssText = 'position:absolute;box-sizing:border-box;font:inherit;color:inherit;background:transparent;border:1px solid currentColor;border-radius:4px;padding:4px;'
        input.addEventListener('input', () => { app.input(control.id, input.value); draw() })
        input.addEventListener('keydown', event => { if (app.key_to(control.id,event.key)) event.preventDefault(); draw() })
        inputs.set(control.id,input); parent.append(input)
      }
      if (input.value !== control.value && document.activeElement !== input) input.value = control.value
      const bounds = canvas.getBoundingClientRect(), origin = parent.getBoundingClientRect()
      Object.assign(input.style,{left:`${bounds.left-origin.left+node.rect.x}px`,top:`${bounds.top-origin.top+node.rect.y}px`,width:`${node.rect.width}px`,height:`${node.rect.height}px`})
    }
    for (const [id,input] of inputs) if (!active.has(id)) { input.remove(); inputs.delete(id) }
  }
  const draw = () => {
    cancelAnimationFrame(request); request = 0
    if (disposed || rendererLost) return
    try {
      const bounds = canvas.getBoundingClientRect()
      const scale = Math.max(1, devicePixelRatio || 1)
      scene = JSON.parse(app.frame_at(bounds.width,bounds.height,scale,fixedClock ?? performance.now(),motion.matches))
      renderer.draw(scene,scale); syncInputs()
      if (new URLSearchParams(location.search).has('inspect')) canvas.dispatchEvent(new CustomEvent('deka:native-frame',{detail:scene,bubbles:true}))
      if (scene.animating && fixedClock === undefined) request = requestAnimationFrame(draw)
    } catch (error) { fail(error) }
  }
  let rendererLost = false
  const lost = event => { event.preventDefault(); rendererLost = true; cancelAnimationFrame(request) }
  const restored = () => { try { renderer.dispose(); renderer = new WebGLRenderer(canvas); rendererLost = false; draw() } catch(error) { fail(error) } }
  const pointer = event => {
    if (event.button !== 0) return
    canvas.focus(); const bounds=canvas.getBoundingClientRect()
    app.pointer(event.clientX-bounds.left,event.clientY-bounds.top); draw()
  }
  const key = event => { if(app.key(event.key,event.shiftKey)) event.preventDefault(); draw() }
  const blur = () => { app.blur(); draw() }
  // Explicit deterministic presentation clock for the same scripts as the native gate.
  const command = event => {
    if (!new URLSearchParams(location.search).has('inspect')) return
    const {time} = event.detail; fixedClock=time; draw()
  }
  canvas.addEventListener('pointerup',pointer); canvas.addEventListener('keydown',key); canvas.addEventListener('blur',blur)
  canvas.addEventListener('webglcontextlost',lost); canvas.addEventListener('webglcontextrestored',restored); canvas.addEventListener('deka:clock',command)
  const observer = new ResizeObserver(draw); observer.observe(canvas)
  window.addEventListener('resize',draw); motion.addEventListener('change',draw)
  // A monitor change can change DPR without changing CSS dimensions.
  let dpr
  const watchDpr = () => { dpr?.removeEventListener('change',changedDpr); dpr=matchMedia(`(resolution: ${devicePixelRatio}dppx)`);dpr.addEventListener('change',changedDpr) }
  const changedDpr = () => { watchDpr(); draw() }; watchDpr()
  draw()
  return {dispose() {
    if(disposed) return; disposed=true; cancelAnimationFrame(request);observer.disconnect()
    window.removeEventListener('resize',draw);motion.removeEventListener('change',draw);dpr.removeEventListener('change',changedDpr)
    canvas.removeEventListener('pointerup',pointer);canvas.removeEventListener('keydown',key);canvas.removeEventListener('blur',blur)
    canvas.removeEventListener('webglcontextlost',lost);canvas.removeEventListener('webglcontextrestored',restored);canvas.removeEventListener('deka:clock',command)
    for(const input of inputs.values()) input.remove(); renderer.dispose(); app.free()
  }}
}
export function unmount(handle) { handle.dispose() }
