/**
 * Wasserfall mit WebGL2.
 *
 * Jede Zeile wird mit ihrem Frequenzbereich gespeichert. Der Shader rechnet jede
 * Bildschirmspalte in eine Frequenz um und schlägt sie in der jeweiligen Zeile nach.
 * Dadurch wandert der alte Wasserfall beim Umstimmen oder Zoomen korrekt mit (wie in SDR#).
 */

export type PaletteName = 'sdrsharp' | 'turbo' | 'grau';

const PALETTES: Record<PaletteName, [number, number, number][]> = {
  sdrsharp: [
    [0, 0, 0],
    [0, 0, 80],
    [0, 0, 200],
    [0, 160, 255],
    [0, 255, 200],
    [255, 255, 0],
    [255, 128, 0],
    [255, 0, 0],
    [255, 255, 255],
  ],
  turbo: [
    [48, 18, 59],
    [70, 107, 227],
    [40, 187, 236],
    [49, 242, 153],
    [162, 252, 60],
    [237, 208, 58],
    [251, 128, 34],
    [208, 47, 5],
    [122, 4, 3],
  ],
  grau: [
    [0, 0, 0],
    [255, 255, 255],
  ],
};

export function paletteColors(name: PaletteName): Uint8Array {
  const stops = PALETTES[name];
  const out = new Uint8Array(256 * 4);
  for (let i = 0; i < 256; i++) {
    const t = (i / 255) * (stops.length - 1);
    const k = Math.min(Math.floor(t), stops.length - 2);
    const f = t - k;
    for (let c = 0; c < 3; c++) out[i * 4 + c] = stops[k][c] + (stops[k + 1][c] - stops[k][c]) * f;
    out[i * 4 + 3] = 255;
  }
  return out;
}

const VS = `#version 300 es
in vec2 aPos;
out vec2 vUv;
void main() {
  vUv = vec2(aPos.x * 0.5 + 0.5, 0.5 - aPos.y * 0.5);
  gl_Position = vec4(aPos, 0.0, 1.0);
}`;

const FS = `#version 300 es
precision highp float;
uniform sampler2D uData;
uniform sampler2D uMeta;
uniform sampler2D uPalette;
uniform float uHead;
uniform float uRows;
uniform vec2 uView;   // sichtbarer Bereich in MHz (relativ zu uRef)
uniform vec2 uLevels; // Schwarz-/Weißpunkt als Bytewert 0..1
in vec2 vUv;
out vec4 outColor;
void main() {
  float age = floor(vUv.y * uRows);
  float row = mod(uHead - age + uRows, uRows);
  vec2 m = texelFetch(uMeta, ivec2(0, int(row)), 0).rg;
  float f = mix(uView.x, uView.y, vUv.x);
  float u = (f - m.x) / (m.y - m.x);
  if (m.y <= m.x || u < 0.0 || u >= 1.0) { outColor = vec4(0.03, 0.03, 0.06, 1.0); return; }
  float v = texture(uData, vec2(u, (row + 0.5) / uRows)).r;
  float t = clamp((v - uLevels.x) / max(uLevels.y - uLevels.x, 0.001), 0.0, 1.0);
  outColor = texture(uPalette, vec2(t, 0.5));
}`;

export class Waterfall {
  private gl: WebGL2RenderingContext;
  private prog: WebGLProgram;
  private dataTex: WebGLTexture;
  private metaTex: WebGLTexture;
  private palTex: WebGLTexture;
  private width = 0;
  private rows = 0;
  private head = 0;
  private meta = new Float32Array(0);
  /** Bezugsfrequenz in Hz, damit Float32 im Shader genau genug bleibt. */
  private ref = 0;
  private uni: Record<string, WebGLUniformLocation | null> = {};

