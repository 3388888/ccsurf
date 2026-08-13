//! Triangle soup for the 3D preview.
//!
//! Separate from `collide` on purpose. `collide` rebuilds brushes from planes at full float
//! precision because a pixel surf is a sub-unit feature; drawing has the opposite needs —
//! it wants every visible face (walls and ceilings included, which collide throws away) and
//! it does not care about precision, so positions quantise to i16 and the whole map fits in
//! a couple of MB.
//!
//! No materials, no textures, no props: the preview is matte geometry with flat shading, so
//! all that is needed is positions. Face normals are derived in the shader from the triangle
//! itself, so nothing else is stored.

use crate::bsp::{self, lump, Bsp};
use std::path::Path;

const FACE_SIZE: usize = 56;
const TEXINFO_SIZE: usize = 72;
const DISPINFO_SIZE: usize = 176;
const MODEL_SIZE: usize = 48;

// bspflags.h — surfaces that should never be drawn
const SURF_SKY2D: i32 = 0x2;
const SURF_SKY: i32 = 0x4;
const SURF_NODRAW: i32 = 0x80;
const SURF_TRIGGER: i32 = 0x40;
const SURF_HINT: i32 = 0x100;
const SURF_SKIP: i32 = 0x200;
const SKIP_MASK: i32 = SURF_SKY | SURF_SKY2D | SURF_NODRAW | SURF_TRIGGER | SURF_HINT | SURF_SKIP;

/// How finely displacements are tessellated for drawing. Unlike collision, a coarse mesh is
/// fine here — a cap of 8x8 quads per face keeps big terrain maps light.
const MAX_DISP_CELLS: usize = 8;

pub struct Mesh {
    /// 9 i16 per triangle: x,y,z for each of three corners.
    pub pos: Vec<i16>,
    pub tri_count: usize,
    pub bounds: ([f64; 3], [f64; 3]),
}

pub fn extract(path: &Path) -> Result<Mesh, String> {
    let mut b = Bsp::open(path)?;
    let verts = b.read(lump::VERTEXES)?;
    let edges = b.read(lump::EDGES)?;
    let surf = b.read(lump::SURFEDGES)?;
    let faces = b.read(lump::FACES)?;
    let texinfo = b.read(lump::TEXINFO)?;
    let dinfo = b.read(lump::DISPINFO)?;
    let dverts = b.read(lump::DISP_VERTS)?;
    let models = b.read(lump::MODELS)?;
    let ents = bsp::parse_entities(&b.read(lump::ENTITIES)?);

    if verts.is_empty() || faces.is_empty() { return Err("no geometry lumps".into()); }

    let n_verts = verts.len() / 12;
    let n_edges = edges.len() / 4;
    let n_surf = surf.len() / 4;
    let n_faces = faces.len() / FACE_SIZE;
    let n_texinfo = texinfo.len() / TEXINFO_SIZE;
    let n_disp = dinfo.len() / DISPINFO_SIZE;
    let n_models = models.len() / MODEL_SIZE;

    // brush-entity offsets: doors and lifts with an origin brush float away without this
    let mut face_off: Vec<Option<[f64; 3]>> = vec![None; n_faces];
    if n_models > 1 {
        for e in &ents {
            let (Some(m), Some(o)) = (e.get("model"), e.get("origin").and_then(|s| bsp::vec3(s))) else { continue };
            if !m.starts_with('*') { continue; }
            let Ok(idx) = m[1..].parse::<usize>() else { continue };
            if idx == 0 || idx >= n_models { continue; }
            if o == [0.0, 0.0, 0.0] { continue; }
            let base = idx * MODEL_SIZE;
            let first = bsp::i32le(&models, base + 40) as usize;
            let num = bsp::i32le(&models, base + 44) as usize;
            for f in first..(first + num).min(n_faces) { face_off[f] = Some(o); }
        }
    }

    // 3D-skybox mini-world: real geometry parked far away at 1/16 scale. Drawing it wrecks
    // the camera framing, so cull anything near the sky_camera.
    let sky_cam = ents.iter()
        .find(|e| e.get("classname").map(|c| c == "sky_camera").unwrap_or(false))
        .and_then(|e| e.get("origin")).and_then(|s| bsp::vec3(s));
    const SKY_CULL_R: f64 = 2600.0;

    let vpos = |i: usize| -> [f64; 3] {
        [bsp::f32le(&verts, i * 12) as f64, bsp::f32le(&verts, i * 12 + 4) as f64,
         bsp::f32le(&verts, i * 12 + 8) as f64]
    };

    let mut pos: Vec<i16> = Vec::with_capacity(1 << 18);
    let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
    let q = |v: f64| -> i16 { v.clamp(-32768.0, 32767.0).round() as i16 };

    let mut push_tri = |pos: &mut Vec<i16>, lo: &mut [f64; 3], hi: &mut [f64; 3],
                        a: [f64; 3], bb: [f64; 3], c: [f64; 3]| {
        if let Some(sc) = sky_cam {
            let m = [(a[0] + bb[0] + c[0]) / 3.0 - sc[0], (a[1] + bb[1] + c[1]) / 3.0 - sc[1],
                     (a[2] + bb[2] + c[2]) / 3.0 - sc[2]];
            if m[0] * m[0] + m[1] * m[1] + m[2] * m[2] < SKY_CULL_R * SKY_CULL_R { return; }
        }
        // drop degenerate slivers from bsp splitting — they add bytes and no pixels
        let e1 = [bb[0] - a[0], bb[1] - a[1], bb[2] - a[2]];
        let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
        if n[0] * n[0] + n[1] * n[1] + n[2] * n[2] < 4.0 { return; }
        for p in [a, bb, c] {
            pos.push(q(p[0])); pos.push(q(p[1])); pos.push(q(p[2]));
            for k in 0..3 { lo[k] = lo[k].min(p[k]); hi[k] = hi[k].max(p[k]); }
        }
    };

    let mut fv: Vec<[f64; 3]> = Vec::with_capacity(16);

    for f in 0..n_faces {
        let fo = f * FACE_SIZE;
        let first_edge = bsp::i32le(&faces, fo + 4) as usize;
        let num_edges = bsp::i16le(&faces, fo + 8) as usize;
        let ti = bsp::i16le(&faces, fo + 10);
        let di = bsp::i16le(&faces, fo + 12);
        if num_edges < 3 { continue; }
        if ti >= 0 && (ti as usize) < n_texinfo {
            let flags = bsp::i32le(&texinfo, ti as usize * TEXINFO_SIZE + 64);
            if flags & SKIP_MASK != 0 { continue; }
        }

        fv.clear();
        let mut bad = false;
        for i in 0..num_edges {
            let si = first_edge + i;
            if si >= n_surf { bad = true; break; }
            let s = bsp::i32le(&surf, si * 4);
            let ei = s.unsigned_abs() as usize;
            if ei >= n_edges { bad = true; break; }
            let vi = if s >= 0 { bsp::u16le(&edges, ei * 4) } else { bsp::u16le(&edges, ei * 4 + 2) } as usize;
            if vi >= n_verts { bad = true; break; }
            fv.push(vpos(vi));
        }
        if bad { continue; }
        if let Some(o) = face_off[f] {
            for p in fv.iter_mut() { for k in 0..3 { p[k] += o[k]; } }
        }

        if di >= 0 && (di as usize) < n_disp && num_edges == 4 {
            emit_disp(&mut pos, &mut lo, &mut hi, &mut push_tri, &dinfo, &dverts, di as usize, &fv);
            continue;
        }
        for i in 1..num_edges - 1 {
            push_tri(&mut pos, &mut lo, &mut hi, fv[0], fv[i], fv[i + 1]);
        }
    }

    if pos.is_empty() { return Err("no drawable faces".into()); }
    let tri_count = pos.len() / 9;
    Ok(Mesh { pos, tri_count, bounds: (lo, hi) })
}

