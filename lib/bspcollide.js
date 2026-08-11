// Collision geometry out of a Source-1 .bsp, at the precision a pixel surf actually needs.
//
// This is deliberately NOT demo-reader/bspgeo.js. That module reads the FACE lumps and
// quantizes every coordinate to int16 for drawing — both disqualifying here:
//
//   * a pixel surf is a sub-unit feature; rounding to whole units erases the entire signal
//   * the engine collides the player hull against BRUSH PLANES, not rendered faces, and the
//     perch you are looking for is very often exactly where the two disagree
//
// So we rebuild each brush as a convex polyhedron from its own side planes (the standard
// clip-a-huge-quad-against-every-other-plane construction) and keep full float coordinates.
//
// WHAT IS DEPLIBERATELY DROPPED
//
// Only UPWARD-facing polygons are kept (normal.z > 0). You cannot stand or surf on a wall or
// a ceiling, and keeping them would multiply the output several times over for no gain. The
// things that still need to know about walls — headroom and reachability — get the compact
// per-brush AABB list in `solids` instead, which is enough for those queries and costs
// almost nothing.
//
// NOT HANDLED: static prop collision (.phy inside the VPKs). Prop-mounted spots are
// invisible to this module. `stats.propsScanned` is always false so callers can say so out
// loud rather than implying a complete scan.
"use strict";

const fs = require("fs");
const path = require("path");
const zlib = require("zlib");
const C = require("./consts");

const LUMP = {
  ENTITIES: 0, PLANES: 1, VERTEXES: 3, TEXINFO: 6, FACES: 7, EDGES: 12, SURFEDGES: 13,
  MODELS: 14, BRUSHES: 18, BRUSHSIDES: 19, DISPINFO: 26, DISP_VERTS: 33,
};

const PLANE_SIZE = 20;      // normal[3] float, dist float, type int
const BRUSH_SIZE = 12;      // firstside int, numsides int, contents int
// dbrushside_t: planenum ushort, texinfo short, dispinfo short, then TWO bytes.
// The old Source layout ended in a single `short bevel`; CS:GO splits that into
// `byte bevel; byte thin;`. Reading it as a short makes every thin side (thin=1, i.e. 256)
// look like a bevel — on de_dust2 that silently discarded 16133 of 50124 sides and left
// brushes with too few planes to close, which in turn leaked unclipped seed polygons.
const BRUSHSIDE_SIZE = 8;
const SIDE_BEVEL_OFS = 6;
const MODEL_SIZE = 48;
const DISPINFO_SIZE = 176;
const FACE_SIZE = 56;

// Source's world extent. The seed polygon has to comfortably exceed it.
const MAX_COORD = 32768;

// ---------------------------------------------------------------- lump access

function openBsp(file) {
  const isBz2 = /\.bz2$/i.test(file);
  let fd = null, mem = null;
  if (isBz2) mem = require("seek-bzip").decode(fs.readFileSync(file));
  else fd = fs.openSync(file, "r");

  const head = Buffer.alloc(1036);
  if (mem) mem.copy(head, 0, 0, Math.min(1036, mem.length));
  else fs.readSync(fd, head, 0, 1036, 0);

  const magic = head.toString("latin1", 0, 4);
  if (magic !== "VBSP") {
    if (fd != null) fs.closeSync(fd);
    throw new Error(magic === "rBSP" ? "Respawn bsp variant not supported" : `not a VBSP map (magic "${magic}")`);
  }
  const version = head.readInt32LE(4);
  const lumps = [];
  for (let i = 0; i < 64; i++) {
    const o = 8 + i * 16;
    lumps.push({ ofs: head.readInt32LE(o), len: head.readInt32LE(o + 4), ver: head.readInt32LE(o + 8) });
  }
  return {
    version, lumps,
    read(i) {
      const l = lumps[i];
      if (!l || l.len <= 0) return Buffer.alloc(0);
      // fresh alloc so byteOffset is 0 — typed-array views need natural alignment, which
      // subarray()/pooled buffers do not guarantee
      const buf = Buffer.alloc(l.len);
      if (mem) mem.copy(buf, 0, l.ofs, l.ofs + l.len);
      else fs.readSync(fd, buf, 0, l.len, l.ofs);
      if (buf.length >= 4 && buf.toString("latin1", 0, 4) === "LZMA") throw new Error("lump-compressed (LZMA) bsp not supported");
      return buf;
    },
    close() { if (fd != null) try { fs.closeSync(fd); } catch {} },
  };
}

