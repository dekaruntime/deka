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
    const active = new Set(scene.images.map((image) => image.id));
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
