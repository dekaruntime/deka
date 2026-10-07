// Platform adapter reused from the VM-era tour: Rust owns scenes and hit testing.
// ../deka-website/lib/deka-native/webgl.ts
var vertex = `#version 300 es
in vec2 position;
uniform vec4 rect;
uniform vec2 viewport;
out vec2 uv;
void main() { uv=position; vec2 p=rect.xy+position*rect.zw; gl_Position=vec4(p.x/viewport.x*2.-1.,1.-p.y/viewport.y*2.,0.,1.); }`;
var fragment = `#version 300 es
precision highp float;
in vec2 uv;
uniform vec4 color;
uniform vec2 dimensions;
uniform float radius;
uniform bool textured;
uniform sampler2D glyph;
out vec4 outputColor;
void main() {
  if(textured) { outputColor=texture(glyph,uv); outputColor.a *= color.a; return; }
  float r=min(radius,min(dimensions.x,dimensions.y)*.5);
  vec2 q=abs((uv-.5)*dimensions)-(dimensions*.5-r);
  float d=length(max(q,0.))+min(max(q.x,q.y),0.)-r;
  float coverage=1.-smoothstep(-.5,.5,d);
  outputColor=vec4(color.rgb,color.a*coverage);
}`;
var rgb = (n) => [(n >>> 16 & 255) / 255, (n >>> 8 & 255) / 255, (n & 255) / 255];

class WebGLRenderer {
  canvas;
  gl;
  program;
  buffer;
  white;
  textures = new Map;
  constructor(canvas) {
    this.canvas = canvas;
    const gl = canvas.getContext("webgl2", { alpha: false, antialias: true });
    if (!gl)
      throw new Error("This preview requires WebGL 2. Enable hardware acceleration or try another browser.");
    this.gl = gl;
    const shader = (type, source) => {
      const s = gl.createShader(type);
      gl.shaderSource(s, source);
      gl.compileShader(s);
      if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) {
        const error = gl.getShaderInfoLog(s);
        gl.deleteShader(s);
        throw new Error(error || "Shader compilation failed");
      }
      return s;
    };
    const v = shader(gl.VERTEX_SHADER, vertex), f = shader(gl.FRAGMENT_SHADER, fragment);
    this.program = gl.createProgram();
    gl.attachShader(this.program, v);
    gl.attachShader(this.program, f);
    gl.linkProgram(this.program);
    gl.deleteShader(v);
    gl.deleteShader(f);
    if (!gl.getProgramParameter(this.program, gl.LINK_STATUS))
      throw new Error(gl.getProgramInfoLog(this.program) || "Shader link failed");
    this.white = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, this.white);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, 1, 1, 0, gl.RGBA, gl.UNSIGNED_BYTE, new Uint8Array([255, 255, 255, 255]));
    this.buffer = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, this.buffer);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([0, 0, 1, 0, 0, 1, 0, 1, 1, 0, 1, 1]), gl.STATIC_DRAW);
  }
  draw(scene, scale) {
    const gl = this.gl;
    const width = Math.max(1, Math.round(scene.width * scale)), height = Math.max(1, Math.round(scene.height * scale));
    if (this.canvas.width !== width || this.canvas.height !== height) {
      this.canvas.width = width;
      this.canvas.height = height;
    }
    gl.viewport(0, 0, width, height);
    gl.disable(gl.SCISSOR_TEST);
    gl.clearColor(...rgb(scene.background), 1);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.useProgram(this.program);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.buffer);
    const position = gl.getAttribLocation(this.program, "position");
    gl.enableVertexAttribArray(position);
    gl.vertexAttribPointer(position, 2, gl.FLOAT, false, 0, 0);
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
    const uniform = (name) => gl.getUniformLocation(this.program, name);
    gl.uniform2f(uniform("viewport"), scene.width, scene.height);
    gl.activeTexture(gl.TEXTURE0);
    gl.uniform1i(uniform("glyph"), 0);
    const active = new Set(scene.image_ids);
    for (const [id, texture] of this.textures)
      if (!active.has(id)) {
        gl.deleteTexture(texture);
        this.textures.delete(id);
      }
    for (const image of scene.images) {
      if (this.textures.has(image.id))
        continue;
      const texture = gl.createTexture();
      gl.bindTexture(gl.TEXTURE_2D, texture);
      const filter = image.id.startsWith("world-") ? gl.NEAREST : gl.LINEAR;
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, filter);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, filter);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, image.width, image.height, 0, gl.RGBA, gl.UNSIGNED_BYTE, new Uint8Array(image.rgba));
      this.textures.set(image.id, texture);
    }
    gl.bindTexture(gl.TEXTURE_2D, this.white);
    gl.enable(gl.SCISSOR_TEST);
    for (const paint of scene.paint) {
      const clip = paint.clip;
      const x = Math.round(clip.x * scale), y = Math.round(clip.y * scale);
      const right = Math.round((clip.x + clip.width) * scale), bottom = Math.round((clip.y + clip.height) * scale);
      gl.scissor(x, height - bottom, Math.max(0, right - x), Math.max(0, bottom - y));
      const r = paint.rect;
      gl.uniform4f(uniform("rect"), r.x, r.y, r.width, r.height);
      gl.uniform2f(uniform("dimensions"), r.width, r.height);
      gl.uniform4f(uniform("color"), ...rgb(paint.color), paint.opacity);
      gl.uniform1f(uniform("radius"), paint.radius);
      gl.uniform1i(uniform("textured"), paint.image ? 1 : 0);
      if (paint.image)
        gl.bindTexture(gl.TEXTURE_2D, this.textures.get(paint.image));
      gl.drawArrays(gl.TRIANGLES, 0, 6);
    }
    gl.disable(gl.SCISSOR_TEST);
  }
  dispose() {
    for (const texture of this.textures.values())
      this.gl.deleteTexture(texture);
    this.textures.clear();
    this.gl.deleteTexture(this.white);
    this.gl.deleteBuffer(this.buffer);
    this.gl.deleteProgram(this.program);
  }
}
export {
  WebGLRenderer
};