function parseEntities(buf) {
  const txt = buf.toString("latin1");
  const out = [];
  const blockRe = /\{([^{}]*)\}/g;
  let m;
  while ((m = blockRe.exec(txt))) {
    const ent = {};
    const kvRe = /"([^"]*)"\s*"([^"]*)"/g;
    let kv;
    while ((kv = kvRe.exec(m[1]))) ent[kv[1].toLowerCase()] = kv[2];
    out.push(ent);
  }
  return out;
}

function vec3(s) {
  if (!s) return null;
  const p = String(s).trim().split(/\s+/).map(Number);
  return p.length >= 3 && p.every(Number.isFinite) ? [p[0], p[1], p[2]] : null;
}

// ---------------------------------------------------------------- polygon from planes

// A quad on the plane, large enough to contain any real brush face before clipping.
// Quake's BaseWindingForPlane, wound so the polygon's own normal equals n.
//
// The normal is re-normalised first and it matters: the seed extends MAX_COORD units from
// the origin, so an input normal off unit length by 1 part in 10^4 leaves the projected
// basis non-perpendicular and throws the far corners ~6 units off the plane. BSP normals are
// stored as float32 and are not exactly unit.
function baseWinding(nx, ny, nz, d) {
  const nl = Math.hypot(nx, ny, nz);
  if (nl < 1e-9) return null;
  nx /= nl; ny /= nl; nz /= nl; d /= nl;

  const ax = Math.abs(nx), ay = Math.abs(ny), az = Math.abs(nz);
  let ux = 0, uy = 0, uz = 0;
  if (az >= ax && az >= ay) ux = 1; else uz = 1;   // pick an axis not parallel to n
  // project the helper axis onto the plane and normalise
  const dot = ux * nx + uy * ny + uz * nz;
  ux -= nx * dot; uy -= ny * dot; uz -= nz * dot;
  const ul = Math.hypot(ux, uy, uz);
  if (ul < 1e-9) return null;
  ux /= ul; uy /= ul; uz /= ul;
  // right = n x up, which gives (up, right, n) a right-handed orientation so the vertex
  // order below winds counter-clockwise when viewed from the +n side
  const rx = ny * uz - nz * uy, ry = nz * ux - nx * uz, rz = nx * uy - ny * ux;
  const ox = nx * d, oy = ny * d, oz = nz * d;
  const U = MAX_COORD, R = MAX_COORD;
  return [
    [ox - rx * R + ux * U, oy - ry * R + uy * U, oz - rz * R + uz * U],
    [ox + rx * R + ux * U, oy + ry * R + uy * U, oz + rz * R + uz * U],
    [ox + rx * R - ux * U, oy + ry * R - uy * U, oz + rz * R - uz * U],
    [ox - rx * R - ux * U, oy - ry * R - uy * U, oz - rz * R - uz * U],
  ];
}

// Sutherland-Hodgman: keep the part of `poly` inside the half-space dot(n,p) <= d.
const CLIP_EPS = 0.01;
function clipToPlane(poly, nx, ny, nz, d) {
  const n = poly.length;
  if (!n) return poly;
  const dist = new Array(n);
  let anyFront = false, anyBack = false;
  for (let i = 0; i < n; i++) {
    const p = poly[i];
    const dd = p[0] * nx + p[1] * ny + p[2] * nz - d;
    dist[i] = dd;
    if (dd > CLIP_EPS) anyFront = true;
    else if (dd < -CLIP_EPS) anyBack = true;
  }
  if (!anyFront) return poly;      // entirely inside
  if (!anyBack) return [];         // entirely outside

  const out = [];
  for (let i = 0; i < n; i++) {
    const p = poly[i], q = poly[(i + 1) % n];
    const dp = dist[i], dq = dist[(i + 1) % n];
    if (dp <= CLIP_EPS) out.push(p);
    if ((dp > CLIP_EPS && dq < -CLIP_EPS) || (dp < -CLIP_EPS && dq > CLIP_EPS)) {
      const t = dp / (dp - dq);
      out.push([p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t, p[2] + (q[2] - p[2]) * t]);
    }
  }
  return out.length >= 3 ? out : [];
}

