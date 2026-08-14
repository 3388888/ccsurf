//! Collision geometry out of a Source-1 .bsp, at the precision a pixel surf needs.
//!
//! The engine collides the player hull against BRUSH PLANES, not rendered faces, and the
//! perch you are hunting for is very often exactly where the two disagree. So each brush is
//! rebuilt as a convex polyhedron from its own side planes (clip a huge seed quad against
//! every other plane) and kept at full float precision — no quantisation anywhere.
//!
//! Only polygons with an upward component are kept: a ceiling or a perfectly vertical wall
//! holds nobody, and keeping them would multiply the output for no gain. Headroom and
//! reachability queries use the compact per-brush AABB list in `solids` instead.
//!
//! The threshold has to be tiny. It was 0.01 and that was too coarse: a wall surf rides a
//! near-vertical face whose normal barely tilts up, so the known de_vertigo_2019 surf at
//! roughly (-2203, 185, 11776) was discarded before it ever reached classification. Anything
//! with a positive normal.z can in principle be ridden.
//!
//! NOT HANDLED: static prop collision (.phy inside the VPKs). Prop-mounted spots are
//! invisible here — `Geometry::props_scanned` is always false so callers can say so out loud
//! rather than implying a complete scan.

use crate::bsp::{self, lump, Bsp};
use crate::consts::*;
use std::path::Path;

const PLANE_SIZE: usize = 20;      // normal[3] f32, dist f32, type i32
const BRUSH_SIZE: usize = 12;      // firstside i32, numsides i32, contents i32
/// dbrushside_t: planenum u16, texinfo i16, dispinfo i16, then TWO bytes.
/// The old Source layout ended in a single `short bevel`; CS:GO splits that into
/// `byte bevel; byte thin;`. Reading it as a short makes every thin side (thin=1, i.e. 256)
/// look like a bevel — on de_dust2 that silently discards 16133 of 50124 sides and leaves
/// brushes with too few planes to close, which leaks unclipped seed polygons into the map.
const BRUSHSIDE_SIZE: usize = 8;
const SIDE_BEVEL_OFS: usize = 6;
const DISPINFO_SIZE: usize = 176;
const FACE_SIZE: usize = 56;

/// Source's world extent; the seed polygon must comfortably exceed it.
const MAX_COORD: f64 = 32768.0;
const CLIP_EPS: f64 = 0.01;
/// Smallest upward tilt worth keeping. Exactly-vertical faces (normal.z == 0) hold nobody, so
/// they still go, but anything above this can be surfed.
const MIN_UP_Z: f64 = 0.0005;

pub type P3 = [f64; 3];

#[derive(Clone, Debug)]
pub struct Face {
    pub n: P3,
    pub d: f64,
    pub poly: Vec<P3>,
    pub contents: i32,
    pub is_disp: bool,
}

