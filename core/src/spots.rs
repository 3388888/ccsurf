//! Turning collision geometry into spots you can actually go and stand on.
//!
//! Three surface classes, split by how the engine treats them rather than by how they look:
//!
//!   * `Ground`    — standable and at least a hull wide. Ordinary floor, not interesting.
//!   * `PixelWalk` — standable (normal.z >= 0.7) but narrower than the 32u hull. You can
//!                   stand and walk on it; it is just a thin ledge.
//!   * `PixelSurf` — a sliver too small for the engine to give you ground. You are technically
//!                   airborne and perched, which is the state demo-reader detects as
//!                   "airborne, no horizontal speed, no vertical movement".
//!   * `Surf`      — normal.z < 0.7. You slide instead of standing (the de_biome dome).
//!
//! Out-of-bounds detection is a reachability diff: flood-fill what a player can walk and jump
//! to from spawn, then anything standable-or-surfable that the fill never reached is a
//! candidate boost spot.

use crate::collide::{Face, Geometry, P3};
use crate::consts::*;
use crate::jumptable;

/// Narrower than this and the engine won't hand you ground — you perch instead of standing.
const PIXEL_SLIVER: f64 = 2.0;
/// ...but only if it is short as well. Beyond this it is a ledge or a run of trim, which you
/// walk along rather than balance on, and there is nothing to hunt for.
const PERCH_MAX_LEN: f64 = 48.0;
/// Ledges wider than the hull are ordinary floor.
const HULL_WIDTH: f64 = HULL_W;
/// A surf ramp needs to be big enough to actually ride.
const MIN_SURF_AREA: f64 = 256.0;

/// Adjacent surfaces you can simply walk between.
const WALK_GAP: f64 = 48.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { Ground, PixelWalk, PixelSurf, Surf }

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self { Kind::Ground => "ground", Kind::PixelWalk => "pixelwalk",
                     Kind::PixelSurf => "pixelsurf", Kind::Surf => "surf" }
    }
}

pub fn classify(f: &Face) -> Kind {
    if f.n[2] >= STANDABLE_NORMAL_Z {
        let w = f.min_width();
        if w < PIXEL_SLIVER { Kind::PixelSurf }
        else if w < HULL_WIDTH { Kind::PixelWalk }
        else { Kind::Ground }
    } else {
        Kind::Surf
    }
}

#[derive(Clone, Debug)]
pub struct Spot {
    pub kind: Kind,
    /// Where to put your feet.
    pub pos: P3,
    /// The patch's real footprint (minX, minY, maxX, maxY). The view draws this rather than a
    /// dot: a spot is a surface with a shape, and a blob tells you nothing about where on it
    /// to actually stand.
    pub rect: [f64; 4],
    /// The cl_showpos z you will read once standing there.
    pub eye_z: f64,
    pub width: f64,
    pub area: f64,
    /// Steepness in degrees from horizontal (0 = flat).
    pub slope_deg: f64,
    /// True if this surface is a player-clip brush rather than visible world geometry —
    /// i.e. an invisible ledge.
    pub is_clip: bool,
    pub is_disp: bool,
    /// The standing hull doesn't clear here but the ducked one does — you have to hold
    /// crouch to stay on it.
    pub duck_only: bool,
    /// False when the reachability fill never got here: a boost/flashboost candidate.
    pub reachable: bool,
    /// How far above the nearest reachable surface below it. The number that says whether
    /// this is a 2-man boost or needs a flashboost.
    pub height_above_reachable: f64,
    /// Why it is out of bounds, when it is.
    pub oob_class: Option<&'static str>,
    /// Ways to jump onto it, best first (empty for unreachable-by-jump spots).
    pub entries: Vec<Entry>,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub label: &'static str,
    pub players: u8,
    pub jump: Option<f64>,
    pub crouch: bool,
    pub stand_eye: f64,
    pub tickrates: Vec<u32>,
}

