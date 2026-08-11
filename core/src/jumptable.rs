//! The heights a Source jump actually passes through, and the arithmetic built on them.
//!
//! A jump is not a continuous arc as far as landing on a ledge goes: the engine samples your
//! position once per tick, so your feet only ever exist at a discrete set of heights. Landing
//! on a pixel-thin ledge means one of those samples has to coincide with it. That discrete
//! set is the whole game.
//!
//! DERIVATION (not copied). Source integrates gravity in two half-steps around the move:
//!
//! ```text
//!   StartGravity()   v -= g*dt/2
//!   AirMove()        z += v*dt
//!   FinishGravity()  v -= g*dt/2
//! ```
//!
//! starting from v = sqrt(2*g*57). Sampling that at dt = 1/128 and reading down from the apex
//! reproduces the reference table (HackerPide/Pixurf) to within 0.01u on every one of its 42
//! entries, and reproduces its 64-vs-128 tickrate column exactly.
//!
//! The reference is NOT reproduced bit-exactly, and it shouldn't be: its residuals against a
//! best-fit quadratic scatter in BOTH directions (56.84 vs 56.83, but 42.13 vs 42.14), which
//! a computed table cannot do. Those numbers were measured in-game off cl_showpos and carry
//! its 0.01 display rounding. What is here is the underlying physics, so it is the more
//! accurate of the two.
//!
//! Crouch jumps are not a separate table: every crouch height is its normal counterpart plus
//! [`consts::DUCK_FEET_GAIN`], which holds for 42/42 reference entries.

use crate::consts::*;

/// Java rounds half away from zero; matching that keeps comparisons against the reference
/// free of rounding-mode noise.
pub fn round2(x: f64) -> f64 { (x * 100.0 + 0.5).floor() / 100.0 }

/// Feet height at every tick of a jump, from launch until back through zero.
pub fn arc(tickrate: u32) -> Vec<f64> {
    let dt = 1.0 / tickrate as f64;
    let mut out = Vec::with_capacity(tickrate as usize * 2);
    let (mut v, mut z) = (jump_impulse(), 0.0f64);
    for _ in 0..(tickrate * 4) {
        v -= GRAVITY * dt * 0.5;
        z += v * dt;
        v -= GRAVITY * dt * 0.5;
        out.push(z);
        if z < 0.0 { break; }
    }
    out
}

/// Two samples this close together are the same physical apex, not two options.
const APEX_DUP: f64 = 0.02;

