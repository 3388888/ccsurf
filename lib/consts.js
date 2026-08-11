// Source-engine movement constants. Single source of truth — every other module reads
// from here rather than hardcoding, so there is exactly one place to correct if a value
// turns out to be wrong.
//
// CS:GO / Source 1 values. CS2 differs and is explicitly out of scope.
"use strict";

// ---------------------------------------------------------------- physics
const GRAVITY = 800;                        // sv_gravity
const JUMP_IMPULSE = Math.sqrt(2 * GRAVITY * 57);  // sv_jump_impulse ~= 301.993377
const STEP_SIZE = 18;                       // sv_stepsize — free height gain while walking

// ---------------------------------------------------------------- player hull
// The standing hull is 32x32x72; ducking shrinks it to 32x32x54. Ducking in mid-air keeps
// the hull centred, so the feet rise by half the 18u shrink — this is exactly why a crouch
// jump reaches 9.00u higher than a normal one (see jumptable.js).
const HULL_W = 32;
const HULL_H_STAND = 72;
const HULL_H_DUCK = 54;
const DUCK_FEET_GAIN = (HULL_H_STAND - HULL_H_DUCK) / 2;   // 9.00

// Eye offsets (view_ofs). These are the numbers cl_showpos reports, so they are what the
// user reads off their screen when lining a surf up.
const EYE_STAND = 64.09;
const EYE_DUCK = 46.07;

// Hitbox heights used when stacking players for a boost. The engine reports a hair less
// for players above the first in a stack, which is why 72.04 and 72.03 both exist.
const BOX_STAND = 72.04;
const BOX_STAND_UPPER = 72.03;
const BOX_DUCK = 54.04;
const BOX_DUCK_UPPER = 54.03;

// ---------------------------------------------------------------- surfaces
// PM_CategorizePosition: a plane counts as ground only if its normal points up enough.
// Anything shallower is a surf ramp — you slide along it instead of standing on it.
const STANDABLE_NORMAL_Z = 0.7;

// ---------------------------------------------------------------- speeds
const SPEED_WALK = 250;      // +speed held
const SPEED_RUN = 320;       // knife/pistol; rifles are slower, see WEAPON_SPEED
const SPEED_MAX_GROUND = 320;
const AIR_SPEED_CAP = 30;    // the 30 u/s air-accel window that makes strafing work
const AIR_ACCELERATE = 12;   // sv_airaccelerate
const ACCELERATE = 5.5;      // sv_accelerate
const FRICTION = 5.2;        // sv_friction
const STOP_SPEED = 80;       // sv_stopspeed

// ---------------------------------------------------------------- bsp contents flags
const CONTENTS_SOLID = 0x1;
const CONTENTS_WATER = 0x20;
const CONTENTS_PLAYERCLIP = 0x10000;
const CONTENTS_MONSTERCLIP = 0x20000;
const CONTENTS_LADDER = 0x20000000;

// Everything that stops a player. Grate/window contents are deliberately excluded — they
// block bullets, not movement.
const CONTENTS_PLAYER_SOLID = CONTENTS_SOLID | CONTENTS_PLAYERCLIP;

// ---------------------------------------------------------------- tickrates
const TICKRATES = [64, 128];

module.exports = {
  GRAVITY, JUMP_IMPULSE, STEP_SIZE,
  HULL_W, HULL_H_STAND, HULL_H_DUCK, DUCK_FEET_GAIN,
  EYE_STAND, EYE_DUCK,
  BOX_STAND, BOX_STAND_UPPER, BOX_DUCK, BOX_DUCK_UPPER,
  STANDABLE_NORMAL_Z,
  SPEED_WALK, SPEED_RUN, SPEED_MAX_GROUND, AIR_SPEED_CAP, AIR_ACCELERATE,
  ACCELERATE, FRICTION, STOP_SPEED,
  CONTENTS_SOLID, CONTENTS_WATER, CONTENTS_PLAYERCLIP, CONTENTS_MONSTERCLIP,
  CONTENTS_LADDER, CONTENTS_PLAYER_SOLID,
  TICKRATES,
};
