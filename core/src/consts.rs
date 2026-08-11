//! Source-engine movement constants — CS:GO / Source 1. CS2 differs and is out of scope.
//!
//! Single source of truth: nothing else in the crate hardcodes these, so there is exactly
//! one place to correct if a value turns out to be wrong.

// ---------------------------------------------------------------- physics
pub const GRAVITY: f64 = 800.0;                 // sv_gravity
pub const JUMP_APEX: f64 = 57.0;                // a standing jump peaks exactly here
/// sv_jump_impulse ~= 301.993377, i.e. sqrt(2 * g * 57).
pub fn jump_impulse() -> f64 { (2.0 * GRAVITY * JUMP_APEX).sqrt() }
pub const STEP_SIZE: f64 = 18.0;                // sv_stepsize — free height while walking

// ---------------------------------------------------------------- player hull
pub const HULL_W: f64 = 32.0;
pub const HULL_H_STAND: f64 = 72.0;
pub const HULL_H_DUCK: f64 = 54.0;
/// Ducking mid-air keeps the hull centred, so the feet rise by half the shrink. This is
/// exactly why a crouch jump reaches 9.00u higher than a normal one.
pub const DUCK_FEET_GAIN: f64 = (HULL_H_STAND - HULL_H_DUCK) / 2.0;   // 9.00

/// Eye offsets (view_ofs) — what cl_showpos reports, so what the user reads off screen.
pub const EYE_STAND: f64 = 64.09;
pub const EYE_DUCK: f64 = 46.07;

/// Hitbox heights for boost stacks. The engine reports a hair less for players above the
/// first in a stack, which is why both 72.04 and 72.03 exist.
pub const BOX_STAND: f64 = 72.04;
pub const BOX_STAND_UPPER: f64 = 72.03;
pub const BOX_DUCK: f64 = 54.04;
pub const BOX_DUCK_UPPER: f64 = 54.03;

// ---------------------------------------------------------------- surfaces
/// PM_CategorizePosition: a plane is ground only if its normal points up at least this much.
/// 0.7 is 45.573 degrees — so an exactly-45-degree ramp is still a floor, and surf ramps are
/// the ones steeper than that.
pub const STANDABLE_NORMAL_Z: f64 = 0.7;

// ---------------------------------------------------------------- speeds
pub const SPEED_WALK: f64 = 250.0;
pub const SPEED_RUN: f64 = 320.0;
pub const AIR_SPEED_CAP: f64 = 30.0;    // the window that makes air-strafing work
pub const AIR_ACCELERATE: f64 = 12.0;   // sv_airaccelerate
pub const ACCELERATE: f64 = 5.5;        // sv_accelerate
pub const FRICTION: f64 = 5.2;          // sv_friction
pub const STOP_SPEED: f64 = 80.0;       // sv_stopspeed

// ---------------------------------------------------------------- bsp contents
pub const CONTENTS_SOLID: i32 = 0x1;
pub const CONTENTS_WATER: i32 = 0x20;
pub const CONTENTS_PLAYERCLIP: i32 = 0x10000;
pub const CONTENTS_MONSTERCLIP: i32 = 0x20000;
pub const CONTENTS_LADDER: i32 = 0x2000_0000;

/// Everything that stops a player. Grate/window contents are deliberately excluded — they
/// block bullets, not movement.
pub const CONTENTS_PLAYER_SOLID: i32 = CONTENTS_SOLID | CONTENTS_PLAYERCLIP;

pub const TICKRATES: [u32; 2] = [64, 128];
