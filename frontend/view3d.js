// The 3D map view — matte geometry, glowing spots, fly camera.
//
// Shared by the app (a tab) and the standalone exported viewer, which inlines this file.
// View3D.load(canvas, geoBase64, spots) swaps in a map; call it again to change maps.
"use strict";

const View3D = (() => {
  let gl, mapProg, spotProg, cv;
  let bPos, bNrm, triCount = 0;
  let bCorner, bCentre, bCol, bSize, spotVerts = 0;
  let spots = [], visible = [], selected = null;
  let raf = null, bounds = null;
  const cam = { pos: [0, 0, 0], yaw: Math.PI / 2, pitch: -0.25, speed: 900 };
  const keys = {};
  let onSelect = null;

  const KIND_COL = { pixelsurf: [1.0, 0.66, 0.20], pixelwalk: [0.36, 0.60, 0.88],
                     surf: [0.44, 0.72, 0.36], ground: [0.6, 0.6, 0.6] };
  const OOB_COL = [0.92, 0.33, 0.29];

  function sh(type, src) {
    const s = gl.createShader(type);
    gl.shaderSource(s, src); gl.compileShader(s);
    if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s));
    return s;
  }
  function prog(vs, fs) {
    const p = gl.createProgram();
    gl.attachShader(p, sh(gl.VERTEX_SHADER, vs));
    gl.attachShader(p, sh(gl.FRAGMENT_SHADER, fs));
    gl.linkProgram(p);
    if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(p));
    return p;
  }
  function buf(data) {
    const b = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, b);
    gl.bufferData(gl.ARRAY_BUFFER, data, gl.STATIC_DRAW);
    return b;
  }

  function initGL(canvas) {
    if (gl) return true;
    cv = canvas;
    gl = cv.getContext("webgl", { antialias: true, depth: true });
    if (!gl) return false;
    // Matte: no textures, no materials. A key light, a little fill, a touch of up-light so
    // floors separate from walls, and fog so distance reads.
    mapProg = prog(`
      attribute vec3 aPos; attribute vec3 aNrm;
      uniform mat4 uMVP; varying vec3 vN; varying float vDepth;
      void main(){ vN = aNrm; vec4 p = uMVP * vec4(aPos,1.0); vDepth = p.w; gl_Position = p; }`, `
      precision mediump float;
      varying vec3 vN; varying float vDepth;
      void main(){
        vec3 n = normalize(vN);
        float key  = max(dot(n, normalize(vec3(0.35,0.5,0.85))), 0.0);
        float fill = max(dot(n, normalize(vec3(-0.5,-0.3,0.2))), 0.0);
        float l = 0.055 + 0.42*key + 0.11*fill + 0.16*max(n.z,0.0);
        float fog = clamp(vDepth / 9000.0, 0.0, 1.0);
        gl_FragColor = vec4(mix(vec3(l), vec3(0.043,0.047,0.055), fog*0.85), 1.0);
      }`);
    // Spots draw with the depth test off so one behind a wall still shows where to go.
    spotProg = prog(`
      attribute vec2 aCorner; attribute vec3 aCentre; attribute vec3 aCol; attribute float aSize;
      uniform mat4 uMVP; uniform vec3 uRight; uniform vec3 uUp;
      varying vec2 vUV; varying vec3 vCol;
      void main(){
        vUV = aCorner; vCol = aCol;
        vec3 w = aCentre + (uRight*aCorner.x + uUp*aCorner.y) * aSize;
        gl_Position = uMVP * vec4(w,1.0);
      }`, `
      precision mediump float;
      varying vec2 vUV; varying vec3 vCol;
      void main(){
        float d = length(vUV);
        if (d > 1.0) discard;
        float glow = pow(smoothstep(1.0, 0.0, d), 2.5);
        gl_FragColor = vec4(vCol * (0.55 + glow*1.9), glow*0.92);
      }`);
    wireInput();
    return true;
  }

  function wireInput() {
    addEventListener("keydown", (e) => {
      if (document.pointerLockElement !== cv) return;
      keys[e.code] = 1;
      if (e.code === "Escape") document.exitPointerLock();
      if (["Space", "KeyW", "KeyA", "KeyS", "KeyD"].includes(e.code)) e.preventDefault();
    });
    addEventListener("keyup", (e) => { keys[e.code] = 0; });
    // requestPointerLock rejects when the page is embedded in a sandboxed frame; swallow it
    // rather than leaving an unhandled rejection in the console on every click.
    cv.addEventListener("click", () => {
      const r = cv.requestPointerLock();
      if (r && typeof r.catch === "function") r.catch(() => {});
    });
    document.addEventListener("pointerlockchange",
      () => cv.classList.toggle("locked", document.pointerLockElement === cv));
    addEventListener("mousemove", (e) => {
      if (document.pointerLockElement !== cv) return;
      cam.yaw -= e.movementX * 0.0022;
      cam.pitch = Math.max(-1.55, Math.min(1.55, cam.pitch - e.movementY * 0.0022));
    });
  }

  function b64ToBytes(b64) {
    const bin = atob(b64);
    const out = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
    return out;
  }

  function uploadGeometry(geoB64) {
    const raw = b64ToBytes(geoB64);
    const posI16 = new Int16Array(raw.buffer, raw.byteOffset, raw.byteLength / 2);
    triCount = posI16.length / 9;
    const pos = new Float32Array(triCount * 9), nrm = new Float32Array(triCount * 9);
    let lo = [1e9, 1e9, 1e9], hi = [-1e9, -1e9, -1e9];
    for (let t = 0; t < triCount; t++) {
      const o = t * 9;
      for (let k = 0; k < 9; k++) pos[o + k] = posI16[o + k];
      // flat normal per triangle, computed here so the shader stays WebGL1-simple
      const e1 = [pos[o+3]-pos[o], pos[o+4]-pos[o+1], pos[o+5]-pos[o+2]];
      const e2 = [pos[o+6]-pos[o], pos[o+7]-pos[o+1], pos[o+8]-pos[o+2]];
      let n = [e1[1]*e2[2]-e1[2]*e2[1], e1[2]*e2[0]-e1[0]*e2[2], e1[0]*e2[1]-e1[1]*e2[0]];
      const l = Math.hypot(n[0], n[1], n[2]) || 1;
      for (let v = 0; v < 3; v++) for (let k = 0; k < 3; k++) nrm[o+v*3+k] = n[k]/l;
      for (let v = 0; v < 3; v++) for (let k = 0; k < 3; k++) {
        const val = pos[o+v*3+k];
        if (val < lo[k]) lo[k] = val;
        if (val > hi[k]) hi[k] = val;
      }
    }
    bPos = buf(pos); bNrm = buf(nrm);
    bounds = { lo, hi };
  }

  function buildSpots() {
    const n = visible.length;
    if (!n) { spotVerts = 0; return; }
    const corner = new Float32Array(n*12), centre = new Float32Array(n*18);
    const col = new Float32Array(n*18), size = new Float32Array(n*6);
    const quad = [[-1,-1],[1,-1],[1,1],[-1,-1],[1,1],[-1,1]];
    visible.forEach((s, i) => {
      const c = s.reachable ? (KIND_COL[s.kind] || [1,1,1]) : OOB_COL;
      for (let v = 0; v < 6; v++) {
        corner[i*12+v*2] = quad[v][0]; corner[i*12+v*2+1] = quad[v][1];
        centre[i*18+v*3] = s.x; centre[i*18+v*3+1] = s.y; centre[i*18+v*3+2] = s.z + 6;
        col[i*18+v*3] = c[0]; col[i*18+v*3+1] = c[1]; col[i*18+v*3+2] = c[2];
        size[i*6+v] = (s === selected) ? 34 : 20;
      }
    });
    bCorner = buf(corner); bCentre = buf(centre); bCol = buf(col); bSize = buf(size);
    spotVerts = n*6;
  }

  const fwd = () => { const cp = Math.cos(cam.pitch);
    return [Math.cos(cam.yaw)*cp, Math.sin(cam.yaw)*cp, Math.sin(cam.pitch)]; };
  function mul(a,b){ const o=new Float32Array(16);
    for(let i=0;i<4;i++)for(let j=0;j<4;j++){let s=0;for(let k=0;k<4;k++)s+=a[k*4+j]*b[i*4+k];o[i*4+j]=s;} return o; }
  function persp(f,asp,n,fa){ const t=1/Math.tan(f/2);
    return new Float32Array([t/asp,0,0,0, 0,t,0,0, 0,0,(fa+n)/(n-fa),-1, 0,0,2*fa*n/(n-fa),0]); }
  function lookAt(e,c,u){
    const f=[c[0]-e[0],c[1]-e[1],c[2]-e[2]]; let l=Math.hypot(...f); f[0]/=l;f[1]/=l;f[2]/=l;
    let s=[f[1]*u[2]-f[2]*u[1], f[2]*u[0]-f[0]*u[2], f[0]*u[1]-f[1]*u[0]];
    l=Math.hypot(...s)||1; s=[s[0]/l,s[1]/l,s[2]/l];
    const v=[s[1]*f[2]-s[2]*f[1], s[2]*f[0]-s[0]*f[2], s[0]*f[1]-s[1]*f[0]];
    return new Float32Array([s[0],v[0],-f[0],0, s[1],v[1],-f[1],0, s[2],v[2],-f[2],0,
      -(s[0]*e[0]+s[1]*e[1]+s[2]*e[2]), -(v[0]*e[0]+v[1]*e[1]+v[2]*e[2]), f[0]*e[0]+f[1]*e[1]+f[2]*e[2], 1]);
  }

  let flyTo = null, last = 0;
  function frame(now) {
    raf = requestAnimationFrame(frame);
    const dt = Math.min((now - last)/1000, 0.1); last = now;
    if (!bPos) return;

    const d = fwd();
    const right = [Math.cos(cam.yaw-Math.PI/2), Math.sin(cam.yaw-Math.PI/2), 0];
    const sp = cam.speed * ((keys.ShiftLeft||keys.ShiftRight) ? 3.4 : 1) * dt;
    const step = (v,m) => { cam.pos[0]+=v[0]*m; cam.pos[1]+=v[1]*m; cam.pos[2]+=v[2]*m; };
    if (keys.KeyW) step(d, sp);
    if (keys.KeyS) step(d, -sp);
    if (keys.KeyD) step(right, sp);
    if (keys.KeyA) step(right, -sp);
    if (keys.Space) cam.pos[2] += sp;
    if (keys.ControlLeft||keys.ControlRight) cam.pos[2] -= sp;
    if (flyTo) {
      flyTo.t = Math.min(1, flyTo.t + dt*2.6);
      for (let i=0;i<3;i++) cam.pos[i] += (flyTo.pos[i]-cam.pos[i]) * 0.35;
      if (flyTo.t >= 1) flyTo = null;
    }

    const w = cv.clientWidth, h = cv.clientHeight;
    if (!w || !h) return;
    const dpr = Math.min(devicePixelRatio||1, 2);
    if (cv.width !== Math.round(w*dpr) || cv.height !== Math.round(h*dpr)) {
      cv.width = Math.round(w*dpr); cv.height = Math.round(h*dpr);
    }
    gl.viewport(0,0,cv.width,cv.height);
    gl.clearColor(0.043,0.047,0.055,1);
    gl.enable(gl.DEPTH_TEST);
    gl.clear(gl.COLOR_BUFFER_BIT|gl.DEPTH_BUFFER_BIT);

    const tgt = [cam.pos[0]+d[0], cam.pos[1]+d[1], cam.pos[2]+d[2]];
    const mvp = mul(persp(1.15, w/h, 8, 40000), lookAt(cam.pos, tgt, [0,0,1]));

    gl.useProgram(mapProg);
    gl.uniformMatrix4fv(gl.getUniformLocation(mapProg,"uMVP"), false, mvp);
    const ap = gl.getAttribLocation(mapProg,"aPos"), an = gl.getAttribLocation(mapProg,"aNrm");
    gl.bindBuffer(gl.ARRAY_BUFFER,bPos); gl.enableVertexAttribArray(ap); gl.vertexAttribPointer(ap,3,gl.FLOAT,false,0,0);
    gl.bindBuffer(gl.ARRAY_BUFFER,bNrm); gl.enableVertexAttribArray(an); gl.vertexAttribPointer(an,3,gl.FLOAT,false,0,0);
    gl.drawArrays(gl.TRIANGLES, 0, triCount*3);

    if (spotVerts) {
      gl.useProgram(spotProg);
      gl.uniformMatrix4fv(gl.getUniformLocation(spotProg,"uMVP"), false, mvp);
      gl.uniform3fv(gl.getUniformLocation(spotProg,"uRight"), right);
      gl.uniform3fv(gl.getUniformLocation(spotProg,"uUp"),
        [-d[0]*Math.sin(cam.pitch), -d[1]*Math.sin(cam.pitch), Math.cos(cam.pitch)]);
      const bind = (name,b,n) => { const a=gl.getAttribLocation(spotProg,name);
        gl.bindBuffer(gl.ARRAY_BUFFER,b); gl.enableVertexAttribArray(a); gl.vertexAttribPointer(a,n,gl.FLOAT,false,0,0); };
      bind("aCorner",bCorner,2); bind("aCentre",bCentre,3); bind("aCol",bCol,3); bind("aSize",bSize,1);
      gl.enable(gl.BLEND); gl.blendFunc(gl.SRC_ALPHA, gl.ONE);
      gl.depthMask(false); gl.disable(gl.DEPTH_TEST);
      gl.drawArrays(gl.TRIANGLES, 0, spotVerts);
      gl.depthMask(true); gl.disable(gl.BLEND); gl.enable(gl.DEPTH_TEST);
    }

    if (View3D.onFrame) View3D.onFrame(cam);
  }

  return {
    onFrame: null,
    /** Swap in a map. Safe to call repeatedly. */
    load(canvas, geoB64, spotList, selectCb) {
      if (!initGL(canvas)) return false;
      onSelect = selectCb || null;
      uploadGeometry(geoB64);
      spots = spotList || [];
      visible = spots.slice();
      selected = null;
      buildSpots();
      const c = [(bounds.lo[0]+bounds.hi[0])/2, (bounds.lo[1]+bounds.hi[1])/2, (bounds.lo[2]+bounds.hi[2])/2];
      cam.pos = [c[0], c[1]-1200, c[2]+500];
      cam.yaw = Math.PI/2; cam.pitch = -0.25;
      if (!raf) { last = performance.now(); raf = requestAnimationFrame(frame); }
      return true;
    },
    setFilter(pred) { visible = pred ? spots.filter(pred) : spots.slice(); buildSpots(); },
    select(s) { selected = s; buildSpots(); if (s) this.focus(s); if (onSelect) onSelect(s); },
    focus(s) {
      const d = fwd();
      flyTo = { pos: [s.x - d[0]*260, s.y - d[1]*260, s.z + 110], t: 0 };
    },
    get camera() { return cam; },
    get triCount() { return triCount; },
    get visible() { return visible; },
  };
})();
if (typeof module !== "undefined") module.exports = View3D;
