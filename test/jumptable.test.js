// Validates the derived jump table against HackerPide/Pixurf's in-game measurements.
//
// The reference values below are used only as a test oracle — nothing in lib/ reads them,
// and the shipped table is derived from sv_gravity/sv_jump_impulse in lib/jumptable.js.
// They are measurements of game behaviour (attributed: github.com/HackerPide/Pixurf, GPL-3.0).
"use strict";

const jt = require("../lib/jumptable");
const { DUCK_FEET_GAIN, EYE_STAND } = require("../lib/consts");

// [height, tickrate flag] — flag 1 = the reference marks this reachable only at 128 tick
const REF_NORMAL = [
  [57.00,0],[56.94,0],[56.83,1],[56.68,0],[56.47,1],[56.22,0],[55.92,1],[55.57,0],[55.17,1],
  [54.72,0],[54.22,1],[53.68,0],[53.08,1],[52.44,0],[51.75,1],[51.00,0],[50.21,1],[49.37,0],
  [48.49,1],[47.55,0],[46.57,1],[45.53,0],[44.44,1],[43.32,0],[42.14,1],[40.91,0],[39.63,1],
  [38.30,0],[36.92,1],[35.50,0],[34.02,1],[32.50,0],[30.93,1],[29.31,0],[27.64,1],[25.92,0],
  [24.16,1],[22.34,0],[20.48,1],[18.56,0],[16.60,1],[14.59,0],
];
const REF_CROUCH = [66.00,65.94,65.83,65.68,65.47,65.22,64.92,64.57,64.17,63.72,63.22,62.68,
  62.08,61.44,60.75,60.00,59.21,58.37,57.49,56.55,55.57,54.53,53.44,52.32,51.14,49.91,48.63,
  47.30,45.92,44.50,43.02,41.50,39.93,38.31,36.64,34.92,33.16,31.34,29.48,27.56,25.60,23.59];

// The reference is empirical: its residuals against a best-fit quadratic scatter in both
// directions with a max of 0.0073u, i.e. cl_showpos display noise. Agreement is asserted
// inside that noise floor, not bit-exactly.
const NOISE = 0.011;

module.exports = function run(t) {
  const tbl = jt.table({ crouch: false });

  t.eq(tbl.length, REF_NORMAL.length, "normal table length matches reference");

  let maxErr = 0;
  for (let i = 0; i < REF_NORMAL.length; i++) {
    const err = Math.abs(tbl[i].h - REF_NORMAL[i][0]);
    if (err > maxErr) maxErr = err;
    t.ok(err < NOISE, `normal[${i}] ${tbl[i].h} ~= ${REF_NORMAL[i][0]} (err ${err.toFixed(4)})`);
  }
  // One entry sits exactly on a 0.005 boundary and rounds the other way, so the worst case
  // is a full display cent rather than the 0.0073 the best-fit quadratic suggested.
  t.ok(maxErr <= 0.0101, `max deviation ${maxErr.toFixed(4)}u is within cl_showpos noise`);

  // the tickrate column is a hard prediction, not a fit — it must be exact
  for (let i = 0; i < REF_NORMAL.length; i++) {
    const only128 = !tbl[i].tickrates.includes(64);
    t.eq(only128, REF_NORMAL[i][1] === 1, `normal[${i}] tickrate flag`);
  }

  // crouch is normal + 9.00, derived from the hull staying centred while ducking
  const ctbl = jt.table({ crouch: true });
  t.eq(ctbl.length, REF_CROUCH.length, "crouch table length matches reference");
  for (let i = 0; i < REF_CROUCH.length; i++) {
    t.ok(Math.abs(ctbl[i].h - REF_CROUCH[i]) < NOISE, `crouch[${i}] ${ctbl[i].h} ~= ${REF_CROUCH[i]}`);
    t.ok(Math.abs((REF_CROUCH[i] - REF_NORMAL[i][0]) - DUCK_FEET_GAIN) < 0.005,
      `reference crouch[${i}] - normal[${i}] == ${DUCK_FEET_GAIN}`);
  }

  // apex sanity: a solo jump peaks at 57, a crouch jump at 66
  t.eq(tbl[0].h, 57.00, "normal jump apex is 57.00");
  t.eq(ctbl[0].h, 66.00, "crouch jump apex is 66.00");

  // the Pixurf identity: standing eye height + apex - eye offset lands feet on the ledge
  const sols = jt.solutions(1000, { stacks: ["1man"], min: 990, max: 1010 });
  t.ok(sols.length > 0, "solo solutions exist for a ledge at z=1000");
  const apexSol = sols.find((s) => s.jump === 57.00 && !s.crouch);
  t.ok(apexSol, "apex solution present");
  t.eq(apexSol.standEye, jt.round2(1000 + EYE_STAND - 57.00), "standEye = ledge + eye - jump");
};