  constructor(private canvas: HTMLCanvasElement) {
    const gl = canvas.getContext('webgl2', { antialias: false, preserveDrawingBuffer: false });
    if (!gl) throw new Error('WebGL2 wird von diesem Browser nicht unterstützt');
    this.gl = gl;
    this.prog = this.link(VS, FS);
    for (const n of ['uData', 'uMeta', 'uPalette', 'uHead', 'uRows', 'uView', 'uLevels'])
      this.uni[n] = gl.getUniformLocation(this.prog, n);
    const vbo = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, vbo);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]), gl.STATIC_DRAW);
    const loc = gl.getAttribLocation(this.prog, 'aPos');
    gl.enableVertexAttribArray(loc);
    gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);
    this.dataTex = this.tex();
    this.metaTex = this.tex();
    this.palTex = this.tex();
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
    this.setPalette('sdrsharp');
  }

  private tex(): WebGLTexture {
    const gl = this.gl;
    const t = gl.createTexture()!;
    gl.bindTexture(gl.TEXTURE_2D, t);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    return t;
  }

  private link(vs: string, fs: string): WebGLProgram {
    const gl = this.gl;
    const sh = (type: number, src: string) => {
      const s = gl.createShader(type)!;
      gl.shaderSource(s, src);
      gl.compileShader(s);
      if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s) ?? 'Shader');
      return s;
    };
    const p = gl.createProgram()!;
    gl.attachShader(p, sh(gl.VERTEX_SHADER, vs));
    gl.attachShader(p, sh(gl.FRAGMENT_SHADER, fs));
    gl.linkProgram(p);
    if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(p) ?? 'Link');
    return p;
  }

  setPalette(name: PaletteName) {
    const gl = this.gl;
    gl.bindTexture(gl.TEXTURE_2D, this.palTex);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, 256, 1, 0, gl.RGBA, gl.UNSIGNED_BYTE, paletteColors(name));
  }

  /** Passt die Texturen an eine neue Zeilenbreite/-anzahl an (löscht den Verlauf). */
  private resize(width: number, rows: number) {
    const gl = this.gl;
    this.width = width;
    this.rows = rows;
    this.head = 0;
    this.meta = new Float32Array(rows * 2);
    gl.bindTexture(gl.TEXTURE_2D, this.dataTex);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.R8, width, rows, 0, gl.RED, gl.UNSIGNED_BYTE, new Uint8Array(width * rows));
    gl.bindTexture(gl.TEXTURE_2D, this.metaTex);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RG32F, 1, rows, 0, gl.RG, gl.FLOAT, this.meta);
  }

  addRow(start: number, end: number, data: Uint8Array) {
    const rows = Math.max(16, this.canvas.height);
    if (data.length !== this.width || rows !== this.rows) this.resize(data.length, rows);
    if (this.ref === 0 || Math.abs(start - this.ref) > 50e6) this.rebase(start);
    const gl = this.gl;
    this.head = (this.head + 1) % this.rows;
    gl.bindTexture(gl.TEXTURE_2D, this.dataTex);
    gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, this.head, this.width, 1, gl.RED, gl.UNSIGNED_BYTE, data);
    const m = new Float32Array([(start - this.ref) / 1e6, (end - this.ref) / 1e6]);
    this.meta.set(m, this.head * 2);
    gl.bindTexture(gl.TEXTURE_2D, this.metaTex);
    gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, this.head, 1, 1, gl.RG, gl.FLOAT, m);
  }

  /** Neuer Bezugspunkt bei großen Sprüngen (alte Zeilen werden dann ausgeblendet). */
  private rebase(ref: number) {
    this.ref = ref;
    this.meta.fill(0);
    if (this.rows > 0) {
      const gl = this.gl;
      gl.bindTexture(gl.TEXTURE_2D, this.metaTex);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RG32F, 1, this.rows, 0, gl.RG, gl.FLOAT, this.meta);
    }
  }

  draw(viewStart: number, viewEnd: number, levelMin: number, levelMax: number) {
    const gl = this.gl;
    if (this.rows === 0) return;
    gl.viewport(0, 0, this.canvas.width, this.canvas.height);
    gl.useProgram(this.prog);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, this.dataTex);
    gl.activeTexture(gl.TEXTURE1);
    gl.bindTexture(gl.TEXTURE_2D, this.metaTex);
    gl.activeTexture(gl.TEXTURE2);
    gl.bindTexture(gl.TEXTURE_2D, this.palTex);
    gl.uniform1i(this.uni.uData, 0);
    gl.uniform1i(this.uni.uMeta, 1);
    gl.uniform1i(this.uni.uPalette, 2);
    gl.uniform1f(this.uni.uHead, this.head);
    gl.uniform1f(this.uni.uRows, this.rows);
    gl.uniform2f(this.uni.uView, (viewStart - this.ref) / 1e6, (viewEnd - this.ref) / 1e6);
    gl.uniform2f(this.uni.uLevels, levelMin / 255, levelMax / 255);
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
  }
}