function polyArea(poly, nx, ny, nz) {
  // projected area via the shoelace formula in 3D
  let cx = 0, cy = 0, cz = 0;
  for (let i = 1; i + 1 < poly.length; i++) {
    const a = poly[0], b = poly[i], c = poly[i + 1];
    const e1x = b[0] - a[0], e1y = b[1] - a[1], e1z = b[2] - a[2];
    const e2x = c[0] - a[0], e2y = c[1] - a[1], e2z = c[2] - a[2];
    cx += e1y * e2z - e1z * e2y; cy += e1z * e2x - e1x * e2z; cz += e1x * e2y - e1y * e2x;
  }
  return Math.abs(cx * nx + cy * ny + cz * nz) / 2;
}

// ---------------------------------------------------------------- extraction

/**
 * @param {string} file  path to a .bsp or .bsp.bz2
 * @returns {{name,version,faces,solids,spawns,bounds,play,stats}}
 *   faces  — upward-facing collision polygons {n:[x,y,z], d, poly, contents, kind}
 *   solids — AABB per player-solid brush, for headroom/occlusion queries
 */
function extract(file, opts = {}) {
  const minArea = opts.minArea != null ? opts.minArea : 0.25;  // drop slivers below 0.5x0.5u
  const bsp = openBsp(file);
  try {
    const planesBuf = bsp.read(LUMP.PLANES);
    const brushesBuf = bsp.read(LUMP.BRUSHES);
    const sidesBuf = bsp.read(LUMP.BRUSHSIDES);
    const modelsBuf = bsp.read(LUMP.MODELS);
    const ents = parseEntities(bsp.read(LUMP.ENTITIES));

    if (!planesBuf.length || !brushesBuf.length) throw new Error("no brush lumps");

    const nPlanes = (planesBuf.length / PLANE_SIZE) | 0;
    const nBrush = (brushesBuf.length / BRUSH_SIZE) | 0;
    const nSides = (sidesBuf.length / BRUSHSIDE_SIZE) | 0;
    const nModels = (modelsBuf.length / MODEL_SIZE) | 0;

    // plane lump as flat typed arrays — this is the hot data
    const pnx = new Float32Array(nPlanes), pny = new Float32Array(nPlanes),
      pnz = new Float32Array(nPlanes), pd = new Float32Array(nPlanes);
    for (let i = 0; i < nPlanes; i++) {
      const o = i * PLANE_SIZE;
      pnx[i] = planesBuf.readFloatLE(o);
      pny[i] = planesBuf.readFloatLE(o + 4);
      pnz[i] = planesBuf.readFloatLE(o + 8);
      pd[i] = planesBuf.readFloatLE(o + 12);
    }

    // KNOWN GAP: brush entities with a non-zero origin (doors, elevators, moving platforms)
    // are emitted at their compiled position, not their in-game one. bspgeo.js:187-208 can
    // correct this because dmodel_t indexes FACES, which is what it reads; brushes are tied
    // to models only through the BSP tree (nodes -> leaves -> leafbrushes), which is a much
    // bigger walk. Counted here so the number is visible rather than assumed to be zero.
    let movingBrushEnts = 0;
    for (const e of ents) {
      if (!(e.model && e.model[0] === "*")) continue;
      const idx = parseInt(e.model.slice(1), 10);
      const o = vec3(e.origin);
      if (Number.isFinite(idx) && idx > 0 && idx < nModels && o && (o[0] || o[1] || o[2])) movingBrushEnts++;
    }

    // spawns, for the reachability flood fill later
    const spawns = [];
    for (const e of ents) {
      if (!/^info_(player_(terrorist|counterterrorist|start|deathmatch)|deathmatch_spawn)$/.test(e.classname || "")) continue;
      const o = vec3(e.origin);
      if (o) spawns.push(o);
    }

    const faces = [];
    const solids = [];
    let minX = Infinity, minY = Infinity, minZ = Infinity, maxX = -Infinity, maxY = -Infinity, maxZ = -Infinity;
    // brushesSolid is the denominator that means anything: the total brush count also
    // includes triggers, areaportals, nodraw and water, which we never intended to build.
    let degenerate = 0, bevelSkipped = 0, brushesKept = 0, brushesSolid = 0, unbounded = 0, bevelFallback = 0;

    // scratch reused per brush; grown on demand rather than capping (and silently dropping)
    let sideP = new Int32Array(64);

    for (let b = 0; b < nBrush; b++) {
      const bo = b * BRUSH_SIZE;
      const contents = brushesBuf.readInt32LE(bo + 8);
      if (!(contents & C.CONTENTS_PLAYER_SOLID)) continue;
      brushesSolid++;
      const firstSide = brushesBuf.readInt32LE(bo);
      const numSides = brushesBuf.readInt32LE(bo + 4);
      if (numSides < 4) continue;
      if (numSides > sideP.length) sideP = new Int32Array(numSides);

      // Collect the real side planes. Bevel sides are axis-aligned padding the compiler adds
      // for the expanded-hull sweep; they are not surfaces of the brush, and including them
      // shaves the corners off every polygon we build.
      let ns = 0;
      for (let s = firstSide; s < firstSide + numSides; s++) {
        if (s < 0 || s >= nSides) continue;
        const so = s * BRUSHSIDE_SIZE;
        if (sidesBuf.readUInt8(so + SIDE_BEVEL_OFS)) { bevelSkipped++; continue; }
        const pn = sidesBuf.readUInt16LE(so);
        if (pn < nPlanes) sideP[ns++] = pn;
      }
      // Fewer than four real planes cannot bound a volume. Rather than drop the brush, fall
      // back to every side including bevels — bevel planes only touch the brush, so they
      // clip nothing away that the real planes wouldn't.
      if (ns < 4) {
        ns = 0;
        for (let s = firstSide; s < firstSide + numSides; s++) {
          if (s < 0 || s >= nSides) continue;
          const pn = sidesBuf.readUInt16LE(s * BRUSHSIDE_SIZE);
          if (pn < nPlanes) sideP[ns++] = pn;
        }
        if (ns < 4) continue;
        bevelFallback++;
      }
      brushesKept++;

      let bMinX = Infinity, bMinY = Infinity, bMinZ = Infinity, bMaxX = -Infinity, bMaxY = -Infinity, bMaxZ = -Infinity;

      for (let i = 0; i < ns; i++) {
        const pi = sideP[i];
        const nx = pnx[pi], ny = pny[pi], nz = pnz[pi], d = pd[pi];
        let poly = baseWinding(nx, ny, nz, d);
        if (!poly) { degenerate++; continue; }
        for (let j = 0; j < ns && poly.length; j++) {
          if (j === i) continue;
          const pj = sideP[j];
          poly = clipToPlane(poly, pnx[pj], pny[pj], pnz[pj], pd[pj]);
        }
        if (poly.length < 3) continue;

        // A brush whose planes don't actually close leaves part of the seed winding intact.
        // Such a polygon is meaningless and would wreck the map bounds, so drop it and count
        // it — silently keeping it is how you end up with a map 65000 units tall.
        let esc = false;
        for (const p of poly) {
          if (Math.abs(p[0]) >= MAX_COORD - 1 || Math.abs(p[1]) >= MAX_COORD - 1 || Math.abs(p[2]) >= MAX_COORD - 1) { esc = true; break; }
        }
        if (esc) { unbounded++; continue; }

        for (const p of poly) {
          if (p[0] < bMinX) bMinX = p[0]; if (p[1] < bMinY) bMinY = p[1]; if (p[2] < bMinZ) bMinZ = p[2];
          if (p[0] > bMaxX) bMaxX = p[0]; if (p[1] > bMaxY) bMaxY = p[1]; if (p[2] > bMaxZ) bMaxZ = p[2];
        }

        // only upward-facing polygons can be stood or surfed on
        if (nz <= 0.01) continue;
        if (polyArea(poly, nx, ny, nz) < minArea) { degenerate++; continue; }
        faces.push({ n: [nx, ny, nz], d, poly, contents, kind: "brush" });
      }

      if (bMinX < Infinity) {
        solids.push({ minX: bMinX, minY: bMinY, minZ: bMinZ, maxX: bMaxX, maxY: bMaxY, maxZ: bMaxZ, contents });
        if (bMinX < minX) minX = bMinX; if (bMinY < minY) minY = bMinY; if (bMinZ < minZ) minZ = bMinZ;
        if (bMaxX > maxX) maxX = bMaxX; if (bMaxY > maxY) maxY = bMaxY; if (bMaxZ > maxZ) maxZ = bMaxZ;
      }
    }

    // ---- displacements: real ground on most maps, and the big surf ramps
    const dispFaces = extractDisplacements(bsp, minArea);
    for (const f of dispFaces) faces.push(f);

    let play = null;
    if (spawns.length) {
      let a = Infinity, bb = Infinity, c = Infinity, d2 = -Infinity, e2 = -Infinity, f2 = -Infinity;
      for (const s of spawns) {
        a = Math.min(a, s[0]); bb = Math.min(bb, s[1]); c = Math.min(c, s[2]);
        d2 = Math.max(d2, s[0]); e2 = Math.max(e2, s[1]); f2 = Math.max(f2, s[2]);
      }
      play = { minX: a, minY: bb, minZ: c, maxX: d2, maxY: e2, maxZ: f2 };
    }

    return {
      name: path.basename(file).replace(/\.bsp(\.bz2)?$/i, "").toLowerCase(),
      version: bsp.version,
      faces, solids, spawns,
      bounds: { minX, minY, minZ, maxX, maxY, maxZ },
      play,
      stats: {
        planes: nPlanes, brushes: nBrush, brushesKept, sides: nSides,
        upFaces: faces.length, dispFaces: dispFaces.length,
        degenerate, bevelSkipped, bevelFallback, unbounded, movingBrushEnts,
        spawns: spawns.length,
        propsScanned: false,   // .phy collision is not implemented — say so, don't imply it
      },
    };
  } finally {
    bsp.close();
  }
}