/// Horizontal distance a player can cover while rising `dz`, at run speed.
///
/// From the jump arc: the time spent at or above height `dz` is 2*sqrt(2*(apex-dz)/g), and
/// you carry run speed the whole way. Note this must NOT shortcut to the walking gap for
/// small rises — a flat gap is exactly where the full ~260u jump is available, and capping
/// it at walking distance makes the reachability fill unable to cross open ground, which
/// would report most of a map as out of bounds.
fn jump_gap(dz: f64) -> f64 {
    let apex = JUMP_APEX + DUCK_FEET_GAIN;          // assume a crouch jump is allowed
    if dz >= apex { return 0.0; }
    let ballistic = SPEED_RUN * 2.0 * (2.0 * (apex - dz.max(0.0)) / GRAVITY).sqrt();
    ballistic.max(WALK_GAP)
}

const MAX_RISE: f64 = JUMP_APEX + DUCK_FEET_GAIN;   // 66u, the best a solo player can do

// ---------------------------------------------------------------- patch merging
//
// The BSP compiler splits one flat surface into many faces, so raw faces are the wrong unit
// to classify: a single wall top on cs_italy arrives as ~400 separate 16u strips, each of
// which would be reported as its own "pixelwalk". Worse, the width test is meaningless per
// face — a 4-metre floor cut into 16u strips looks like 400 thin ledges.
//
// So coplanar faces that touch are merged first, and everything downstream works on patches.

/// Faces are the same plane if their normals and offsets agree to this much. Loose enough to
/// survive f32 plane storage, tight enough not to weld a ramp to the floor it meets.
const PLANE_EPS_N: f64 = 0.002;
const PLANE_EPS_D: f64 = 0.05;
/// Two coplanar faces belong to the same patch if their footprints come within this.
const TOUCH_EPS: f64 = 1.0;

pub struct Patch {
    pub faces: Vec<u32>,
    pub n: P3,
    pub contents_or: i32,
    pub contents_and: i32,
    pub is_disp: bool,
    pub area: f64,
    pub centroid: P3,
    /// XY bounds of the whole merged region — this is what the width test must use.
    pub min: [f64; 2],
    pub max: [f64; 2],
}

impl Patch {
    pub fn min_width(&self) -> f64 { (self.max[0] - self.min[0]).min(self.max[1] - self.min[1]) }
    /// Longest horizontal extent. A perch is small in BOTH directions; a 1u lip running 230u
    /// along a wall is trim, and looks identical to a pixel surf if you only measure the
    /// narrow side.
    pub fn max_width(&self) -> f64 { (self.max[0] - self.min[0]).max(self.max[1] - self.min[1]) }
}

struct Dsu(Vec<u32>);
impl Dsu {
    fn new(n: usize) -> Dsu { Dsu((0..n as u32).collect()) }
    fn find(&mut self, mut x: u32) -> u32 {
        while self.0[x as usize] != x { let p = self.0[x as usize]; self.0[x as usize] = self.0[p as usize]; x = p; }
        x
    }
    fn union(&mut self, a: u32, b: u32) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb { self.0[ra as usize] = rb; }
    }
}

fn face_bounds2(f: &Face) -> ([f64; 2], [f64; 2]) {
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for p in &f.poly { for k in 0..2 { lo[k] = lo[k].min(p[k]); hi[k] = hi[k].max(p[k]); } }
    (lo, hi)
}

/// Terrain triangles are merged if their footprints touch and their heights agree to this
/// much. Displacements are a continuous surface, so neighbours are the same walkable ground
/// even where the slope changes between them.
const DISP_Z_TOL: f64 = 24.0;

