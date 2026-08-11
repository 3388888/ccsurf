// Geometry tests for the brush->polygon construction.
//
// The load-bearing one is the closure check: for any closed convex polyhedron the sum of
// (face area x face normal) over all faces is the zero vector. That catches a wrong plane
// sign, a bad winding order, an over-aggressive clip, or a missing side — the whole class of
// bugs that otherwise only shows up as a spot that isn't really there.
"use strict";

const fs = require("fs");
const bc = require("../lib/bspcollide");
const C = require("../lib/consts");

const MAPS = "C:/Users/w/Desktop/ClassicCounter/csgo/maps";

// build every face of a brush from its planes (not just the upward ones extract() keeps)
function brushFaces(planes) {
  const out = [];
  for (let i = 0; i < planes.length; i++) {
    const p = planes[i];
    let poly = bc.baseWinding(p[0], p[1], p[2], p[3]);
    if (!poly) continue;
    for (let j = 0; j < planes.length && poly.length; j++) {
      if (j === i) continue;
      const q = planes[j];
      poly = bc.clipToPlane(poly, q[0], q[1], q[2], q[3]);
    }
    if (poly.length >= 3) out.push({ n: [p[0], p[1], p[2]], poly });
  }
  return out;
}

function closureError(faces) {
  let sx = 0, sy = 0, sz = 0, total = 0;
  for (const f of faces) {
    const a = bc.polyArea(f.poly, f.n[0], f.n[1], f.n[2]);
    sx += a * f.n[0]; sy += a * f.n[1]; sz += a * f.n[2];
    total += a;
  }
  return total > 0 ? Math.hypot(sx, sy, sz) / total : Infinity;
}

module.exports = function run(t) {
  // ---- a 64-unit cube from six axis-aligned planes; interior is dot(n,p) <= d
  const cube = [
    [1, 0, 0, 32], [-1, 0, 0, 32], [0, 1, 0, 32],
    [0, -1, 0, 32], [0, 0, 1, 32], [0, 0, -1, 32],
  ];
  const cf = brushFaces(cube);
  t.eq(cf.length, 6, "cube has 6 faces");
  for (const f of cf) {
    t.eq(f.poly.length, 4, "cube face is a quad");
    const a = bc.polyArea(f.poly, f.n[0], f.n[1], f.n[2]);
    t.ok(Math.abs(a - 64 * 64) < 0.01, `cube face area is 4096 (got ${a.toFixed(2)})`);
  }
  t.ok(closureError(cf) < 1e-6, "cube is closed");

  // ---- ramps. The standable cutoff is normal.z >= 0.7, i.e. 45.573 degrees, so an exactly
  // 45-degree ramp is still a floor. Surf ramps are the ones steeper than that, and getting
  // this boundary wrong would misclassify every ramp in the game by one category.
  t.ok(Math.SQRT1_2 > C.STANDABLE_NORMAL_Z, "45 degrees (normal.z 0.7071) is standable");
  t.ok(Math.cos(46 * Math.PI / 180) < C.STANDABLE_NORMAL_Z, "46 degrees is surfable");
  t.ok(Math.abs(Math.acos(C.STANDABLE_NORMAL_Z) * 180 / Math.PI - 45.573) < 0.01,
    "the standable cutoff is 45.573 degrees");

  // a 60-degree wedge: clearly a surf ramp
  const ang = 60 * Math.PI / 180, sn = Math.sin(ang), cs = Math.cos(ang);
  const wedge = [
    [0, 0, -1, 0],        // floor at z=0
    [-1, 0, 0, 0],        // back wall at x=0
    [0, 1, 0, 32], [0, -1, 0, 32],
    [sn, 0, cs, 64 * sn], // the ramp itself
  ];
  const wf = brushFaces(wedge);
  t.eq(wf.length, 5, "wedge has 5 faces");
  t.ok(closureError(wf) < 1e-6, "wedge is closed");
  const slope = wf.find((f) => Math.abs(f.n[2] - cs) < 1e-6);
  t.ok(slope, "wedge slope face exists");
  t.ok(slope.n[2] > 0 && slope.n[2] < C.STANDABLE_NORMAL_Z, "a 60-degree slope is surfable, not standable");

  // ---- baseWinding orientation: the seed polygon's own normal must equal the plane normal.
  // The last entry is deliberately NOT unit length (|n| = 1.00035) — BSP normals are float32
  // and baseWinding has to survive that without throwing its far corners off the plane.
  const s = Math.SQRT1_2;
  for (const n of [[0, 0, 1], [0, 0, -1], [1, 0, 0], [s, s, 0], [0.267, 0.535, 0.802]]) {
    const w = bc.baseWinding(n[0], n[1], n[2], 100);
    t.ok(w, `baseWinding for [${n}]`);
    const [a, b, c] = w;
    const e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    const e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    const cr = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
    const l = Math.hypot(...cr);
    const dot = (cr[0] * n[0] + cr[1] * n[1] + cr[2] * n[2]) / l;
    t.ok(dot > 0.999, `winding normal matches plane normal for [${n}] (dot ${dot.toFixed(4)})`);
    // and every seed vertex must lie on the plane
    for (const p of w) {
      const off = p[0] * n[0] + p[1] * n[1] + p[2] * n[2] - 100;
      t.ok(Math.abs(off) < 0.05, `seed vertex on plane (off by ${off.toFixed(4)})`);
    }
  }

  // ---- real map: no polygon may escape to the seed size, and bounds must be plausible
  const real = `${MAPS}/de_dust2.bsp`;
  if (!fs.existsSync(real)) { console.log("  (skipping real-map checks: de_dust2.bsp not found)"); return; }

  const g = bc.extract(real);
  t.eq(g.stats.unbounded, 0, "no unbounded polygons survive extraction");
  t.ok(g.stats.brushesKept / g.stats.brushes > 0.9,
    `most brushes build successfully (${g.stats.brushesKept}/${g.stats.brushes})`);
  t.ok(g.bounds.maxZ - g.bounds.minZ < 20000, `map height is plausible (${(g.bounds.maxZ - g.bounds.minZ).toFixed(0)}u)`);
  t.ok(g.spawns.length > 0, "spawns found");
  t.eq(g.stats.propsScanned, false, "prop collision is reported as not scanned");

  // every kept face must be upward-facing and lie on its own plane
  let offPlane = 0, notUp = 0;
  for (const f of g.faces) {
    if (f.n[2] <= 0.01) notUp++;
    for (const p of f.poly) {
      if (Math.abs(p[0] * f.n[0] + p[1] * f.n[1] + p[2] * f.n[2] - f.d) > 0.1) { offPlane++; break; }
    }
  }
  t.eq(notUp, 0, "every kept face is upward-facing");
  t.eq(offPlane, 0, "every face vertex lies on its face plane");
};