let nextMount = 1
const mounts = new Map(), pending = new Set()
export function wake(id) {
  if (!mounts.has(id) || pending.has(id)) return
  pending.add(id)
  queueMicrotask(() => { pending.delete(id); mounts.get(id)?.() })
}
export function mount(app, canvas, inspect = false) {
  if (!(canvas instanceof HTMLCanvasElement)) { app.free(); throw new TypeError('launch requires an HTML canvas') }
  let renderer
  try { renderer = new WebGLRenderer(canvas) } catch (error) { app.free(); throw error }
  let disposed = false, request = 0, scene, fixedClock = inspect ? 0 : undefined
  const motion = matchMedia('(prefers-reduced-motion: reduce)')
  const inputs = new Map()
  const semantics = new Map()
  const previousTabIndex = canvas.getAttribute("tabindex")
  const previousAriaHidden = canvas.getAttribute("aria-hidden")
  canvas.tabIndex = -1; canvas.setAttribute("aria-hidden", "true")
  const parent = canvas.parentElement
  const fail = error => { console.error('Deka browser render failed:', error); canvas.dispatchEvent(new CustomEvent('deka:error', { detail: String(error), bubbles: true })) }
  const inspectedImages = new Map()
  const syncInputs = () => {
    const active = new Set()
    for (const control of JSON.parse(app.inputs())) {
      const node = scene.nodes.find(node => node.id === control.id)
      if (!node || node.rect.width <= 0 || node.rect.height <= 0) continue
      active.add(control.id)
      let input = inputs.get(control.id)
      if (!input) {
        input = document.createElement(control.tag === 'textarea' ? 'textarea' : 'input'); input.setAttribute('aria-label', control.tag === 'textarea' ? 'Deka text area' : 'Deka text input')
        input.style.cssText = 'position:absolute;box-sizing:border-box;font:inherit;color:inherit;background:transparent;border:1px solid currentColor;border-radius:4px;padding:4px;'
        input.addEventListener('compositionstart', () => { input.dekaComposing = true })
        input.addEventListener('compositionend', () => { input.dekaComposing = false; try { app.input(control.id, input.value); draw() } catch(error) { fail(error) } })
        input.addEventListener('input', () => { if (!input.dekaComposing) { try { app.input(control.id, input.value); draw() } catch(error) { fail(error) } } })
        input.addEventListener('keydown', event => { if(event.isComposing) return; try { app.key_to(control.id,event.key); draw() } catch(error) { fail(error) } })
        input.addEventListener('focus', () => { try { app.focus_node(control.id, input.matches(":focus-visible")); draw() } catch(error) { fail(error) } })
        input.addEventListener('blur', () => { app.blur(); draw() })
        inputs.set(control.id,input); parent.append(input)
      }
      const value = control.value ?? ""
      input.placeholder = control.placeholder ?? ""
      if (!input.dekaComposing && (control.controlled || input.dekaObservedValue !== value) && input.value !== value) {
        const selection = [input.selectionStart, input.selectionEnd]
        input.value = value
        if (document.activeElement === input && selection.every(index => index !== null)) input.setSelectionRange(Math.min(selection[0],input.value.length),Math.min(selection[1],input.value.length))
      }
      if (!input.dekaComposing) input.dekaObservedValue = value
      const bounds = canvas.getBoundingClientRect(), origin = parent.getBoundingClientRect()
      Object.assign(input.style,{left:`${bounds.left-origin.left+node.rect.x}px`,top:`${bounds.top-origin.top+node.rect.y}px`,width:`${node.rect.width}px`,height:`${node.rect.height}px`})
    }
    for (const [id,input] of inputs) if (!active.has(id)) { input.remove(); inputs.delete(id) }
  }
  const syncSemantics = () => {
    const descriptors = JSON.parse(app.semantic_nodes())
    const active = new Set()
    for (const node of descriptors) {
      if (node.hidden) continue
      active.add(node.id)
      let element = inputs.get(node.id) ?? semantics.get(node.id)
      if (!element) {
        element = document.createElement(node.role === 'button' ? 'button' : node.role === 'text' ? 'span' : 'div')
        if (node.role === 'group') { element.setAttribute('role', 'group'); element.style.display='contents' }
        else if (node.role === 'button') {
          element.style.cssText='position:absolute;opacity:0;pointer-events:none;'
          element.addEventListener('click', () => { try { app.activate(node.id); draw() } catch(error) { fail(error) } })
        } else element.style.cssText='position:absolute;width:1px;height:1px;overflow:hidden;clip-path:inset(50%);'
        element.addEventListener('focus', () => { try { app.focus_node(node.id, element.matches(":focus-visible")); draw() } catch(error) { fail(error) } })
        element.addEventListener('blur', () => { app.blur(); draw() })
        semantics.set(node.id, element)
      }
      if (node.name) element.setAttribute('aria-label', node.name)
      else if (!inputs.has(node.id)) element.removeAttribute('aria-label')
      element.setAttribute('aria-disabled', String(node.disabled))
      if ('disabled' in element) element.disabled = node.disabled
      if (node.tabIndex !== null && !node.disabled) element.tabIndex = node.tabIndex
      else if (!inputs.has(node.id)) element.removeAttribute('tabindex')
      if (node.role === 'text') element.textContent = node.name
      const container = semantics.get(node.parent) ?? parent
      // Preserve DOM source order without moving focused controls each frame.
      if (element.parentElement !== container) container.append(element)
      if (node.role === 'button') {
        const box = scene.nodes.find(n=>n.id===node.id)?.rect
        if (box) {
          const bounds=canvas.getBoundingClientRect(), origin=parent.getBoundingClientRect()
          Object.assign(element.style,{left:`${bounds.left-origin.left+box.x}px`,top:`${bounds.top-origin.top+box.y}px`,width:`${box.width}px`,height:`${box.height}px`})
        }
      }
    }
    for (const [id, element] of semantics) if (!active.has(id)) { element.remove(); semantics.delete(id) }
    for (const [id, input] of inputs) if (!active.has(id)) input.hidden = true; else input.hidden = false
  }
  const draw = () => {
    cancelAnimationFrame(request); request = 0
    if (disposed || rendererLost) return
    try {
      const bounds = canvas.getBoundingClientRect()
      const scale = Math.max(1, devicePixelRatio || 1)
      scene = JSON.parse(app.frame_at(bounds.width,bounds.height,scale,fixedClock ?? performance.now(),motion.matches))
      renderer.draw(scene,scale); syncInputs(); syncSemantics()
      if (inspect) {
        const {image_ids, ...rendered} = scene
        const active = new Set(image_ids)
        for (const id of inspectedImages.keys()) if (!active.has(id)) inspectedImages.delete(id)
        for (const image of scene.images) inspectedImages.set(image.id, image)
        canvas.dispatchEvent(new CustomEvent('deka:native-frame',{detail:{...rendered,images:[...inspectedImages.values()].sort((a,b)=>a.id.localeCompare(b.id))},bubbles:true}))
      }
      if (scene.animating && fixedClock === undefined) request = requestAnimationFrame(draw)
    } catch (error) { fail(error) }
  }
  let rendererLost = false
  const lost = event => { event.preventDefault(); rendererLost = true; cancelAnimationFrame(request) }
  const restored = () => { try { renderer.dispose(); renderer = new WebGLRenderer(canvas); rendererLost = false; draw() } catch(error) { fail(error) } }
  const pointer = event => {
    if (event.button !== 0) return
    canvas.focus(); const bounds=canvas.getBoundingClientRect()
    try { app.pointer(event.clientX-bounds.left,event.clientY-bounds.top); draw() } catch(error) { fail(error) }
  }
  const key = event => { try { if(app.key(event.key,event.shiftKey)) event.preventDefault(); draw() } catch(error) { fail(error) } }
  const blur = () => { app.blur(); draw() }
  // Explicit deterministic presentation clock for the same scripts as the native gate.
  const command = event => {
    if (!inspect) return
    const {time} = event.detail; fixedClock=time; draw()
  }
  canvas.addEventListener('pointerup',pointer); canvas.addEventListener('keydown',key); canvas.addEventListener('blur',blur)
  canvas.addEventListener('webglcontextlost',lost); canvas.addEventListener('webglcontextrestored',restored); if (inspect) canvas.addEventListener('deka:clock',command)
  const observer = new ResizeObserver(draw); observer.observe(canvas)
  window.addEventListener('resize',draw); motion.addEventListener('change',draw)
  // A monitor change can change DPR without changing CSS dimensions.
  let dpr
  const watchDpr = () => { dpr?.removeEventListener('change',changedDpr); dpr=matchMedia(`(resolution: ${devicePixelRatio}dppx)`);dpr.addEventListener('change',changedDpr) }
  const changedDpr = () => { watchDpr(); draw() }; watchDpr()
  const mountId = nextMount++; mounts.set(mountId, draw); app.wake_on(mountId)
  draw()
  return {dispose() {
    if(disposed) return; disposed=true; mounts.delete(mountId); pending.delete(mountId); cancelAnimationFrame(request);observer.disconnect()
    window.removeEventListener('resize',draw);motion.removeEventListener('change',draw);dpr.removeEventListener('change',changedDpr)
    canvas.removeEventListener('pointerup',pointer);canvas.removeEventListener('keydown',key);canvas.removeEventListener('blur',blur)
    canvas.removeEventListener('webglcontextlost',lost);canvas.removeEventListener('webglcontextrestored',restored);canvas.removeEventListener('deka:clock',command)
    for(const input of inputs.values()) input.remove(); for(const element of semantics.values()) element.remove()
    if (previousTabIndex === null) canvas.removeAttribute("tabindex"); else canvas.setAttribute("tabindex",previousTabIndex)
    if (previousAriaHidden === null) canvas.removeAttribute('aria-hidden'); else canvas.setAttribute('aria-hidden',previousAriaHidden)
    renderer.dispose(); app.free()
  }}
}
export function unmount(handle) { handle.dispose() }

export function reportPanic(message) { console.error(`Deka Rust panic: ${message}`) }