impl Face {
    pub fn area(&self) -> f64 { poly_area(&self.poly, self.n) }
    pub fn centroid(&self) -> P3 {
        let mut c = [0.0; 3];
        for p in &self.poly { for i in 0..3 { c[i] += p[i]; } }
        let n = self.poly.len().max(1) as f64;
        [c[0] / n, c[1] / n, c[2] / n]
    }
    /// Narrowest horizontal extent — the number that separates a walkable ledge from a sliver.
    pub fn min_width(&self) -> f64 {
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for p in &self.poly {
            for i in 0..2 { lo[i] = lo[i].min(p[i]); hi[i] = hi[i].max(p[i]); }
        }
        (hi[0] - lo[0]).min(hi[1] - lo[1])
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Aabb {
    pub min: P3, pub max: P3, pub contents: i32,
}

#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub planes: usize, pub brushes: usize, pub brushes_solid: usize, pub brushes_kept: usize,
    pub up_faces: usize, pub disp_faces: usize,
    pub degenerate: usize, pub bevel_skipped: usize, pub bevel_fallback: usize,
    pub unbounded: usize, pub moving_brush_ents: usize,
    /// Props turned into standable top faces, and those whose .mdl hull could not be read.
    pub prop_faces: usize, pub props_placed: usize, pub props_no_hull: usize,
}

pub struct Geometry {
    pub name: String,
    pub version: i32,
    pub faces: Vec<Face>,
    pub solids: Vec<Aabb>,
    pub spawns: Vec<P3>,
    pub bounds: (P3, P3),
    /// Spawn-derived volume. NOT the same as `bounds`: a map's outer skybox shell is real
    /// brush geometry spanning +/-16376, so `bounds` says de_log is 32752u tall when its
    /// actual geometry lives in -704..992. Anything reasoning about "the playable area" must
    /// use this, never `bounds`.
    pub play: Option<(P3, P3)>,
    pub stats: Stats,
    /// True once prop boxes have been folded in. They are bounding-box approximations of the
    /// real .phy hulls, not the hulls themselves — scans should say so.
    pub props_scanned: bool,
}

// ---------------------------------------------------------------- polygon construction

/// A quad on the plane, large enough to contain any real brush face before clipping.
/// Quake's BaseWindingForPlane, wound so the polygon's own normal equals n.
///
/// The normal is re-normalised first and it matters: the seed extends MAX_COORD from the
/// origin, so a normal off unit length by 1 part in 10^4 leaves the basis non-perpendicular
/// and throws the far corners ~6 units off the plane. BSP normals are f32 and not exactly unit.
pub fn base_winding(n: P3, d: f64) -> Option<Vec<P3>> {
    let nl = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if nl < 1e-9 { return None; }
    let (nx, ny, nz, d) = (n[0] / nl, n[1] / nl, n[2] / nl, d / nl);

    let (ax, ay, az) = (nx.abs(), ny.abs(), nz.abs());
    // pick a helper axis not parallel to n
    let (mut ux, mut uy, mut uz) = if az >= ax && az >= ay { (1.0, 0.0, 0.0) } else { (0.0, 0.0, 1.0) };
    let dot = ux * nx + uy * ny + uz * nz;
    ux -= nx * dot; uy -= ny * dot; uz -= nz * dot;
    let ul = (ux * ux + uy * uy + uz * uz).sqrt();
    if ul < 1e-9 { return None; }
    ux /= ul; uy /= ul; uz /= ul;

    // right = n x up, giving (up, right, n) a right-handed orientation so the vertex order
    // below winds counter-clockwise viewed from the +n side
    let (rx, ry, rz) = (ny * uz - nz * uy, nz * ux - nx * uz, nx * uy - ny * ux);
    let (ox, oy, oz) = (nx * d, ny * d, nz * d);
    let m = MAX_COORD;
    Some(vec![
        [ox - rx * m + ux * m, oy - ry * m + uy * m, oz - rz * m + uz * m],
        [ox + rx * m + ux * m, oy + ry * m + uy * m, oz + rz * m + uz * m],
        [ox + rx * m - ux * m, oy + ry * m - uy * m, oz + rz * m - uz * m],
        [ox - rx * m - ux * m, oy - ry * m - uy * m, oz - rz * m - uz * m],
    ])
}

/// Sutherland-Hodgman: keep the part of `poly` inside the half-space dot(n,p) <= d.
pub fn clip_to_plane(poly: &[P3], n: P3, d: f64) -> Vec<P3> {
    let len = poly.len();
    if len == 0 { return Vec::new(); }
    let mut dist = Vec::with_capacity(len);
    let (mut any_front, mut any_back) = (false, false);
    for p in poly {
        let dd = p[0] * n[0] + p[1] * n[1] + p[2] * n[2] - d;
        if dd > CLIP_EPS { any_front = true; } else if dd < -CLIP_EPS { any_back = true; }
        dist.push(dd);
    }
    if !any_front { return poly.to_vec(); }
    if !any_back { return Vec::new(); }

    let mut out = Vec::with_capacity(len + 2);
    for i in 0..len {
        let j = (i + 1) % len;
        let (p, q, dp, dq) = (poly[i], poly[j], dist[i], dist[j]);
        if dp <= CLIP_EPS { out.push(p); }
        if (dp > CLIP_EPS && dq < -CLIP_EPS) || (dp < -CLIP_EPS && dq > CLIP_EPS) {
            let t = dp / (dp - dq);
            out.push([p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t, p[2] + (q[2] - p[2]) * t]);
        }
    }
    if out.len() >= 3 { out } else { Vec::new() }
}

pub fn poly_area(poly: &[P3], n: P3) -> f64 {
    let mut c = [0.0f64; 3];
    for i in 1..poly.len().saturating_sub(1) {
        let (a, b, d) = (poly[0], poly[i], poly[i + 1]);
        let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let e2 = [d[0] - a[0], d[1] - a[1], d[2] - a[2]];
        c[0] += e1[1] * e2[2] - e1[2] * e2[1];
        c[1] += e1[2] * e2[0] - e1[0] * e2[2];
        c[2] += e1[0] * e2[1] - e1[1] * e2[0];
    }
    (c[0] * n[0] + c[1] * n[1] + c[2] * n[2]).abs() / 2.0
}

// ---------------------------------------------------------------- extraction

pub fn extract(path: &Path) -> Result<Geometry, String> {
    // drop slivers below half a unit square — below this it is compiler noise, not a perch
    const MIN_AREA: f64 = 0.25;

    let mut bsp = Bsp::open(path)?;
    let version = bsp.version;
    let planes_buf = bsp.read(lump::PLANES)?;
    let brushes_buf = bsp.read(lump::BRUSHES)?;
    let sides_buf = bsp.read(lump::BRUSHSIDES)?;
    let models_buf = bsp.read(lump::MODELS)?;
    let ents = bsp::parse_entities(&bsp.read(lump::ENTITIES)?);

    if planes_buf.is_empty() || brushes_buf.is_empty() { return Err("no brush lumps".into()); }

    let n_planes = planes_buf.len() / PLANE_SIZE;
    let n_brush = brushes_buf.len() / BRUSH_SIZE;
    let n_sides = sides_buf.len() / BRUSHSIDE_SIZE;
    let n_models = models_buf.len() / 48;

    // plane lump as flat arrays — this is the hot data
    let mut pn = vec![[0.0f64; 3]; n_planes];
    let mut pd = vec![0.0f64; n_planes];
    for i in 0..n_planes {
        let o = i * PLANE_SIZE;
        pn[i] = [bsp::f32le(&planes_buf, o) as f64, bsp::f32le(&planes_buf, o + 4) as f64,
                 bsp::f32le(&planes_buf, o + 8) as f64];
        pd[i] = bsp::f32le(&planes_buf, o + 12) as f64;
    }

    let mut stats = Stats { planes: n_planes, brushes: n_brush, ..Default::default() };

    // KNOWN GAP: brush entities with a non-zero origin (doors, elevators) are emitted at their
    // compiled position, not their in-game one. dmodel_t indexes FACES, but brushes are tied
    // to models only through the BSP tree, which is a much bigger walk. Counted, not assumed zero.
    for e in &ents {
        let (Some(m), Some(o)) = (e.get("model"), e.get("origin").and_then(|s| bsp::vec3(s))) else { continue };
        if !m.starts_with('*') { continue; }
        let idx: usize = m[1..].parse().unwrap_or(0);
        if idx > 0 && idx < n_models && (o[0] != 0.0 || o[1] != 0.0 || o[2] != 0.0) {
            stats.moving_brush_ents += 1;
        }
    }

    let mut spawns = Vec::new();
    for e in &ents {
        let Some(c) = e.get("classname") else { continue };
        let is_spawn = matches!(c.as_str(),
            "info_player_terrorist" | "info_player_counterterrorist" | "info_player_start"
            | "info_player_deathmatch" | "info_deathmatch_spawn");
        if is_spawn { if let Some(o) = e.get("origin").and_then(|s| bsp::vec3(s)) { spawns.push(o); } }
    }

    let mut faces: Vec<Face> = Vec::new();
    let mut solids: Vec<Aabb> = Vec::new();
    let (mut bmin, mut bmax) = ([f64::MAX; 3], [f64::MIN; 3]);
    let mut side_planes: Vec<usize> = Vec::with_capacity(64);

    for b in 0..n_brush {
        let bo = b * BRUSH_SIZE;
        let contents = bsp::i32le(&brushes_buf, bo + 8);
        if contents & CONTENTS_PLAYER_SOLID == 0 { continue; }
        stats.brushes_solid += 1;
        let first = bsp::i32le(&brushes_buf, bo) as usize;
        let num = bsp::i32le(&brushes_buf, bo + 4) as usize;
        if num < 4 { continue; }

        // Bevel sides are axis-aligned padding the compiler adds for the expanded-hull sweep.
        // They are not surfaces of the brush; including them shaves every polygon's corners.
        side_planes.clear();
        for s in first..(first + num).min(n_sides) {
            let so = s * BRUSHSIDE_SIZE;
            if sides_buf[so + SIDE_BEVEL_OFS] != 0 { stats.bevel_skipped += 1; continue; }
            let p = bsp::u16le(&sides_buf, so) as usize;
            if p < n_planes { side_planes.push(p); }
        }
        // Fewer than four real planes cannot bound a volume. Rather than drop the brush, fall
        // back to every side: bevels only touch the brush, so they clip nothing the real
        // planes wouldn't.
        if side_planes.len() < 4 {
            side_planes.clear();
            for s in first..(first + num).min(n_sides) {
                let p = bsp::u16le(&sides_buf, s * BRUSHSIDE_SIZE) as usize;
                if p < n_planes { side_planes.push(p); }
            }
            if side_planes.len() < 4 { continue; }
            stats.bevel_fallback += 1;
        }
        stats.brushes_kept += 1;

        let (mut amin, mut amax) = ([f64::MAX; 3], [f64::MIN; 3]);
        for i in 0..side_planes.len() {
            let pi = side_planes[i];
            let Some(mut poly) = base_winding(pn[pi], pd[pi]) else { stats.degenerate += 1; continue };
            for j in 0..side_planes.len() {
                if j == i || poly.is_empty() { continue; }
                let pj = side_planes[j];
                poly = clip_to_plane(&poly, pn[pj], pd[pj]);
            }
            if poly.len() < 3 { continue; }

            // A brush whose planes don't actually close leaves part of the seed intact. Such a
            // polygon is meaningless and would wreck the map bounds, so drop and count it.
            if poly.iter().any(|p| p.iter().any(|c| c.abs() >= MAX_COORD - 1.0)) {
                stats.unbounded += 1;
                continue;
            }
            for p in &poly {
                for k in 0..3 { amin[k] = amin[k].min(p[k]); amax[k] = amax[k].max(p[k]); }
            }
            if pn[pi][2] <= MIN_UP_Z { continue; }      // needs some upward tilt to hold anyone
            if poly_area(&poly, pn[pi]) < MIN_AREA { stats.degenerate += 1; continue; }
            faces.push(Face { n: pn[pi], d: pd[pi], poly, contents, is_disp: false });
        }

        if amin[0] < f64::MAX {
            solids.push(Aabb { min: amin, max: amax, contents });
            for k in 0..3 { bmin[k] = bmin[k].min(amin[k]); bmax[k] = bmax[k].max(amax[k]); }
        }
    }

    // ---- static props
    //
    // Props are the geometry a brush-only scanner cannot see, and on prop-heavy maps that is
    // most of the interesting surfaces. The true hull is a convex decomposition in a .phy
    // file; what we use is the model's bounding box from its .mdl header. For the beams,
    // slabs and crates people actually stand on that is very close, and for a curved prop it
    // over-estimates — which is the right way to be wrong, since a slightly-off spot you can
    // go and check beats a spot that was never mentioned.
    let mut prop_faces = 0usize;
    let mut props_placed = 0usize;
    let mut props_no_hull = 0usize;
    if let Some(maps_dir) = path.parent() {
        let archives: Vec<crate::vpk::Vpk> = crate::vpk::find_archives(maps_dir)
            .into_iter().filter_map(|p| crate::vpk::Vpk::open(&p).ok()).collect();
        if !archives.is_empty() {
            if let Ok((props, _)) = crate::props::extract(&mut bsp) {
                let mut hull_cache: std::collections::HashMap<String, Option<([f64;3],[f64;3])>> =
                    std::collections::HashMap::new();
                for pr in &props {
                    if !pr.is_solid() { continue; }
                    let hull = hull_cache.entry(pr.model.clone())
                        .or_insert_with(|| crate::props::model_hull(&archives, &pr.model));
                    let Some((lo, hi)) = *hull else { props_no_hull += 1; continue };
                    let (wlo, whi) = crate::props::world_box(pr, lo, hi);
                    props_placed += 1;
                    solids.push(Aabb { min: wlo, max: whi, contents: CONTENTS_SOLID });
                    for k in 0..3 { bmin[k] = bmin[k].min(wlo[k]); bmax[k] = bmax[k].max(whi[k]); }
                    // only the top face can be stood on
                    faces.push(Face {
                        n: [0.0, 0.0, 1.0], d: whi[2], contents: CONTENTS_SOLID, is_disp: false,
                        poly: vec![[wlo[0], wlo[1], whi[2]], [whi[0], wlo[1], whi[2]],
                                   [whi[0], whi[1], whi[2]], [wlo[0], whi[1], whi[2]]],
                    });
                    prop_faces += 1;
                }
            }
        }
    }

    let disp = extract_displacements(&mut bsp, MIN_AREA)?;
    stats.disp_faces = disp.len();
    for f in disp {
        for p in &f.poly { for k in 0..3 { bmin[k] = bmin[k].min(p[k]); bmax[k] = bmax[k].max(p[k]); } }
        faces.push(f);
    }
    stats.up_faces = faces.len();
    stats.prop_faces = prop_faces;
    stats.props_placed = props_placed;
    stats.props_no_hull = props_no_hull;

    let play = if spawns.is_empty() { None } else {
        let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
        for s in &spawns { for k in 0..3 { lo[k] = lo[k].min(s[k]); hi[k] = hi[k].max(s[k]); } }
        Some((lo, hi))
    };

    let name = path.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
    Ok(Geometry { name, version, faces, solids, spawns, bounds: (bmin, bmax), play, stats,
        props_scanned: prop_faces > 0 })
}

/// Displacement surfaces at FULL tessellation. A drawing-oriented reader caps this (bspgeo.js
/// uses 8x8 cells) because it is plenty for a preview; for collision the cap would smooth away
/// exactly the small lips that make a surf spot, so every quad is emitted.
fn extract_displacements(bsp: &mut Bsp, min_area: f64) -> Result<Vec<Face>, String> {
    let mut out = Vec::new();
    let info = bsp.read(lump::DISPINFO)?;
    if info.is_empty() { return Ok(out); }
    let dverts = bsp.read(lump::DISP_VERTS)?;
    let verts = bsp.read(lump::VERTEXES)?;
    let edges = bsp.read(lump::EDGES)?;
    let surf = bsp.read(lump::SURFEDGES)?;
    let faces_buf = bsp.read(lump::FACES)?;
    if faces_buf.is_empty() { return Ok(out); }

    let n_disp = info.len() / DISPINFO_SIZE;
    let n_verts = verts.len() / 12;
    let n_edges = edges.len() / 4;
    let n_surf = surf.len() / 4;
    let n_faces = faces_buf.len() / FACE_SIZE;

    let vpos = |i: usize| -> P3 {
        [bsp::f32le(&verts, i * 12) as f64, bsp::f32le(&verts, i * 12 + 4) as f64,
         bsp::f32le(&verts, i * 12 + 8) as f64]
    };
    let lerp = |a: P3, b: P3, t: f64| -> P3 {
        [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
    };

    for f in 0..n_faces {
        let fo = f * FACE_SIZE;
        let di = bsp::i16le(&faces_buf, fo + 12);
        if di < 0 || di as usize >= n_disp { continue; }
        if bsp::i16le(&faces_buf, fo + 8) != 4 { continue; }
        let first_edge = bsp::i32le(&faces_buf, fo + 4) as usize;

        let mut corners = [[0.0f64; 3]; 4];
        let mut bad = false;
        for i in 0..4 {
            let si = first_edge + i;
            if si >= n_surf { bad = true; break; }
            let s = bsp::i32le(&surf, si * 4);
            let ei = s.unsigned_abs() as usize;
            if ei >= n_edges { bad = true; break; }
            let vi = if s >= 0 { bsp::u16le(&edges, ei * 4) } else { bsp::u16le(&edges, ei * 4 + 2) } as usize;
            if vi >= n_verts { bad = true; break; }
            corners[i] = vpos(vi);
        }
        if bad { continue; }

        let base = di as usize * DISPINFO_SIZE;
        let start = [bsp::f32le(&info, base) as f64, bsp::f32le(&info, base + 4) as f64,
                     bsp::f32le(&info, base + 8) as f64];
        let vstart = bsp::i32le(&info, base + 12) as usize;
        let power = bsp::i32le(&info, base + 20);
        if !(2..=4).contains(&power) { continue; }
        let size = (1usize << power) + 1;

        // the corner nearest startPosition is grid (0,0)
        let mut best = 0usize;
        let mut best_d = f64::MAX;
        for (i, c) in corners.iter().enumerate() {
            let d = (c[0] - start[0]).powi(2) + (c[1] - start[1]).powi(2) + (c[2] - start[2]).powi(2);
            if d < best_d { best_d = d; best = i; }
        }
        let c: Vec<P3> = (0..4).map(|i| corners[(best + i) % 4]).collect();

        let mut grid = Vec::with_capacity(size);
        for i in 0..size {
            let ti = i as f64 / (size - 1) as f64;
            let (l, r) = (lerp(c[0], c[1], ti), lerp(c[3], c[2], ti));
            let mut row = Vec::with_capacity(size);
            for j in 0..size {
                let mut p = lerp(l, r, j as f64 / (size - 1) as f64);
                let vi = vstart + i * size + j;
                let o = vi * 20;   // ddispvert_t: vec3 vec, f32 dist, f32 alpha
                if o + 16 <= dverts.len() {
                    let dist = bsp::f32le(&dverts, o + 12) as f64;
                    for k in 0..3 { p[k] += bsp::f32le(&dverts, o + k * 4) as f64 * dist; }
                }
                row.push(p);
            }
            grid.push(row);
        }

        for i in 0..size - 1 {
            for j in 0..size - 1 {
                let (a, b, cc, d) = (grid[i][j], grid[i][j + 1], grid[i + 1][j + 1], grid[i + 1][j]);
                for tri in [[a, b, cc], [a, cc, d]] {
                    let e1 = [tri[1][0] - tri[0][0], tri[1][1] - tri[0][1], tri[1][2] - tri[0][2]];
                    let e2 = [tri[2][0] - tri[0][0], tri[2][1] - tri[0][1], tri[2][2] - tri[0][2]];
                    let mut n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2],
                                 e1[0] * e2[1] - e1[1] * e2[0]];
                    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                    if len < 1e-6 { continue; }
                    for k in 0..3 { n[k] /= len; }
                    if n[2] <= MIN_UP_Z { continue; }    // same rule as brushes
                    if len / 2.0 < min_area { continue; }
                    let d = tri[0][0] * n[0] + tri[0][1] * n[1] + tri[0][2] * n[2];
                    out.push(Face { n, d, poly: tri.to_vec(), contents: CONTENTS_SOLID, is_disp: true });
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brush_faces(planes: &[(P3, f64)]) -> Vec<Face> {
        let mut out = Vec::new();
        for (i, (n, d)) in planes.iter().enumerate() {
            let Some(mut poly) = base_winding(*n, *d) else { continue };
            for (j, (n2, d2)) in planes.iter().enumerate() {
                if i == j || poly.is_empty() { continue; }
                poly = clip_to_plane(&poly, *n2, *d2);
            }
            if poly.len() >= 3 { out.push(Face { n: *n, d: *d, poly, contents: 1, is_disp: false }); }
        }
        out
    }

    /// For any closed convex polyhedron, sum(area * normal) over all faces is the zero vector.
    /// This catches a wrong plane sign, a bad winding, an over-aggressive clip, or a missing
    /// side — the whole class of bugs that otherwise only shows up as a spot that isn't there.
    fn closure_error(faces: &[Face]) -> f64 {
        let (mut s, mut total) = ([0.0f64; 3], 0.0f64);
        for f in faces {
            let a = f.area();
            for k in 0..3 { s[k] += a * f.n[k]; }
            total += a;
        }
        if total <= 0.0 { return f64::MAX; }
        (s[0] * s[0] + s[1] * s[1] + s[2] * s[2]).sqrt() / total
    }

    #[test]
    fn cube_is_closed() {
        let cube = [([1.0, 0.0, 0.0], 32.0), ([-1.0, 0.0, 0.0], 32.0), ([0.0, 1.0, 0.0], 32.0),
                    ([0.0, -1.0, 0.0], 32.0), ([0.0, 0.0, 1.0], 32.0), ([0.0, 0.0, -1.0], 32.0)];
        let f = brush_faces(&cube);
        assert_eq!(f.len(), 6, "cube has 6 faces");
        for face in &f {
            assert_eq!(face.poly.len(), 4);
            assert!((face.area() - 4096.0).abs() < 0.01, "face area 4096, got {}", face.area());
        }
        assert!(closure_error(&f) < 1e-6);
    }

    #[test]
    fn wedge_slope_is_surfable() {
        let a = 60f64.to_radians();
        let wedge = [([0.0, 0.0, -1.0], 0.0), ([-1.0, 0.0, 0.0], 0.0), ([0.0, 1.0, 0.0], 32.0),
                     ([0.0, -1.0, 0.0], 32.0), ([a.sin(), 0.0, a.cos()], 64.0 * a.sin())];
        let f = brush_faces(&wedge);
        assert_eq!(f.len(), 5);
        assert!(closure_error(&f) < 1e-6);
        let slope = f.iter().find(|x| (x.n[2] - a.cos()).abs() < 1e-9).expect("slope face");
        assert!(slope.n[2] > 0.0 && slope.n[2] < STANDABLE_NORMAL_Z, "60 deg is surfable");
    }

    #[test]
    fn base_winding_orientation_and_planarity() {
        let s = std::f64::consts::FRAC_1_SQRT_2;
        // the last is deliberately NOT unit length (|n| = 1.00035) — BSP normals are f32
        for n in [[0.0, 0.0, 1.0], [0.0, 0.0, -1.0], [1.0, 0.0, 0.0], [s, s, 0.0], [0.267, 0.535, 0.802]] {
            let w = base_winding(n, 100.0).expect("winding");
            let (a, b, c) = (w[0], w[1], w[2]);
            let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let cr = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2],
                      e1[0] * e2[1] - e1[1] * e2[0]];
            let l = (cr[0] * cr[0] + cr[1] * cr[1] + cr[2] * cr[2]).sqrt();
            let dot = (cr[0] * n[0] + cr[1] * n[1] + cr[2] * n[2]) / l;
            assert!(dot > 0.999, "winding normal matches plane normal for {n:?} (dot {dot})");
            for p in &w {
                let off = p[0] * n[0] + p[1] * n[1] + p[2] * n[2] - 100.0;
                assert!(off.abs() < 0.05, "seed vertex on plane, off by {off}");
            }
        }
    }
}
