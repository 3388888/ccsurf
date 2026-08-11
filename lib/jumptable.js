// The heights a Source jump actually passes through, and the Pixurf arithmetic built on top.
//
// A jump is not a continuous arc as far as landing on a ledge is concerned: the engine only
// samples your position once per tick, so your feet only ever exist at a discrete set of
// heights. Landing on a pixel-thin ledge means one of those samples has to coincide with it.
// That discrete set is the whole game.
//
// DERIVATION (not copied). Source integrates gravity in two half-steps around the move:
//
//     StartGravity()   v -= g*dt/2
//     AirMove()        z += v*dt
//     FinishGravity()  v -= g*dt/2
//
// starting from v = sqrt(2*g*57). Sampling that at dt = 1/128 and reading down from the
// apex reproduces the reference table (HackerPide/Pixurf) to within 0.0073u on every one of
// its 42 entries, and reproduces its 64-vs-128 tickrate column exactly.
//
// The reference table is NOT reproduced bit-exactly, and it shouldn't be: its residuals
// scatter in BOTH directions (56.84 vs 56.83, but 42.13 vs 42.14), which a computed table
// cannot do. Those numbers were measured in-game off cl_showpos and carry its 0.01 display
// rounding. The values here are the underlying physics, so they are the more accurate of
// the two. test/jumptable.test.js asserts agreement stays inside that measurement noise.
//
// Crouch jumps are not a separate table. Ducking mid-air keeps the hull centred, so the feet
// rise by half the 72->54 shrink: every crouch height is its normal counterpart + 9.00,
// which holds for 42/42 reference entries.
"use strict";

const { GRAVITY, JUMP_IMPULSE, DUCK_FEET_GAIN, EYE_STAND, EYE_DUCK,
  BOX_STAND, BOX_STAND_UPPER, BOX_DUCK, BOX_DUCK_UPPER, TICKRATES } = require("./consts");

// Java rounds half away from zero; JS toFixed rounds the binary value. Match the former so
// comparisons against the reference aren't polluted by a rounding-mode difference.
const round2 = (x) => Math.floor(x * 100 + 0.5) / 100;

// Feet height at every tick of a jump, from launch until back through zero.
function arc(tickrate) {
  const dt = 1 / tickrate;
  const out = [];
  let v = JUMP_IMPULSE, z = 0;
  for (let n = 0; n < tickrate * 4; n++) {
    v -= GRAVITY * dt * 0.5;
    z += v * dt;
    v -= GRAVITY * dt * 0.5;
    out.push(z);
    if (z < 0) break;
  }
  return out;
}

// Two samples that land this close together are the same physical apex, not two heights
// you could choose between.
const APEX_DUP = 0.02;

// The reachable heights, highest first.
//
// At 128 tick the arc is flat enough at the top to be sampled twice within 0.01u. Those are
// one moment, not two options, so the lower is dropped. At 64 tick the neighbouring samples
// are ~0.07u apart and both are real — dropping one there would delete a reachable height
// and corrupt the tickrate column, so the test is on proximity, never on index.
function heights(tickrate) {
  const a = arc(tickrate);
  let pk = 0;
  for (let i = 1; i < a.length; i++) if (a[i] > a[pk]) pk = i;
  const apex = a[pk];
  const dupIdx = pk + 1 < a.length && apex - a[pk + 1] < APEX_DUP ? pk + 1 : -1;
  const out = [];
  for (let i = pk; i < a.length; i++) {
    if (i === dupIdx) continue;
    out.push(round2(57.0 - (apex - a[i])));     // normalise apex to the exact 57.0
  }
  return out;
}

/**
 * Every jump height a player can land a ledge on.
 * @param {{crouch?:boolean, minHeight?:number}} opts
 * @returns {Array<{h:number, tickrates:number[]}>} highest first
 */
function table({ crouch = false, minHeight = 14 } = {}) {
  const h128 = heights(128);
  const h64 = heights(64);
  const lift = crouch ? DUCK_FEET_GAIN : 0;
  // The two arcs peak at slightly different sub-tick phases, so a shared 64-tick height can
  // land 0.01 apart in the two lists. Match on proximity — string equality drops real hits.
  const on64 = (h) => h64.some((x) => Math.abs(x - h) < 0.015);
  const out = [];
  for (const h of h128) {
    if (h < minHeight) break;
    out.push({
      h: round2(h + lift),
      // a 64-tick server samples half as often, so only some heights exist there
      tickrates: on64(h) ? TICKRATES.slice() : [128],
    });
  }
  return out;
}