pub fn merge_patches(faces: &[Face]) -> Vec<Patch> {
    use std::collections::HashMap;
    let mut groups: HashMap<(i64, i64, i64, i64), Vec<u32>> = HashMap::new();
    for (i, f) in faces.iter().enumerate() {
        // Displacements get ONE bucket per standable class rather than one per plane.
        //
        // Terrain is a curved mesh: every triangle has a slightly different normal, so plane
        // bucketing puts each in its own group, nothing ever merges, and a lone 24u terrain
        // triangle gets classified as a narrow ledge. On cs_italy that produced 616 bogus
        // "pixelwalks" out of 684 — the open ground you walk across every round.
        let key = if f.is_disp {
            let standable = f.n[2] >= STANDABLE_NORMAL_Z;
            (i64::MIN, if standable { 1 } else { 0 }, 0, 0)
        } else {
            ((f.n[0] / PLANE_EPS_N).round() as i64, (f.n[1] / PLANE_EPS_N).round() as i64,
             (f.n[2] / PLANE_EPS_N).round() as i64, (f.d / PLANE_EPS_D).round() as i64)
        };
        groups.entry(key).or_default().push(i as u32);
    }

    let mut out = Vec::new();
    let mut cellmap: HashMap<(i64, i64), Vec<u32>> = HashMap::new();
    for (_, members) in groups {
        // spatial hash within the group so merging a 400-face floor isn't quadratic
        const CELL: f64 = 128.0;
        cellmap.clear();
        let bounds: Vec<([f64; 2], [f64; 2])> = members.iter().map(|&i| face_bounds2(&faces[i as usize])).collect();
        for (li, (lo, hi)) in bounds.iter().enumerate() {
            let (x0, x1) = ((lo[0] / CELL).floor() as i64, (hi[0] / CELL).floor() as i64);
            let (y0, y1) = ((lo[1] / CELL).floor() as i64, (hi[1] / CELL).floor() as i64);
            for cy in y0..=y1 { for cx in x0..=x1 { cellmap.entry((cx, cy)).or_default().push(li as u32); } }
        }

        let mut dsu = Dsu::new(members.len());
        for cand in cellmap.values() {
            for a in 0..cand.len() {
                for b in a + 1..cand.len() {
                    let (ia, ib) = (cand[a] as usize, cand[b] as usize);
                    let (la, ha) = bounds[ia];
                    let (lb, hb) = bounds[ib];
                    let touch = la[0] <= hb[0] + TOUCH_EPS && lb[0] <= ha[0] + TOUCH_EPS
                             && la[1] <= hb[1] + TOUCH_EPS && lb[1] <= ha[1] + TOUCH_EPS;
                    if !touch { continue; }
                    // Coplanar faces share a plane, so a footprint overlap is enough. Terrain
                    // does not: without a height check a hillside would weld to the valley
                    // floor it overhangs, merging separate ground into one patch.
                    let fa = &faces[members[ia] as usize];
                    let fb = &faces[members[ib] as usize];
                    if fa.is_disp || fb.is_disp {
                        let za = fa.centroid()[2];
                        let zb = fb.centroid()[2];
                        if (za - zb).abs() > DISP_Z_TOL { continue; }
                    }
                    dsu.union(cand[a], cand[b]);
                }
            }
        }

        let mut clusters: HashMap<u32, Vec<u32>> = HashMap::new();
        for li in 0..members.len() { let r = dsu.find(li as u32); clusters.entry(r).or_default().push(li as u32); }

        for (_, local) in clusters {
            let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
            let (mut area, mut cw) = (0.0f64, [0.0f64; 3]);
            let (mut cor, mut cand_) = (0i32, !0i32);
            let mut is_disp = false;
            let mut nrm = [0.0f64; 3];
            let mut ids = Vec::with_capacity(local.len());
            for &li in &local {
                let gi = members[li as usize];
                let f = &faces[gi as usize];
                ids.push(gi);
                let (l, h) = bounds[li as usize];
                for k in 0..2 { lo[k] = lo[k].min(l[k]); hi[k] = hi[k].max(h[k]); }
                let a = f.area();
                area += a;
                let c = f.centroid();
                for k in 0..3 { cw[k] += c[k] * a; }
                cor |= f.contents; cand_ &= f.contents;
                is_disp |= f.is_disp;
                nrm = f.n;
            }
            let centroid = if area > 0.0 { [cw[0] / area, cw[1] / area, cw[2] / area] }
                           else { faces[ids[0] as usize].centroid() };
            out.push(Patch { faces: ids, n: nrm, contents_or: cor, contents_and: cand_,
                is_disp, area, centroid, min: lo, max: hi });
        }
    }
    out
}

