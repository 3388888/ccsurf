// Jump arithmetic for the Calculator tab.
//
// Mirrors core/src/jumptable.rs exactly — see that file for the derivation and for why the
// reference table is treated as a noisy oracle rather than ground truth. Kept in JS so the
// calculator works with no round-trip to Rust; core/ stays the authority.
"use strict";

const PixelJump = (() => {
  const GRAVITY = 800, JUMP_APEX = 57, JUMP_IMPULSE = Math.sqrt(2 * GRAVITY * JUMP_APEX);
  const DUCK_FEET_GAIN = 9, EYE_STAND = 64.09, EYE_DUCK = 46.07;
  const BOX_STAND = 72.04, BOX_STAND_UPPER = 72.03, BOX_DUCK = 54.04, BOX_DUCK_UPPER = 54.03;
  const APEX_DUP = 0.02;
  const round2 = (x) => Math.floor(x * 100 + 0.5) / 100;

  function arc(tickrate) {
    const dt = 1 / tickrate, out = [];
    let v = JUMP_IMPULSE, z = 0;
    for (let n = 0; n < tickrate * 4; n++) {
      v -= GRAVITY * dt * 0.5; z += v * dt; v -= GRAVITY * dt * 0.5;
      out.push(z);
      if (z < 0) break;
    }
    return out;
  }
  // At 128 tick the apex is sampled twice within 0.01u — one moment, not two options, so the
  // duplicate goes. At 64 tick the neighbours are ~0.07u apart and both are real, which is
  // why the test is proximity and never index.
  function heights(tickrate) {
    const a = arc(tickrate);
    let pk = 0;
    for (let i = 1; i < a.length; i++) if (a[i] > a[pk]) pk = i;
    const apex = a[pk];
    const dup = (pk + 1 < a.length && apex - a[pk + 1] < APEX_DUP) ? pk + 1 : -1;
    const out = [];
    for (let i = pk; i < a.length; i++) {
      if (i === dup) continue;
      out.push(round2(JUMP_APEX - (apex - a[i])));
    }
    return out;
  }
  function table(crouch) {
    const h128 = heights(128), h64 = heights(64);
    const lift = crouch ? DUCK_FEET_GAIN : 0;
    const on64 = (h) => h64.some((x) => Math.abs(x - h) < 0.015);
    const out = [];
    for (const h of h128) {
      if (h < 14) break;
      out.push({ h: round2(h + lift), t64: on64(h) });
    }
    return out;
  }
  const STACKS = [
    { id:"1man", label:"Solo", players:1, eye:EYE_STAND, drop:0 },
    { id:"2man", label:"2-man boost", players:2, eye:EYE_STAND, drop:BOX_STAND_UPPER },
    { id:"2man_wo", label:"2-man walk-off", players:2, eye:EYE_STAND, drop:BOX_STAND, walkoff:true },
    { id:"2man_wo_c", label:"2-man walk-off (crouched)", players:2, eye:EYE_DUCK, drop:BOX_DUCK_UPPER, walkoff:true },
    { id:"3man", label:"3-man boost", players:3, eye:EYE_STAND, drop:BOX_STAND_UPPER+BOX_STAND },
    { id:"3man_1c", label:"3-man (1 crouched)", players:3, eye:EYE_STAND, drop:BOX_STAND_UPPER+BOX_DUCK },
    { id:"3man_2c", label:"3-man (2 crouched)", players:3, eye:EYE_DUCK, drop:BOX_DUCK_UPPER+BOX_DUCK },
    { id:"4man", label:"4-man boost", players:4, eye:EYE_STAND, drop:2*BOX_STAND_UPPER+BOX_STAND },
    { id:"4man_1c", label:"4-man (1 crouched)", players:4, eye:EYE_STAND, drop:2*BOX_STAND_UPPER+BOX_DUCK },
    { id:"4man_2c", label:"4-man (2 crouched)", players:4, eye:EYE_DUCK, drop:BOX_STAND_UPPER+BOX_DUCK_UPPER+BOX_DUCK },
    { id:"5man", label:"5-man boost", players:5, eye:EYE_STAND, drop:3*BOX_STAND_UPPER+BOX_STAND },
    { id:"5man_1c", label:"5-man (1 crouched)", players:5, eye:EYE_STAND, drop:3*BOX_STAND_UPPER+BOX_DUCK },
    { id:"5man_2c", label:"5-man (2 crouched)", players:5, eye:EYE_DUCK, drop:2*BOX_STAND_UPPER+BOX_DUCK_UPPER+BOX_DUCK },
  ];
  function solutions(ledgeZ, min, max, tick, maxPlayers) {
    const out = [];
    for (const st of STACKS) {
      if (st.players > maxPlayers) continue;
      if (st.walkoff) {
        const e = round2(ledgeZ + st.eye - st.drop);
        if (e >= min && e <= max) out.push({ ...st, jump:null, crouch:false, standEye:e, t64:true });
        continue;
      }
      for (const crouch of [false, true]) {
        for (const j of table(crouch)) {
          if (tick === 64 && !j.t64) continue;
          if (tick === 128 && j.t64) continue;
          const e = round2(ledgeZ + st.eye - j.h - st.drop);
          if (e < min || e > max) continue;
          out.push({ ...st, jump:j.h, crouch, standEye:e, t64:j.t64 });
        }
      }
    }
    out.sort((a, b) => a.players - b.players || (b.jump || 0) - (a.jump || 0));
    return out;
  }
  return { table, heights, solutions, EYE_STAND, EYE_DUCK };
})();
