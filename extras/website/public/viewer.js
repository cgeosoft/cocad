/**
 * Small WebGL2 model viewer. No library.
 * Reads the meshes that scripts/website-assets.py writes and draws them in
 * three modes: solid (flat shaded, like the app), wire and slice (layer
 * outlines, cut from the mesh in this browser).
 */
(function () {
  "use strict";

  const LAYER_HEIGHT = 0.2;
  const ORANGE = [1, 0.576, 0];
  const VIEWS = {
    iso: [-0.9, 0.55],
    top: [-Math.PI / 2, 1.55],
    front: [-Math.PI / 2, 0.02],
    right: [0, 0.02],
  };

  // Meshes -----------------------------------------------------------------

  const cache = new Map();

  function loadMesh(url) {
    if (!cache.has(url)) {
      cache.set(url, fetch(url).then((r) => {
        if (!r.ok) throw new Error(`${url}: ${r.status}`);
        return r.arrayBuffer();
      }).then(parseMesh).catch((err) => {
        cache.delete(url);
        throw err;
      }));
    }
    return cache.get(url);
  }

  function parseMesh(buffer) {
    const head = new DataView(buffer);
    const vertexCount = head.getUint32(0, true);
    const indexCount = head.getUint32(4, true);
    const low = [8, 12, 16].map((o) => head.getFloat32(o, true));
    const size = [20, 24, 28].map((o) => head.getFloat32(o, true));
    const packed = new Uint16Array(buffer, 32, vertexCount * 3);
    const positions = new Float32Array(vertexCount * 3);
    for (let i = 0; i < packed.length; i++) {
      positions[i] = low[i % 3] + (packed[i] / 65535) * size[i % 3];
    }
    const index = new Uint16Array(buffer, 32 + vertexCount * 6, indexCount);
    return { positions, index, low, size, edges: null, layers: null };
  }

  function wireEdges(mesh) {
    if (mesh.edges) return mesh.edges;
    const seen = new Set();
    const out = [];
    const add = (a, b) => {
      const key = a < b ? a * 65536 + b : b * 65536 + a;
      if (!seen.has(key)) { seen.add(key); out.push(a, b); }
    };
    for (let i = 0; i < mesh.index.length; i += 3) {
      add(mesh.index[i], mesh.index[i + 1]);
      add(mesh.index[i + 1], mesh.index[i + 2]);
      add(mesh.index[i + 2], mesh.index[i]);
    }
    mesh.edges = new Uint16Array(out);
    return mesh.edges;
  }

  /** Cuts the mesh at each layer. Result: lines with a normal, sorted by layer. */
  function sliceMesh(mesh) {
    if (mesh.layers) return mesh.layers;
    const { positions: p, index, low, size } = mesh;
    const count = Math.max(1, Math.round(size[2] / LAYER_HEIGHT));
    const buckets = Array.from({ length: count }, () => []);
    const bottom = low[2];
    const cut = (a, b, z, out) => {
      const za = p[a * 3 + 2];
      const zb = p[b * 3 + 2];
      if ((za >= z) === (zb >= z)) return;
      const t = (z - za) / (zb - za);
      out.push(p[a * 3] + t * (p[b * 3] - p[a * 3]), p[a * 3 + 1] + t * (p[b * 3 + 1] - p[a * 3 + 1]), z);
    };
    for (let i = 0; i < index.length; i += 3) {
      const a = index[i], b = index[i + 1], c = index[i + 2];
      const az = p[a * 3 + 2], bz = p[b * 3 + 2], cz = p[c * 3 + 2];
      const zMin = Math.min(az, bz, cz), zMax = Math.max(az, bz, cz);
      const first = Math.max(0, Math.ceil((zMin - bottom) / LAYER_HEIGHT - 0.5));
      const last = Math.min(count - 1, Math.floor((zMax - bottom) / LAYER_HEIGHT - 0.5));
      if (first > last) continue;
      const ux = p[b * 3] - p[a * 3], uy = p[b * 3 + 1] - p[a * 3 + 1], uz = bz - az;
      const vx = p[c * 3] - p[a * 3], vy = p[c * 3 + 1] - p[a * 3 + 1], vz = cz - az;
      let nx = uy * vz - uz * vy, ny = uz * vx - ux * vz, nz = ux * vy - uy * vx;
      const len = Math.hypot(nx, ny, nz) || 1;
      nx /= len; ny /= len; nz /= len;
      for (let layer = first; layer <= last; layer++) {
        const z = bottom + (layer + 0.5) * LAYER_HEIGHT;
        const hit = [];
        cut(a, b, z, hit); cut(b, c, z, hit); cut(c, a, z, hit);
        if (hit.length === 6) {
          buckets[layer].push(hit[0], hit[1], hit[2], nx, ny, nz, hit[3], hit[4], hit[5], nx, ny, nz);
        }
      }
    }
    let total = 0;
    const offsets = new Uint32Array(count + 1);
    buckets.forEach((b, i) => { offsets[i] = total / 6; total += b.length; });
    offsets[count] = total / 6;
    const data = new Float32Array(total);
    let at = 0;
    buckets.forEach((b) => { data.set(b, at); at += b.length; });
    mesh.layers = { data, offsets, count };
    return mesh.layers;
  }

  // Math -------------------------------------------------------------------

  function perspective(fov, aspect, near, far, shift) {
    const f = 1 / Math.tan(fov / 2);
    const nf = 1 / (near - far);
    return [f / aspect, 0, 0, 0, 0, f, 0, 0, -shift, 0, (far + near) * nf, -1, 0, 0, 2 * far * near * nf, 0];
  }

  function lookAt(eye, target) {
    const z = norm(sub(eye, target));
    const x = norm(cross([0, 0, 1], z));
    const y = cross(z, x);
    return [x[0], y[0], z[0], 0, x[1], y[1], z[1], 0, x[2], y[2], z[2], 0, -dot(x, eye), -dot(y, eye), -dot(z, eye), 1];
  }

  function multiply(a, b) {
    const out = new Array(16);
    for (let c = 0; c < 4; c++) {
      for (let r = 0; r < 4; r++) {
        out[c * 4 + r] = a[r] * b[c * 4] + a[r + 4] * b[c * 4 + 1] + a[r + 8] * b[c * 4 + 2] + a[r + 12] * b[c * 4 + 3];
      }
    }
    return out;
  }

  const sub = (a, b) => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
  const dot = (a, b) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
  const cross = (a, b) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
  const norm = (a) => { const l = Math.hypot(a[0], a[1], a[2]) || 1; return [a[0] / l, a[1] / l, a[2] / l]; };
  const ease = (t) => 1 - Math.pow(1 - t, 3);

  // Shaders ----------------------------------------------------------------

  const SOLID_VS = `#version 300 es
    uniform mat4 uVP;
    in vec3 aPos;
    out vec3 vPos;
    void main() { vPos = aPos; gl_Position = uVP * vec4(aPos, 1.0); }`;

  const SOLID_FS = `#version 300 es
    precision highp float;
    uniform vec3 uEye; uniform vec3 uColor; uniform float uClip;
    in vec3 vPos;
    out vec4 color;
    void main() {
      if (vPos.z > uClip) discard;
      vec3 n = normalize(cross(dFdx(vPos), dFdy(vPos)));
      vec3 v = normalize(uEye - vPos);
      if (dot(n, v) < 0.0) n = -n;
      vec3 key = normalize(v + vec3(0.35, -0.2, 0.75));
      float d = max(dot(n, key), 0.0);
      float s = pow(max(dot(n, normalize(key + v)), 0.0), 40.0);
      color = vec4(uColor * (0.28 + 0.72 * d) + vec3(0.14) * s, 1.0);
    }`;

  const LINE_VS = `#version 300 es
    uniform mat4 uVP; uniform vec3 uEye;
    in vec3 aPos; in vec3 aNormal;
    out float vShade;
    void main() {
      vec3 v = normalize(uEye - aPos);
      vec3 key = normalize(v + vec3(0.35, -0.2, 0.75));
      vShade = 0.3 + 0.7 * abs(dot(aNormal, key));
      gl_Position = uVP * vec4(aPos, 1.0);
    }`;

  const LINE_FS = `#version 300 es
    precision highp float;
    uniform vec4 uColor;
    in float vShade;
    out vec4 color;
    void main() { color = vec4(uColor.rgb * vShade, uColor.a); }`;

  const GRID_VS = `#version 300 es
    uniform mat4 uVP;
    in vec3 aPos; in vec4 aColor;
    out vec4 vColor; out vec3 vPos;
    void main() { vColor = aColor; vPos = aPos; gl_Position = uVP * vec4(aPos, 1.0); }`;

  const GRID_FS = `#version 300 es
    precision highp float;
    uniform vec3 uCenter; uniform float uRadius;
    in vec4 vColor; in vec3 vPos;
    out vec4 color;
    void main() {
      float f = 1.0 - smoothstep(0.35, 1.0, length(vPos.xy - uCenter.xy) / uRadius);
      color = vec4(vColor.rgb, vColor.a * f);
    }`;

  function program(gl, vs, fs) {
    const p = gl.createProgram();
    [[gl.VERTEX_SHADER, vs], [gl.FRAGMENT_SHADER, fs]].forEach(([type, src]) => {
      const s = gl.createShader(type);
      gl.shaderSource(s, src);
      gl.compileShader(s);
      if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s));
      gl.attachShader(p, s);
    });
    gl.linkProgram(p);
    if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(p));
    const u = {};
    const n = gl.getProgramParameter(p, gl.ACTIVE_UNIFORMS);
    for (let i = 0; i < n; i++) {
      const name = gl.getActiveUniform(p, i).name;
      u[name] = gl.getUniformLocation(p, name);
    }
    return { p, u, aPos: gl.getAttribLocation(p, "aPos"), aNormal: gl.getAttribLocation(p, "aNormal"), aColor: gl.getAttribLocation(p, "aColor") };
  }

  // Viewer -----------------------------------------------------------------

  class Viewer {
    constructor(canvas, options = {}) {
      const gl = canvas.getContext("webgl2", { antialias: true, alpha: true, premultipliedAlpha: false });
      if (!gl) throw new Error("WebGL2 is not available");
      this.gl = gl;
      this.canvas = canvas;
      this.shift = options.shift || 0;
      this.autoRotate = !!options.autoRotate;
      this.zoomable = !!options.zoom;
      this.margin = options.margin || 1.08;
      this.mode = "solid";
      this.layer = 1;
      this.clip = 1;
      this.visible = false;
      this.mesh = null;
      this.yaw = VIEWS.iso[0]; this.pitch = VIEWS.iso[1];
      this.yawGoal = this.yaw; this.pitchGoal = this.pitch;
      this.dist = 200; this.distGoal = 200;
      this.center = [0, 0, 0]; this.centerGoal = [0, 0, 0];
      this.zoom = 1;
      this.solid = program(gl, SOLID_VS, SOLID_FS);
      this.line = program(gl, LINE_VS, LINE_FS);
      this.grid = program(gl, GRID_VS, GRID_FS);
      gl.enable(gl.DEPTH_TEST);
      gl.enable(gl.BLEND);
      gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
      gl.clearColor(0, 0, 0, 0);
      this.buffers = {};
      this.bindInput();
      new ResizeObserver(() => this.resize()).observe(canvas);
      new IntersectionObserver(([e]) => {
        this.visible = e.isIntersecting;
        if (this.visible) this.wake();
      }, { threshold: 0.02 }).observe(canvas);
      this.resize();
    }

    bindInput() {
      const c = this.canvas;
      let last = null;
      let armed = false;
      c.addEventListener("pointerdown", (e) => {
        last = [e.clientX, e.clientY];
        armed = true;
        this.held = true;
        c.setPointerCapture(e.pointerId);
      });
      c.addEventListener("pointermove", (e) => {
        if (!last) return;
        this.yawGoal -= (e.clientX - last[0]) * 0.008;
        this.pitchGoal = Math.max(-1.5, Math.min(1.55, this.pitchGoal + (e.clientY - last[1]) * 0.008));
        last = [e.clientX, e.clientY];
        this.touched = true;
        this.wake();
      });
      const end = () => { last = null; this.held = false; };
      c.addEventListener("pointerup", end);
      c.addEventListener("pointercancel", end);
      c.addEventListener("pointerleave", () => { armed = false; });
      c.addEventListener("dblclick", () => this.setView("iso"));
      if (this.zoomable) {
        // The wheel zooms only after a click on the view, so the page scroll stays free.
        c.addEventListener("wheel", (e) => {
          if (!armed) return;
          e.preventDefault();
          this.zoom = Math.max(0.3, Math.min(3, this.zoom * Math.exp(e.deltaY * 0.001)));
          this.fit();
          this.wake();
        }, { passive: false });
      }
    }

    resize() {
      const dpr = Math.min(window.devicePixelRatio || 1, 2);
      const w = Math.max(1, Math.round(this.canvas.clientWidth * dpr));
      const h = Math.max(1, Math.round(this.canvas.clientHeight * dpr));
      if (this.canvas.width !== w || this.canvas.height !== h) {
        this.canvas.width = w;
        this.canvas.height = h;
      }
      this.gl.viewport(0, 0, w, h);
      this.fit();
      this.wake();
    }

    /** Shows a mesh. `reveal` builds it from the bottom up, as a short render cue. */
    setMesh(mesh, { reveal = true, snap = false } = {}) {
      const gl = this.gl;
      this.mesh = mesh;
      Object.values(this.buffers).forEach((b) => gl.deleteBuffer(b));
      const vb = gl.createBuffer();
      gl.bindBuffer(gl.ARRAY_BUFFER, vb);
      gl.bufferData(gl.ARRAY_BUFFER, mesh.positions, gl.STATIC_DRAW);
      const ib = gl.createBuffer();
      gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, ib);
      gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, mesh.index, gl.STATIC_DRAW);
      this.buffers = { vb, ib, grid: this.makeGrid(mesh) };
      this.centerGoal = [0, 1, 2].map((k) => mesh.low[k] + mesh.size[k] / 2);
      this.fit();
      if (snap) { this.dist = this.distGoal; this.center = this.centerGoal.slice(); }
      if (this.mode === "wire") this.prepareWire();
      if (this.mode === "slice") this.prepareSlice();
      this.revealFrom = reveal ? performance.now() : 0;
      this.wake();
    }

    makeGrid(mesh) {
      const gl = this.gl;
      const span = Math.max(mesh.size[0], mesh.size[1], mesh.size[2]);
      const step = span > 120 ? 20 : 10;
      const half = Math.ceil((span * 1.6) / step) * step;
      const c = [Math.round((mesh.low[0] + mesh.size[0] / 2) / step) * step, Math.round((mesh.low[1] + mesh.size[1] / 2) / step) * step];
      const z = mesh.low[2];
      const data = [];
      for (let t = -half; t <= half; t += step) {
        const x = c[0] + t, y = c[1] + t;
        const axisX = y === 0, axisY = x === 0;
        const rgba = (axis, col) => (axis ? col : [1, 1, 1, 0.09]);
        data.push(c[0] - half, y, z, ...rgba(axisX, [0.85, 0.25, 0.25, 0.7]), c[0] + half, y, z, ...rgba(axisX, [0.85, 0.25, 0.25, 0.7]));
        data.push(x, c[1] - half, z, ...rgba(axisY, [0.25, 0.75, 0.35, 0.7]), x, c[1] + half, z, ...rgba(axisY, [0.25, 0.75, 0.35, 0.7]));
      }
      this.gridCount = data.length / 7;
      this.gridCenter = [c[0], c[1], z];
      this.gridRadius = half;
      const b = gl.createBuffer();
      gl.bindBuffer(gl.ARRAY_BUFFER, b);
      gl.bufferData(gl.ARRAY_BUFFER, new Float32Array(data), gl.STATIC_DRAW);
      return b;
    }

    fit() {
      if (!this.mesh) return;
      const r = 0.5 * Math.hypot(...this.mesh.size);
      const aspect = this.canvas.clientWidth / Math.max(1, this.canvas.clientHeight);
      const fov = 0.6;
      const tall = r / Math.sin(fov / 2);
      this.distGoal = (aspect < 1 ? tall / aspect : tall) * this.margin * this.zoom;
    }

    setView(name) {
      const [yaw, pitch] = VIEWS[name];
      const turns = Math.round((this.yawGoal - yaw) / (Math.PI * 2));
      this.yawGoal = yaw + turns * Math.PI * 2;
      this.pitchGoal = pitch;
      this.zoom = 1;
      this.fit();
      this.wake();
    }

    setMode(mode) {
      this.mode = mode;
      if (this.mesh && mode === "wire") this.prepareWire();
      if (this.mesh && mode === "slice") this.prepareSlice();
      this.wake();
    }

    prepareWire() {
      const gl = this.gl;
      if (this.buffers.wire) gl.deleteBuffer(this.buffers.wire);
      const edges = wireEdges(this.mesh);
      this.buffers.wire = gl.createBuffer();
      gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, this.buffers.wire);
      gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, edges, gl.STATIC_DRAW);
      this.wireCount = edges.length;
    }

    prepareSlice() {
      const gl = this.gl;
      if (this.buffers.layers) gl.deleteBuffer(this.buffers.layers);
      const layers = sliceMesh(this.mesh);
      this.buffers.layers = gl.createBuffer();
      gl.bindBuffer(gl.ARRAY_BUFFER, this.buffers.layers);
      gl.bufferData(gl.ARRAY_BUFFER, layers.data, gl.STATIC_DRAW);
      this.layers = layers;
    }

    setLayer(fraction) {
      this.layer = Math.max(0, Math.min(1, fraction));
      this.wake();
    }

    /** Runs the layer position from 0 to 1. Resolves at the end. */
    sweep(ms) {
      return new Promise((done) => {
        const from = performance.now();
        const step = (now) => {
          const t = Math.min(1, (now - from) / ms);
          if (this.sweepId !== id) return done();
          this.setLayer(t * t * (3 - 2 * t));
          if (this.onLayer) this.onLayer(this.layer);
          if (t < 1) requestAnimationFrame(step); else done();
        };
        const id = (this.sweepId = (this.sweepId || 0) + 1);
        requestAnimationFrame(step);
      });
    }

    stopSweep() { this.sweepId = (this.sweepId || 0) + 1; }

    wake() {
      if (this.running || !this.visible) return;
      this.running = true;
      this.then = performance.now();
      requestAnimationFrame((t) => this.frame(t));
    }

    frame(now) {
      if (!this.visible) { this.running = false; return; }
      const dt = Math.min(0.05, (now - this.then) / 1000);
      this.then = now;
      const k = 1 - Math.exp(-dt * 9);
      if (this.autoRotate && !this.held) this.yawGoal += dt * 0.28;
      this.yaw += (this.yawGoal - this.yaw) * k;
      this.pitch += (this.pitchGoal - this.pitch) * k;
      this.dist += (this.distGoal - this.dist) * k;
      this.center = this.center.map((v, i) => v + (this.centerGoal[i] - v) * k);
      let clip = 1;
      if (this.revealFrom) {
        const t = (now - this.revealFrom) / 420;
        clip = t >= 1 ? 1 : ease(Math.max(t, 0));
        if (t >= 1) this.revealFrom = 0;
      }
      this.clip = clip;
      this.draw();
      const moving = Math.abs(this.yawGoal - this.yaw) > 1e-4 || Math.abs(this.pitchGoal - this.pitch) > 1e-4
        || Math.abs(this.distGoal - this.dist) > 0.01 || this.revealFrom || this.autoRotate
        || Math.abs(this.centerGoal[2] - this.center[2]) > 0.01;
      if (moving) requestAnimationFrame((t) => this.frame(t));
      else { this.running = false; }
    }

    draw() {
      const gl = this.gl;
      gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
      if (!this.mesh) return;
      const cp = Math.cos(this.pitch);
      const eye = [
        this.center[0] + this.dist * cp * Math.cos(this.yaw),
        this.center[1] + this.dist * cp * Math.sin(this.yaw),
        this.center[2] + this.dist * Math.sin(this.pitch),
      ];
      const aspect = this.canvas.width / Math.max(1, this.canvas.height);
      const vp = multiply(perspective(0.6, aspect, this.dist / 50, this.dist * 6, this.shift), lookAt(eye, this.center));
      this.drawGrid(vp);
      if (this.mode === "slice") return this.drawSlice(vp, eye);
      this.drawSolid(vp, eye);
      if (this.mode === "wire") this.drawWire(vp, eye);
    }

    drawGrid(vp) {
      const gl = this.gl, g = this.grid;
      gl.useProgram(g.p);
      gl.uniformMatrix4fv(g.u.uVP, false, vp);
      gl.uniform3fv(g.u.uCenter, this.gridCenter);
      gl.uniform1f(g.u.uRadius, this.gridRadius);
      gl.bindBuffer(gl.ARRAY_BUFFER, this.buffers.grid);
      gl.enableVertexAttribArray(g.aPos);
      gl.vertexAttribPointer(g.aPos, 3, gl.FLOAT, false, 28, 0);
      gl.enableVertexAttribArray(g.aColor);
      gl.vertexAttribPointer(g.aColor, 4, gl.FLOAT, false, 28, 12);
      gl.depthMask(false);
      gl.drawArrays(gl.LINES, 0, this.gridCount);
      gl.depthMask(true);
      gl.disableVertexAttribArray(g.aColor);
    }

    drawSolid(vp, eye) {
      const gl = this.gl, s = this.solid;
      const m = this.mesh;
      gl.useProgram(s.p);
      gl.uniformMatrix4fv(s.u.uVP, false, vp);
      gl.uniform3fv(s.u.uEye, eye);
      const colors = { wire: [0.12, 0.13, 0.15], slice: [0.42, 0.25, 0.04], solid: ORANGE };
      gl.uniform3fv(s.u.uColor, colors[this.mode]);
      const reveal = this.clip < 1 ? m.low[2] + m.size[2] * this.clip : m.low[2] + m.size[2] + 1;
      gl.uniform1f(s.u.uClip, this.mode === "slice" ? this.ghost : reveal);
      gl.bindBuffer(gl.ARRAY_BUFFER, this.buffers.vb);
      gl.enableVertexAttribArray(s.aPos);
      gl.vertexAttribPointer(s.aPos, 3, gl.FLOAT, false, 0, 0);
      gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, this.buffers.ib);
      gl.enable(gl.POLYGON_OFFSET_FILL);
      gl.polygonOffset(1, 1);
      gl.drawElements(gl.TRIANGLES, m.index.length, gl.UNSIGNED_SHORT, 0);
      gl.disable(gl.POLYGON_OFFSET_FILL);
    }

    drawWire(vp, eye) {
      const gl = this.gl, l = this.line;
      gl.useProgram(l.p);
      gl.uniformMatrix4fv(l.u.uVP, false, vp);
      gl.uniform3fv(l.u.uEye, eye);
      gl.uniform4f(l.u.uColor, ORANGE[0], ORANGE[1], ORANGE[2], 0.9);
      gl.bindBuffer(gl.ARRAY_BUFFER, this.buffers.vb);
      gl.enableVertexAttribArray(l.aPos);
      gl.vertexAttribPointer(l.aPos, 3, gl.FLOAT, false, 0, 0);
      gl.disableVertexAttribArray(l.aNormal);
      gl.vertexAttrib3f(l.aNormal, 0, 0, 1);
      gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, this.buffers.wire);
      gl.drawElements(gl.LINES, this.wireCount, gl.UNSIGNED_SHORT, 0);
    }

    drawSlice(vp, eye) {
      const gl = this.gl, l = this.line;
      const { offsets, count } = this.layers;
      const top = Math.round(this.layer * (count - 1));
      // The printed part up to this layer, dim, so flat faces do not look empty.
      this.ghost = this.mesh.low[2] + (top + 1) * LAYER_HEIGHT;
      this.drawSolid(vp, eye);
      gl.useProgram(l.p);
      gl.uniformMatrix4fv(l.u.uVP, false, vp);
      gl.uniform3fv(l.u.uEye, eye);
      gl.bindBuffer(gl.ARRAY_BUFFER, this.buffers.layers);
      gl.enableVertexAttribArray(l.aPos);
      gl.vertexAttribPointer(l.aPos, 3, gl.FLOAT, false, 24, 0);
      gl.enableVertexAttribArray(l.aNormal);
      gl.vertexAttribPointer(l.aNormal, 3, gl.FLOAT, false, 24, 12);
      gl.uniform4f(l.u.uColor, ORANGE[0], ORANGE[1], ORANGE[2], 1);
      gl.drawArrays(gl.LINES, 0, offsets[top]);
      // The current layer, drawn over the stack.
      gl.disable(gl.DEPTH_TEST);
      gl.uniform4f(l.u.uColor, 1.5, 1.5, 1.4, 1);
      gl.drawArrays(gl.LINES, offsets[top], offsets[top + 1] - offsets[top]);
      gl.enable(gl.DEPTH_TEST);
      gl.disableVertexAttribArray(l.aNormal);
    }
  }

  window.ScadView = { Viewer, loadMesh };
})();