pub fn classify_patch(p: &Patch) -> Kind {
    if p.n[2] >= STANDABLE_NORMAL_Z {
        let w = p.min_width();
        // A pixel surf is a POINT you perch on, so it has to be small both ways. Testing only
        // the narrow side made every strip of moulding qualify: on de_seaside all eight
        // "pixelsurfs" were one 1u lip, 230u long, repeated at z=207 around the map.
        if w < PIXEL_SLIVER && p.max_width() <= PERCH_MAX_LEN { Kind::PixelSurf }
        else if w < HULL_WIDTH { Kind::PixelWalk }
        else { Kind::Ground }
    } else {
        Kind::Surf
    }
}

// ---------------------------------------------------------------- hull clearance
//
// An upward-facing surface is not a spot unless a player actually FITS on it. Without this
// test, every ledge with a wall or ceiling right above it gets reported — which is most of
// them, and is why a raw scan of cs_italy returns thousands of "pixelwalks" that you could
// never stand on. Reporting a candidate the user has to go and disprove in-game is worse
// than not reporting it.
//
// The test is the engine's own: place the player AABB with its feet on the surface and see
// if it intersects anything player-solid. Standing is 32x32x72; if that fails, try the 54u
// ducked hull, because plenty of real spots are crouch-only.

/// Start the hull just above the surface — the surface is itself the top of a brush, so a
/// hull resting exactly on it always "touches" the brush it is standing on.
const FOOT_EPS: f64 = 0.25;
/// Ignore grazing contact; brush faces meet at shared planes and f32 coordinates wobble.
const OVERLAP_EPS: f64 = 0.5;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fit { Standing, DuckOnly, Blocked }

/// Uniform grid over solid AABBs, so a clearance test doesn't scan every brush on the map.
struct SolidGrid { cell: f64, min: [f64; 2], w: usize, h: usize, buckets: Vec<Vec<u32>> }

impl SolidGrid {
    fn build(solids: &[crate::collide::Aabb], cell: f64) -> SolidGrid {
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for s in solids {
            for k in 0..2 { lo[k] = lo[k].min(s.min[k]); hi[k] = hi[k].max(s.max[k]); }
        }
        if solids.is_empty() { lo = [0.0; 2]; hi = [0.0; 2]; }
        let w = (((hi[0] - lo[0]) / cell).ceil() as usize + 1).max(1);
        let h = (((hi[1] - lo[1]) / cell).ceil() as usize + 1).max(1);
        let mut g = SolidGrid { cell, min: lo, w, h, buckets: vec![Vec::new(); w * h] };
        for (i, s) in solids.iter().enumerate() {
            let (x0, x1) = (g.cx(s.min[0]), g.cx(s.max[0]));
            let (y0, y1) = (g.cy(s.min[1]), g.cy(s.max[1]));
            for cy in y0..=y1 { for cx in x0..=x1 { g.buckets[cy * g.w + cx].push(i as u32); } }
        }
        g
    }
    fn cx(&self, x: f64) -> usize { (((x - self.min[0]) / self.cell).max(0.0) as usize).min(self.w - 1) }
    fn cy(&self, y: f64) -> usize { (((y - self.min[1]) / self.cell).max(0.0) as usize).min(self.h - 1) }