/// The reachable heights, highest first.
///
/// At 128 tick the arc is flat enough at the top to be sampled twice within 0.01u; those are
/// one moment, so the lower is dropped. At 64 tick the neighbouring samples are ~0.07u apart
/// and both are real — dropping one there would delete a reachable height and corrupt the
/// tickrate column. Hence the test is on proximity, never on index.
pub fn heights(tickrate: u32) -> Vec<f64> {
    let a = arc(tickrate);
    let mut pk = 0usize;
    for i in 1..a.len() { if a[i] > a[pk] { pk = i; } }
    let apex = a[pk];
    let dup = if pk + 1 < a.len() && apex - a[pk + 1] < APEX_DUP { pk as i64 + 1 } else { -1 };
    let mut out = Vec::new();
    for i in pk..a.len() {
        if i as i64 == dup { continue; }
        out.push(round2(JUMP_APEX - (apex - a[i])));   // normalise apex to exactly 57.0
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
pub struct JumpHeight {
    pub h: f64,
    /// Tickrates on which this height exists at all.
    pub tickrates: Vec<u32>,
}

/// Every jump height a player can land a ledge on, highest first.
pub fn table(crouch: bool, min_height: f64) -> Vec<JumpHeight> {
    let h128 = heights(128);
    let h64 = heights(64);
    let lift = if crouch { DUCK_FEET_GAIN } else { 0.0 };
    // The two arcs peak at slightly different sub-tick phases, so a height shared by both can
    // land 0.01 apart in the two lists. Match on proximity — exact equality drops real hits.
    let on64 = |h: f64| h64.iter().any(|x| (x - h).abs() < 0.015);
    let mut out = Vec::new();
    for h in h128 {
        if h < min_height { break; }
        out.push(JumpHeight {
            h: round2(h + lift),
            tickrates: if on64(h) { TICKRATES.to_vec() } else { vec![128] },
        });
    }
    out
}

// ---------------------------------------------------------------- boost stacks

/// Standing on someone's head raises your feet by their hitbox height. `drop` is how far
/// below the ledge the bottom player's eyes must sit:
/// `stand_eye = ledge_z + eye - jump_height - drop`.
pub struct Stack {
    pub id: &'static str,
    pub label: &'static str,
    pub players: u8,
    pub eye: f64,
    pub drop: f64,
    /// The top player walks off the head below instead of jumping, so no jump height applies.
    pub walkoff: bool,
}

pub fn stacks() -> Vec<Stack> {
    vec![
        Stack { id: "1man", label: "Solo", players: 1, eye: EYE_STAND, drop: 0.0, walkoff: false },
        Stack { id: "2man", label: "2-man boost", players: 2, eye: EYE_STAND, drop: BOX_STAND_UPPER, walkoff: false },
        Stack { id: "2man_walkoff", label: "2-man walk-off", players: 2, eye: EYE_STAND, drop: BOX_STAND, walkoff: true },
        Stack { id: "2man_walkoff_crouch", label: "2-man walk-off (crouched)", players: 2, eye: EYE_DUCK, drop: BOX_DUCK_UPPER, walkoff: true },
        Stack { id: "3man", label: "3-man boost", players: 3, eye: EYE_STAND, drop: BOX_STAND_UPPER + BOX_STAND, walkoff: false },
        Stack { id: "3man_1crouch", label: "3-man (1 crouched)", players: 3, eye: EYE_STAND, drop: BOX_STAND_UPPER + BOX_DUCK, walkoff: false },
        Stack { id: "3man_2crouch", label: "3-man (2 crouched)", players: 3, eye: EYE_DUCK, drop: BOX_DUCK_UPPER + BOX_DUCK, walkoff: false },
        Stack { id: "4man", label: "4-man boost", players: 4, eye: EYE_STAND, drop: 2.0 * BOX_STAND_UPPER + BOX_STAND, walkoff: false },
        Stack { id: "4man_1crouch", label: "4-man (1 crouched)", players: 4, eye: EYE_STAND, drop: 2.0 * BOX_STAND_UPPER + BOX_DUCK, walkoff: false },
        Stack { id: "4man_2crouch", label: "4-man (2 crouched)", players: 4, eye: EYE_DUCK, drop: BOX_STAND_UPPER + BOX_DUCK_UPPER + BOX_DUCK, walkoff: false },
        Stack { id: "5man", label: "5-man boost", players: 5, eye: EYE_STAND, drop: 3.0 * BOX_STAND_UPPER + BOX_STAND, walkoff: false },
        Stack { id: "5man_1crouch", label: "5-man (1 crouched)", players: 5, eye: EYE_STAND, drop: 3.0 * BOX_STAND_UPPER + BOX_DUCK, walkoff: false },
        Stack { id: "5man_2crouch", label: "5-man (2 crouched)", players: 5, eye: EYE_DUCK, drop: 2.0 * BOX_STAND_UPPER + BOX_DUCK_UPPER + BOX_DUCK, walkoff: false },
    ]
}

#[derive(Clone, Debug)]
pub struct Solution {
    pub stack: &'static str,
    pub label: &'static str,
    pub players: u8,
    /// `None` for walk-off stacks, where no jump is involved.
    pub jump: Option<f64>,
    pub crouch: bool,
    /// The cl_showpos z the bottom player must be standing at.
    pub stand_eye: f64,
    pub tickrates: Vec<u32>,
}

/// Every way to reach a ledge, as eye heights to line up against cl_showpos.
pub fn solutions(ledge_z: f64, min: f64, max: f64, only_tickrate: Option<u32>) -> Vec<Solution> {
    let mut out = Vec::new();
    for st in stacks() {
        if st.walkoff {
            let stand_eye = round2(ledge_z + st.eye - st.drop);
            if stand_eye >= min && stand_eye <= max {
                out.push(Solution { stack: st.id, label: st.label, players: st.players,
                    jump: None, crouch: false, stand_eye, tickrates: TICKRATES.to_vec() });
            }
            continue;
        }
        for &crouch in &[false, true] {
            for j in table(crouch, 14.0) {
                if let Some(tr) = only_tickrate { if !j.tickrates.contains(&tr) { continue; } }
                let stand_eye = round2(ledge_z + st.eye - j.h - st.drop);
                if stand_eye < min || stand_eye > max { continue; }
                out.push(Solution { stack: st.id, label: st.label, players: st.players,
                    jump: Some(j.h), crouch, stand_eye, tickrates: j.tickrates.clone() });
            }
        }
    }
    // easiest first: fewest players, then the biggest jump margin
    out.sort_by(|a, b| a.players.cmp(&b.players)
        .then(b.jump.unwrap_or(0.0).partial_cmp(&a.jump.unwrap_or(0.0)).unwrap()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Reference values from HackerPide/Pixurf (GPL-3.0), used ONLY as a test oracle —
    // nothing outside this module reads them. They are in-game measurements, so agreement is
    // asserted inside their 0.01 display-rounding noise rather than bit-exactly.
    const REF_NORMAL: [(f64, u8); 42] = [
        (57.00,0),(56.94,0),(56.83,1),(56.68,0),(56.47,1),(56.22,0),(55.92,1),(55.57,0),(55.17,1),
        (54.72,0),(54.22,1),(53.68,0),(53.08,1),(52.44,0),(51.75,1),(51.00,0),(50.21,1),(49.37,0),
        (48.49,1),(47.55,0),(46.57,1),(45.53,0),(44.44,1),(43.32,0),(42.14,1),(40.91,0),(39.63,1),
        (38.30,0),(36.92,1),(35.50,0),(34.02,1),(32.50,0),(30.93,1),(29.31,0),(27.64,1),(25.92,0),
        (24.16,1),(22.34,0),(20.48,1),(18.56,0),(16.60,1),(14.59,0),
    ];
    const REF_CROUCH: [f64; 42] = [
        66.00,65.94,65.83,65.68,65.47,65.22,64.92,64.57,64.17,63.72,63.22,62.68,62.08,61.44,
        60.75,60.00,59.21,58.37,57.49,56.55,55.57,54.53,53.44,52.32,51.14,49.91,48.63,47.30,
        45.92,44.50,43.02,41.50,39.93,38.31,36.64,34.92,33.16,31.34,29.48,27.56,25.60,23.59,
    ];
    const NOISE: f64 = 0.011;

    #[test]
    fn matches_reference_heights() {
        let t = table(false, 14.0);
        assert_eq!(t.len(), REF_NORMAL.len(), "table length");
        let mut max_err: f64 = 0.0;
        for (i, (h, _)) in REF_NORMAL.iter().enumerate() {
            let err = (t[i].h - h).abs();
            max_err = max_err.max(err);
            assert!(err < NOISE, "normal[{i}] {} vs {h} (err {err:.4})", t[i].h);
        }
        assert!(max_err <= 0.0101, "max deviation {max_err:.4}u within cl_showpos noise");
    }

    #[test]
    fn matches_reference_tickrate_flags() {
        // this column is a hard prediction, not a fit — it must be exact
        let t = table(false, 14.0);
        for (i, (_, flag)) in REF_NORMAL.iter().enumerate() {
            let only128 = !t[i].tickrates.contains(&64);
            assert_eq!(only128, *flag == 1, "normal[{i}] tickrate flag");
        }
    }

    #[test]
    fn crouch_is_normal_plus_nine() {
        let c = table(true, 14.0);
        assert_eq!(c.len(), REF_CROUCH.len());
        for i in 0..REF_CROUCH.len() {
            assert!((c[i].h - REF_CROUCH[i]).abs() < NOISE, "crouch[{i}]");
            assert!(((REF_CROUCH[i] - REF_NORMAL[i].0) - DUCK_FEET_GAIN).abs() < 0.005,
                "reference crouch[{i}] - normal[{i}] == {DUCK_FEET_GAIN}");
        }
    }

    #[test]
    fn apexes() {
        assert_eq!(table(false, 14.0)[0].h, 57.00);
        assert_eq!(table(true, 14.0)[0].h, 66.00);
    }

    #[test]
    fn pixurf_identity() {
        let s = solutions(1000.0, 990.0, 1010.0, None);
        assert!(!s.is_empty());
        let apex = s.iter().find(|x| x.jump == Some(57.00) && !x.crouch).expect("apex solution");
        assert_eq!(apex.stand_eye, round2(1000.0 + EYE_STAND - 57.00));
    }

    #[test]
    fn standable_cutoff_is_45_573_degrees() {
        assert!(std::f64::consts::FRAC_1_SQRT_2 > STANDABLE_NORMAL_Z, "45 deg is standable");
        assert!((46.0f64).to_radians().cos() < STANDABLE_NORMAL_Z, "46 deg is surfable");
        assert!((STANDABLE_NORMAL_Z.acos().to_degrees() - 45.573).abs() < 0.01);
    }
}
