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
    // Spots are drawn as the surface itself: a flat quad on the patch footprint, depth-tested
    // so it is hidden by anything in front of it. A camera-facing billboard drawn through
    // walls told you a spot existed somewhere in that direction, which is not the same as
    // telling you where it is.
    spotProg = prog(`
      attribute vec3 aPos; attribute vec2 aUV; attribute vec3 aCol; attribute float aSel;
      uniform mat4 uMVP;
      varying vec2 vUV; varying vec3 vCol; varying float vSel;
      void main(){ vUV = aUV; vCol = aCol; vSel = aSel; gl_Position = uMVP * vec4(aPos,1.0); }`, `
      precision mediump float;
      varying vec2 vUV; varying vec3 vCol; varying float vSel;
      uniform float uPulse;
      void main(){
        // bright rim, softer fill — reads as an outlined patch rather than a blob
        vec2 e = min(vUV, 1.0 - vUV);
        float edge = min(e.x, e.y);
        float rim  = 1.0 - smoothstep(0.0, 0.16, edge);
        float fill = 0.22 + 0.30 * (1.0 - smoothstep(0.0, 0.5, edge));
        float a = clamp(fill + rim * 0.85, 0.0, 1.0) * (0.62 + 0.38 * vSel * uPulse);
        gl_FragColor = vec4(vCol * (0.85 + rim * 1.5 + vSel * 0.5), a);
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

  // Lift the quad just off the surface so it wins the depth test against the face it sits
  // on, without floating visibly above it.
  const SURF_LIFT = 0.6;
  /// A 1u sliver would be invisible edge-on, so tiny patches get padded to a minimum.
  const MIN_MARK = 6.0;

  function buildSpots() {
    const n = visible.length;
    if (!n) { spotVerts = 0; return; }
    const pos = new Float32Array(n*18), uv = new Float32Array(n*12);
    const col = new Float32Array(n*18), sel = new Float32Array(n*6);
    const quad = [[0,0],[1,0],[1,1],[0,0],[1,1],[0,1]];
    visible.forEach((s, i) => {
      const c = s.reachable ? (KIND_COL[s.kind] || [1,1,1]) : OOB_COL;
      let [x0, y0, x1, y1] = s.rect || [s.x-8, s.y-8, s.x+8, s.y+8];
      // pad so a hairline ledge is still clickable/visible from above
      if (x1 - x0 < MIN_MARK) { const m = (x0+x1)/2; x0 = m - MIN_MARK/2; x1 = m + MIN_MARK/2; }
      if (y1 - y0 < MIN_MARK) { const m = (y0+y1)/2; y0 = m - MIN_MARK/2; y1 = m + MIN_MARK/2; }
      const z = s.z + SURF_LIFT;
      for (let v = 0; v < 6; v++) {
        const u = quad[v];
        pos[i*18+v*3]   = u[0] ? x1 : x0;
        pos[i*18+v*3+1] = u[1] ? y1 : y0;
        pos[i*18+v*3+2] = z;
        uv[i*12+v*2] = u[0]; uv[i*12+v*2+1] = u[1];
        col[i*18+v*3] = c[0]; col[i*18+v*3+1] = c[1]; col[i*18+v*3+2] = c[2];
        sel[i*6+v] = (s === selected) ? 1 : 0;
      }
    });
    bCorner = buf(uv); bCentre = buf(pos); bCol = buf(col); bSize = buf(sel);
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
      gl.uniform1f(gl.getUniformLocation(spotProg,"uPulse"),
        0.65 + 0.35*Math.sin(now/260));
      const bind = (name,b,n) => { const a=gl.getAttribLocation(spotProg,name);
        gl.bindBuffer(gl.ARRAY_BUFFER,b); gl.enableVertexAttribArray(a); gl.vertexAttribPointer(a,n,gl.FLOAT,false,0,0); };
      bind("aUV",bCorner,2); bind("aPos",bCentre,3); bind("aCol",bCol,3); bind("aSel",bSize,1);
      gl.enable(gl.BLEND); gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
      // depth test ON: a marker behind a wall stays behind the wall
      gl.depthMask(false);
      gl.drawArrays(gl.TRIANGLES, 0, spotVerts);
      gl.depthMask(true); gl.disable(gl.BLEND);
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