    /// Does a 32x32x`height` box with its feet at `z` clear everything player-solid?
    fn clear(&self, solids: &[crate::collide::Aabb], x: f64, y: f64, z: f64, height: f64) -> bool {
        let half = HULL_WIDTH / 2.0;
        let (lo, hi) = ([x - half, y - half, z + FOOT_EPS], [x + half, y + half, z + height]);
        let (x0, x1) = (self.cx(lo[0]), self.cx(hi[0]));
        let (y0, y1) = (self.cy(lo[1]), self.cy(hi[1]));
        for cy in y0..=y1 {
            for cx in x0..=x1 {
                for &i in &self.buckets[cy * self.w + cx] {
                    let s = &solids[i as usize];
                    let overlaps = (0..3).all(|k| {
                        lo[k] < s.max[k] - OVERLAP_EPS && s.min[k] < hi[k] - OVERLAP_EPS
                    });
                    if overlaps { return false; }
                }
            }
        }
        true
    }

    fn fit(&self, solids: &[crate::collide::Aabb], x: f64, y: f64, z: f64) -> Fit {
        if self.clear(solids, x, y, z, HULL_H_STAND) { Fit::Standing }
        else if self.clear(solids, x, y, z, HULL_H_DUCK) { Fit::DuckOnly }
        else { Fit::Blocked }
    }
}

/// Uniform grid over the XY plane so neighbour lookups aren't O(n^2).
struct Grid { cell: f64, min: [f64; 2], w: usize, h: usize, buckets: Vec<Vec<u32>> }

impl Grid {
    fn build(pts: &[P3], cell: f64) -> Grid {
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for p in pts { for k in 0..2 { lo[k] = lo[k].min(p[k]); hi[k] = hi[k].max(p[k]); } }
        if pts.is_empty() { lo = [0.0; 2]; hi = [0.0; 2]; }
        let w = (((hi[0] - lo[0]) / cell).ceil() as usize + 1).max(1);
        let h = (((hi[1] - lo[1]) / cell).ceil() as usize + 1).max(1);
        let mut g = Grid { cell, min: lo, w, h, buckets: vec![Vec::new(); w * h] };
        for (i, p) in pts.iter().enumerate() {
            let idx = g.index(p[0], p[1]);
            g.buckets[idx].push(i as u32);
        }
        g
    }
    fn index(&self, x: f64, y: f64) -> usize {
        let cx = (((x - self.min[0]) / self.cell) as usize).min(self.w - 1);
        let cy = (((y - self.min[1]) / self.cell) as usize).min(self.h - 1);
        cy * self.w + cx
    }
    /// Every point within `radius` of (x,y), as indices.
    fn near(&self, x: f64, y: f64, radius: f64, out: &mut Vec<u32>) {
        out.clear();
        let r = (radius / self.cell).ceil() as isize + 1;
        let cx = (((x - self.min[0]) / self.cell) as isize).clamp(0, self.w as isize - 1);
        let cy = (((y - self.min[1]) / self.cell) as isize).clamp(0, self.h as isize - 1);
        for gy in (cy - r).max(0)..=(cy + r).min(self.h as isize - 1) {
            for gx in (cx - r).max(0)..=(cx + r).min(self.w as isize - 1) {
                out.extend_from_slice(&self.buckets[gy as usize * self.w + gx as usize]);
            }
        }
    }
}

pub struct ScanOptions {
    /// Include ordinary floor in the output. Off by default — it is most of the map.
    pub include_ground: bool,
    pub include_surf: bool,
    /// Include narrow ledges flush with walkable floor (window sills, step edges, kerbs).
    /// Off by default: they are standable but you can just walk onto them, so they are not
    /// spots, and there are thousands per map.
    pub include_trim: bool,
    /// Drop out-of-bounds candidates below this height above reachable ground; most tiny
    /// gaps are compiler noise or a lip on a wall, not a spot worth walking to.
    pub min_oob_height: f64,
}