// Displacement surfaces at FULL tessellation. bspgeo caps this at 8x8 cells because that is
// plenty for drawing; for collision the cap would smooth away exactly the small lips that
// make a surf spot, so every quad is emitted.
function extractDisplacements(bsp, minArea) {
  const out = [];
  const dispInfoBuf = bsp.read(LUMP.DISPINFO);
  if (!dispInfoBuf.length) return out;
  const dispVertBuf = bsp.read(LUMP.DISP_VERTS);
  const vertsBuf = bsp.read(LUMP.VERTEXES);
  const edgesBuf = bsp.read(LUMP.EDGES);
  const surfBuf = bsp.read(LUMP.SURFEDGES);
  let facesBuf = bsp.read(LUMP.FACES);
  if (!facesBuf.length) return out;

  const nDisp = (dispInfoBuf.length / DISPINFO_SIZE) | 0;
  const nVerts = (vertsBuf.length / 12) | 0;
  const nEdges = (edgesBuf.length / 4) | 0;
  const nSurf = (surfBuf.length / 4) | 0;
  const nFaces = (facesBuf.length / FACE_SIZE) | 0;

  const vx = new Float32Array(vertsBuf.buffer, vertsBuf.byteOffset, nVerts * 3);
  const ed = new Uint16Array(edgesBuf.buffer, edgesBuf.byteOffset, nEdges * 2);
  const se = new Int32Array(surfBuf.buffer, surfBuf.byteOffset, nSurf);
  const dv = new Float32Array(dispVertBuf.buffer, dispVertBuf.byteOffset, (dispVertBuf.length / 4) | 0);

  const lerp = (a, b, t) => [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];

  for (let f = 0; f < nFaces; f++) {
    const fo = f * FACE_SIZE;
    const dispinfo = facesBuf.readInt16LE(fo + 12);
    if (dispinfo < 0 || dispinfo >= nDisp) continue;
    const numEdges = facesBuf.readInt16LE(fo + 8);
    if (numEdges !== 4) continue;
    const firstEdge = facesBuf.readInt32LE(fo + 4);

    const corners = [];
    let bad = false;
    for (let i = 0; i < 4; i++) {
      const si = firstEdge + i;
      if (si < 0 || si >= nSurf) { bad = true; break; }
      const s = se[si];
      const ei = s >= 0 ? s : -s;
      if (ei < 0 || ei >= nEdges) { bad = true; break; }
      const vi = s >= 0 ? ed[ei * 2] : ed[ei * 2 + 1];
      if (vi >= nVerts) { bad = true; break; }
      corners.push([vx[vi * 3], vx[vi * 3 + 1], vx[vi * 3 + 2]]);
    }
    if (bad) continue;

    const base = dispinfo * DISPINFO_SIZE;
    const sx = dispInfoBuf.readFloatLE(base), sy = dispInfoBuf.readFloatLE(base + 4), sz = dispInfoBuf.readFloatLE(base + 8);
    const vStart = dispInfoBuf.readInt32LE(base + 12);
    const power = dispInfoBuf.readInt32LE(base + 20);
    if (power < 2 || power > 4) continue;
    const size = (1 << power) + 1;

    // the corner nearest startPosition is grid (0,0)
    let best = 0, bestD = Infinity;
    for (let i = 0; i < 4; i++) {
      const dx = corners[i][0] - sx, dy = corners[i][1] - sy, dz = corners[i][2] - sz;
      const dd = dx * dx + dy * dy + dz * dz;
      if (dd < bestD) { bestD = dd; best = i; }
    }
    const c = [];
    for (let i = 0; i < 4; i++) c.push(corners[(best + i) % 4]);

    const grid = [];
    for (let i = 0; i < size; i++) {
      const ti = i / (size - 1);
      const l = lerp(c[0], c[1], ti), r = lerp(c[3], c[2], ti);
      const row = [];
      for (let j = 0; j < size; j++) {
        const p = lerp(l, r, j / (size - 1));
        const vi = vStart + i * size + j;
        const o = vi * 5;   // ddispvert_t: vec3 vec, float dist, float alpha
        if (o + 4 < dv.length) {
          const dist = dv[o + 3];
          p[0] += dv[o] * dist; p[1] += dv[o + 1] * dist; p[2] += dv[o + 2] * dist;
        }
        row.push(p);
      }
      grid.push(row);
    }

    for (let i = 0; i + 1 < size; i++) {
      for (let j = 0; j + 1 < size; j++) {
        const a = grid[i][j], b2 = grid[i][j + 1], cc = grid[i + 1][j + 1], d2 = grid[i + 1][j];
        for (const tri of [[a, b2, cc], [a, cc, d2]]) {
          const [p, q, r] = tri;
          const e1x = q[0] - p[0], e1y = q[1] - p[1], e1z = q[2] - p[2];
          const e2x = r[0] - p[0], e2y = r[1] - p[1], e2z = r[2] - p[2];
          let nx = e1y * e2z - e1z * e2y, ny = e1z * e2x - e1x * e2z, nz = e1x * e2y - e1y * e2x;
          const len = Math.hypot(nx, ny, nz);
          if (len < 1e-6) continue;
          nx /= len; ny /= len; nz /= len;
          if (nz <= 0.01) continue;               // upward-facing only, same rule as brushes
          if (len / 2 < minArea) continue;
          out.push({ n: [nx, ny, nz], d: p[0] * nx + p[1] * ny + p[2] * nz,
            poly: tri, contents: C.CONTENTS_SOLID, kind: "disp" });
        }
      }
    }
  }
  return out;
}