#[allow(clippy::too_many_arguments)]
fn emit_disp(
    pos: &mut Vec<i16>, lo: &mut [f64; 3], hi: &mut [f64; 3],
    push: &mut impl FnMut(&mut Vec<i16>, &mut [f64; 3], &mut [f64; 3], [f64; 3], [f64; 3], [f64; 3]),
    dinfo: &[u8], dverts: &[u8], di: usize, corners: &[[f64; 3]],
) {
    let base = di * DISPINFO_SIZE;
    let start = [bsp::f32le(dinfo, base) as f64, bsp::f32le(dinfo, base + 4) as f64,
                 bsp::f32le(dinfo, base + 8) as f64];
    let vstart = bsp::i32le(dinfo, base + 12) as usize;
    let power = bsp::i32le(dinfo, base + 20);
    if !(2..=4).contains(&power) { return; }
    let size = (1usize << power) + 1;

    let mut best = 0usize;
    let mut best_d = f64::MAX;
    for (i, c) in corners.iter().enumerate().take(4) {
        let d = (c[0] - start[0]).powi(2) + (c[1] - start[1]).powi(2) + (c[2] - start[2]).powi(2);
        if d < best_d { best_d = d; best = i; }
    }
    let c: Vec<[f64; 3]> = (0..4).map(|i| corners[(best + i) % 4]).collect();
    let lerp = |a: [f64; 3], b: [f64; 3], t: f64| -> [f64; 3] {
        [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
    };

    // sample the grid coarsely — plenty for a preview
    let step = ((size - 1) / MAX_DISP_CELLS).max(1);
    let mut rows: Vec<usize> = (0..size).step_by(step).collect();
    if *rows.last().unwrap() != size - 1 { rows.push(size - 1); }

    let mut grid: Vec<Vec<[f64; 3]>> = Vec::with_capacity(rows.len());
    for &i in &rows {
        let ti = i as f64 / (size - 1) as f64;
        let (l, r) = (lerp(c[0], c[1], ti), lerp(c[3], c[2], ti));
        let mut row = Vec::with_capacity(rows.len());
        for &j in &rows {
            let mut p = lerp(l, r, j as f64 / (size - 1) as f64);
            let o = (vstart + i * size + j) * 20;
            if o + 16 <= dverts.len() {
                let dist = bsp::f32le(dverts, o + 12) as f64;
                for k in 0..3 { p[k] += bsp::f32le(dverts, o + k * 4) as f64 * dist; }
            }
            row.push(p);
        }
        grid.push(row);
    }
    for i in 0..grid.len() - 1 {
        for j in 0..grid[i].len() - 1 {
            let (a, b, cc, d) = (grid[i][j], grid[i][j + 1], grid[i + 1][j + 1], grid[i + 1][j]);
            push(pos, lo, hi, a, b, cc);
            push(pos, lo, hi, a, cc, d);
        }
    }
}