impl Default for ScanOptions {
    fn default() -> Self {
        ScanOptions { include_ground: false, include_surf: true, include_trim: false,
            min_oob_height: 40.0 }
    }
}

pub struct ScanResult {
    pub spots: Vec<Spot>,
    /// Candidates rejected because no player hull fits — reported so the filtering is
    /// auditable rather than silently swallowing surfaces.
    pub blocked: usize,
    /// Slivers rejected as compiler seams rather than perches.
    pub seams: usize,
}

pub fn scan(geo: &Geometry, opts: &ScanOptions) -> ScanResult {
    // Merge first: raw BSP faces are fragments, patches are surfaces. Classifying fragments
    // turns one wall top into hundreds of "ledges".
    let patches = merge_patches(&geo.faces);
    let n = patches.len();
    let centroids: Vec<P3> = patches.iter().map(|p| p.centroid).collect();
    let kinds: Vec<Kind> = patches.iter().map(classify_patch).collect();

    // ---- reachability flood fill from the spawns
    let grid = Grid::build(&centroids, 128.0);
    let mut reachable = vec![false; n];
    let mut queue: Vec<u32> = Vec::new();
    let mut scratch: Vec<u32> = Vec::new();

    // seed: standable faces under or near a spawn point
    for s in &geo.spawns {
        grid.near(s[0], s[1], 64.0, &mut scratch);
        let mut best: Option<(usize, f64)> = None;
        for &i in &scratch {
            let i = i as usize;
            if kinds[i] == Kind::Surf { continue; }
            let dz = s[2] - centroids[i][2];
            // spawns float a little above the floor they belong to
            if !(-8.0..=96.0).contains(&dz) { continue; }
            let d = (centroids[i][0] - s[0]).powi(2) + (centroids[i][1] - s[1]).powi(2);
            if best.map_or(true, |(_, bd)| d < bd) { best = Some((i, d)); }
        }
        if let Some((i, _)) = best {
            if !reachable[i] { reachable[i] = true; queue.push(i as u32); }
        }
    }

    let max_gap = jump_gap(0.0);
    while let Some(cur) = queue.pop() {
        let ci = cur as usize;
        let c = centroids[ci];
        grid.near(c[0], c[1], max_gap, &mut scratch);
        for &j in &scratch {
            let j = j as usize;
            if reachable[j] { continue; }
            let t = centroids[j];
            let dz = t[2] - c[2];
            if dz > MAX_RISE { continue; }
            // dropping down is free, so only the rise is limited
            let horiz = ((t[0] - c[0]).powi(2) + (t[1] - c[1]).powi(2)).sqrt();
            let allowed = if dz < 0.0 { max_gap } else { jump_gap(dz) };
            if horiz > allowed { continue; }
            reachable[j] = true;
            queue.push(j as u32);
        }
    }

    // ---- seam rejection
    //
    // A 1-unit sliver flush against a bigger surface at the same height is not a perch, it is
    // a compiler seam: two brushes whose tops differ by a hair bucket into separate patches
    // and the thinner one looks like a pixel surf. On de_seaside this produced eight
    // "pixelsurfs" all at exactly z=207.0 and width 1.0u, scattered across the map — one
    // piece of trim, counted eight times.
    //
    // What makes a sliver real is air beside it. So: if a proper surface sits within a couple
    // of units horizontally at essentially the same height, this is a seam in that surface.
    const SEAM_XY: f64 = 20.0;      // how close the neighbour has to be
    const SEAM_Z: f64 = 4.0;        // how closely their heights must agree
    const SEAM_REAL_WIDTH: f64 = 8.0;   // a neighbour this wide is a genuine surface
    let mut seam = vec![false; n];
    for i in 0..n {
        if kinds[i] != Kind::PixelSurf { continue; }
        let c = centroids[i];
        grid.near(c[0], c[1], SEAM_XY, &mut scratch);
        for &j in &scratch {
            let j = j as usize;
            if j == i { continue; }
            if patches[j].n[2] < STANDABLE_NORMAL_Z { continue; }
            if patches[j].min_width() < SEAM_REAL_WIDTH { continue; }
            if (centroids[j][2] - c[2]).abs() > SEAM_Z { continue; }
            let d2 = (centroids[j][0] - c[0]).powi(2) + (centroids[j][1] - c[1]).powi(2);
            if d2 <= SEAM_XY * SEAM_XY { seam[i] = true; break; }
        }
    }

    // ---- isolation test
    //
    // A narrow ledge flush with ordinary floor is not a spot, it is trim: window sills, step
    // nosings, doorframes and kerbs are all 16u-wide standable strips, and cs_italy has
    // thousands. What makes a ledge worth knowing about is that you cannot simply walk onto
    // it — so a pixelwalk/pixelsurf touching Ground at step height is dropped.
    let mut trim = vec![false; n];
    for i in 0..n {
        if !matches!(kinds[i], Kind::PixelWalk | Kind::PixelSurf) { continue; }
        let c = centroids[i];
        grid.near(c[0], c[1], WALK_GAP, &mut scratch);
        for &j in &scratch {
            let j = j as usize;
            if j == i || kinds[j] != Kind::Ground { continue; }
            if (centroids[j][2] - c[2]).abs() > STEP_SIZE { continue; }
            let d2 = (centroids[j][0] - c[0]).powi(2) + (centroids[j][1] - c[1]).powi(2);
            if d2 <= WALK_GAP * WALK_GAP { trim[i] = true; break; }
        }
    }

    // ---- highest reachable surface beneath each face, for ranking the out-of-bounds ones
    let solid_grid = SolidGrid::build(&geo.solids, 256.0);
    let (mut blocked, mut seams) = (0usize, 0usize);
    let mut out = Vec::new();
    let mut below: Vec<u32> = Vec::new();
    for i in 0..n {
        let kind = kinds[i];
        if kind == Kind::Ground && !opts.include_ground { continue; }
        if kind == Kind::Surf && (!opts.include_surf || patches[i].area < MIN_SURF_AREA) { continue; }
        if trim[i] && !opts.include_trim { continue; }
        if seam[i] { seams += 1; continue; }

        // Can a player actually stand here? A ledge with a wall or ceiling over it is not a
        // spot no matter how inviting the surface looks. Surf ramps are exempt: you ride
        // those moving, not from a standing hull position.
        let fit = if kind == Kind::Surf { Fit::Standing }
                  else { solid_grid.fit(&geo.solids, centroids[i][0], centroids[i][1], centroids[i][2]) };
        if fit == Fit::Blocked { blocked += 1; continue; }

        let f = &patches[i];
        let c = centroids[i];
        let mut height_above = 0.0;
        if !reachable[i] {
            grid.near(c[0], c[1], 96.0, &mut below);
            let mut best = f64::MIN;
            for &j in &below {
                let j = j as usize;
                if !reachable[j] || centroids[j][2] >= c[2] { continue; }
                best = best.max(centroids[j][2]);
            }
            height_above = if best > f64::MIN { c[2] - best } else { f64::INFINITY };
        }
        if !reachable[i] && height_above.is_finite() && height_above < opts.min_oob_height { continue; }

        // clip only if EVERY face in the patch is clip — a patch mixing clip and world
        // geometry is really world geometry with a clip lip, not an invisible ledge
        let is_clip = f.contents_and & CONTENTS_PLAYERCLIP != 0 && f.contents_or & CONTENTS_SOLID == 0;
        let oob_class = if reachable[i] { None } else if kind == Kind::Surf {
            Some("surf-ramp")
        } else if is_clip {
            Some("clip-gap")
        } else {
            // real world geometry nobody clipped off — the "mapmaker never thought anyone
            // would stand here" case
            Some("unclipped-geometry")
        };

        // ways onto it: only meaningful for spots a player could line up under
        let entries = if reachable[i] || height_above.is_finite() {
            jumptable::solutions(c[2], c[2] - 200.0, c[2] + 8.0, None)
                .into_iter().take(6)
                .map(|s| Entry { label: s.label, players: s.players, jump: s.jump,
                    crouch: s.crouch, stand_eye: s.stand_eye, tickrates: s.tickrates })
                .collect()
        } else { Vec::new() };

        out.push(Spot {
            kind,
            pos: c,
            rect: [f.min[0], f.min[1], f.max[0], f.max[1]],
            eye_z: c[2] + EYE_STAND,
            width: f.min_width(),
            area: f.area,
            slope_deg: f.n[2].clamp(-1.0, 1.0).acos().to_degrees(),
            is_clip,
            is_disp: f.is_disp,
            duck_only: fit == Fit::DuckOnly,
            reachable: reachable[i],
            height_above_reachable: if height_above.is_finite() { height_above } else { -1.0 },
            oob_class,
            entries,
        });
    }

    // most interesting first: out-of-bounds by how high, then the narrowest ledges
    out.sort_by(|a, b| {
        b.reachable.cmp(&a.reachable).reverse()
            .then(b.height_above_reachable.partial_cmp(&a.height_above_reachable).unwrap())
            .then(a.width.partial_cmp(&b.width).unwrap())
    });
    ScanResult { spots: out, blocked, seams }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collide::Face;

    fn flat(w: f64, h: f64, z: f64) -> Face {
        Face { n: [0.0, 0.0, 1.0], d: z, contents: CONTENTS_SOLID, is_disp: false,
            poly: vec![[0.0, 0.0, z], [w, 0.0, z], [w, h, z], [0.0, h, z]] }
    }

    #[test]
    fn classification_boundaries() {
        assert_eq!(classify(&flat(64.0, 64.0, 0.0)), Kind::Ground, "a hull-wide slab is floor");
        assert_eq!(classify(&flat(16.0, 200.0, 0.0)), Kind::PixelWalk, "a 16u ledge is walkable");
        assert_eq!(classify(&flat(31.9, 200.0, 0.0)), Kind::PixelWalk, "just under a hull wide");
        assert_eq!(classify(&flat(32.1, 200.0, 0.0)), Kind::Ground, "just over a hull wide");
        assert_eq!(classify(&flat(1.0, 200.0, 0.0)), Kind::PixelSurf, "a 1u sliver is a pixelsurf");

        let ramp = Face { n: [0.6, 0.0, 0.8], d: 0.0, contents: CONTENTS_SOLID, is_disp: false,
            poly: vec![[0.0, 0.0, 0.0], [64.0, 0.0, 0.0], [64.0, 64.0, 0.0], [0.0, 64.0, 0.0]] };
        assert_eq!(classify(&ramp), Kind::Ground, "normal.z 0.8 is still standable");
        let steep = Face { n: [0.8, 0.0, 0.6], ..ramp.clone() };
        assert_eq!(classify(&steep), Kind::Surf, "normal.z 0.6 is a surf ramp");
    }

    #[test]
    fn jump_gap_shrinks_as_you_climb() {
        let flat_reach = jump_gap(0.0);
        // a real CS:GO running jump clears roughly 250u of flat ground
        assert!(flat_reach > 220.0 && flat_reach < 300.0, "flat jump reach {flat_reach} u");
        assert!(jump_gap(50.0) < flat_reach, "less horizontal reach when climbing");
        assert!(jump_gap(4.0) > 200.0, "a small rise still allows a full jump across");
        assert_eq!(jump_gap(200.0), 0.0, "cannot reach above the crouch-jump apex");
        // dropping down is at least as easy as flat
        assert!(jump_gap(-100.0) >= flat_reach - 1e-9, "dropping is never harder than flat");
    }
}