// ---------------------------------------------------------------- cache

function cacheFile(dir, mapName) { return path.join(dir, mapName + ".collide.json.gz"); }

// Cheap on the second run: the extraction is seconds on a big map, the cache is milliseconds.
// Same shape as demo-reader/pixelsurf.js loadMeta().
function load(mapName, { cacheDir, dirs, findBsp } = {}) {
  const name = String(mapName || "").toLowerCase().replace(/[^\w.-]/g, "");
  if (!name) return null;
  const cached = cacheDir ? cacheFile(cacheDir, name) : null;
  if (cached) {
    try {
      if (fs.existsSync(cached)) return JSON.parse(zlib.gunzipSync(fs.readFileSync(cached)).toString("utf8"));
    } catch {}
  }
  const bsp = findBsp ? findBsp(name, dirs) : null;
  if (!bsp) return null;
  const geo = extract(bsp);
  if (cached) {
    try {
      fs.mkdirSync(cacheDir, { recursive: true });
      const tmp = cached + ".tmp";
      fs.writeFileSync(tmp, zlib.gzipSync(Buffer.from(JSON.stringify(geo), "utf8")));
      fs.renameSync(tmp, cached);
    } catch {}
  }
  return geo;
}

module.exports = { extract, load, openBsp, parseEntities, baseWinding, clipToPlane, polyArea, LUMP };