// ---------------------------------------------------------------- boost stacks
//
// Standing on someone's head raises your feet by their hitbox height. These are the stack
// configurations worth lining up; `drop` is how far below the ledge the BOTTOM player's eyes
// must sit, so standEye = ledgeZ + eye - jumpHeight - drop.
//
// "walkoff" entries are the ones where the top player doesn't jump at all — they walk off
// the head of the player below, so no jump height is involved.
const STACKS = [
  { id: "1man", label: "Solo", players: 1, eye: EYE_STAND, drop: 0 },
  { id: "2man", label: "2-man boost", players: 2, eye: EYE_STAND, drop: BOX_STAND_UPPER },
  { id: "2man_walkoff", label: "2-man walk-off", players: 2, eye: EYE_STAND, drop: BOX_STAND, walkoff: true },
  { id: "2man_walkoff_crouch", label: "2-man walk-off (crouched)", players: 2, eye: EYE_DUCK, drop: BOX_DUCK_UPPER, walkoff: true },
  { id: "3man", label: "3-man boost", players: 3, eye: EYE_STAND, drop: BOX_STAND_UPPER + BOX_STAND },
  { id: "3man_1crouch", label: "3-man (1 crouched)", players: 3, eye: EYE_STAND, drop: BOX_STAND_UPPER + BOX_DUCK },
  { id: "3man_2crouch", label: "3-man (2 crouched)", players: 3, eye: EYE_DUCK, drop: BOX_DUCK_UPPER + BOX_DUCK },
  { id: "4man", label: "4-man boost", players: 4, eye: EYE_STAND, drop: 2 * BOX_STAND_UPPER + BOX_STAND },
  { id: "4man_1crouch", label: "4-man (1 crouched)", players: 4, eye: EYE_STAND, drop: 2 * BOX_STAND_UPPER + BOX_DUCK },
  { id: "4man_2crouch", label: "4-man (2 crouched)", players: 4, eye: EYE_DUCK, drop: BOX_STAND_UPPER + BOX_DUCK_UPPER + BOX_DUCK },
  { id: "5man", label: "5-man boost", players: 5, eye: EYE_STAND, drop: 3 * BOX_STAND_UPPER + BOX_STAND },
  { id: "5man_1crouch", label: "5-man (1 crouched)", players: 5, eye: EYE_STAND, drop: 3 * BOX_STAND_UPPER + BOX_DUCK },
  { id: "5man_2crouch", label: "5-man (2 crouched)", players: 5, eye: EYE_DUCK, drop: 2 * BOX_STAND_UPPER + BOX_DUCK_UPPER + BOX_DUCK },
];

/**
 * Every way to reach a ledge, as eye heights to line up against cl_showpos.
 *
 * @param {number} ledgeZ    z of the surface you want your feet on
 * @param {{min?:number, max?:number, stacks?:string[], tickrate?:number}} opts
 *        min/max bound the standing eye height you're willing to look for.
 * @returns {Array<{stack, label, players, jump, crouch, standEye, tickrates}>}
 */
function solutions(ledgeZ, { min = -Infinity, max = Infinity, stacks = null, tickrate = null } = {}) {
  const out = [];
  const wanted = stacks ? new Set(stacks) : null;
  for (const st of STACKS) {
    if (wanted && !wanted.has(st.id)) continue;
    if (st.walkoff) {
      // no jump involved — the height is fixed by the stack alone
      const standEye = round2(ledgeZ + st.eye - st.drop);
      if (standEye >= min && standEye <= max) {
        out.push({ stack: st.id, label: st.label, players: st.players, jump: null,
          crouch: false, standEye, tickrates: TICKRATES.slice() });
      }
      continue;
    }
    for (const crouch of [false, true]) {
      for (const j of table({ crouch })) {
        if (tickrate && !j.tickrates.includes(tickrate)) continue;
        const standEye = round2(ledgeZ + st.eye - j.h - st.drop);
        if (standEye < min || standEye > max) continue;
        out.push({ stack: st.id, label: st.label, players: st.players, jump: j.h,
          crouch, standEye, tickrates: j.tickrates });
      }
    }
  }
  // easiest first: fewest players, then the biggest jump margin
  out.sort((a, b) => a.players - b.players || (b.jump || 0) - (a.jump || 0));
  return out;
}

module.exports = { arc, heights, table, solutions, STACKS, round2 };
