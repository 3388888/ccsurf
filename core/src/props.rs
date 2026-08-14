//! Static props — the geometry the scanner has been blind to.
//!
//! Props are not in the brush lumps at all. They live in the GAME_LUMP (35) as a sub-lump
//! tagged `sprp`, which holds a dictionary of model paths plus one placement record per prop
//! (origin, angles, model index). The collision hull itself is in a `.phy` beside the `.mdl`
//! inside a VPK, so a full solution needs a VPK reader; this module does the half that lives
//! in the map, which is enough to say *where* props are and *which* ones they are.
//!
//! Why it matters: on de_vertigo_2019 a known wall surf at roughly (-2203, 185, 11776) has no
//! world geometry within 180 units. Filters, the hull test and the upward-normal cutoff were
//! all ruled out. Props were the only remaining explanation, and this is how you check.

use crate::bsp::{self, Bsp};

const GAME_LUMP: usize = 35;

#[derive(Clone, Debug)]
pub struct Prop {
    pub origin: [f64; 3],
    /// Pitch, yaw, roll in degrees, as the map stores them.
    pub angles: [f64; 3],
    pub model: String,
    /// SOLID_* value. 0 means no collision at all; 6 (SOLID_VPHYSICS) is the normal case for
    /// something you can stand on.
    pub solid: u8,
}

impl Prop {
    /// Props with `solid == 0` are decoration you fall straight through.
    pub fn is_solid(&self) -> bool { self.solid != 0 }
}

#[derive(Clone, Debug, Default)]
pub struct PropStats {
    pub dict_entries: usize,
    pub prop_entries: usize,
    pub solid_props: usize,
    pub lump_version: u16,
    /// Bytes per placement record. The struct grew across versions, so this is derived from
    /// the lump size rather than hardcoded — that way an unknown version still parses.
    pub entry_size: usize,
}

/// Read every static prop placement out of a map.
///
/// Returns an empty list rather than an error when a map simply has no props.
pub fn extract(bsp: &mut Bsp) -> Result<(Vec<Prop>, PropStats), String> {
    let buf = bsp.read(GAME_LUMP)?;
    if buf.len() < 4 { return Ok((Vec::new(), PropStats::default())); }

    // dgamelump_t directory: count, then {id i32, flags u16, version u16, ofs i32, len i32}
    let count = bsp::i32le(&buf, 0);
    if count <= 0 || count > 64 { return Ok((Vec::new(), PropStats::default())); }

    let mut sprp: Option<(usize, usize, u16)> = None;   // (offset, length, version)
    for i in 0..count as usize {
        let o = 4 + i * 16;
        if o + 16 > buf.len() { break; }
        let id = bsp::i32le(&buf, o);
        let version = u16::from_le_bytes([buf[o + 6], buf[o + 7]]);
        let ofs = bsp::i32le(&buf, o + 8) as usize;
        let len = bsp::i32le(&buf, o + 12) as usize;
        // the id is a four-character code; byte order differs between compilers, so accept both
        let tag = id.to_le_bytes();
        if &tag == b"sprp" || &tag == b"prps" {
            sprp = Some((ofs, len, version));
            break;
        }
    }
    let Some((ofs, len, version)) = sprp else { return Ok((Vec::new(), PropStats::default())) };

    // Game lump offsets are absolute file offsets in most maps, but a few compilers write them
    // relative to the game lump. Detect which by seeing whether the absolute reading lands
    // inside the buffer we already have.
    let lump_start = bsp.lump_offset(GAME_LUMP);
    let base = if ofs >= lump_start && ofs - lump_start < buf.len() { ofs - lump_start } else { ofs };
    if base + 4 > buf.len() { return Ok((Vec::new(), PropStats::default())); }
    let end = (base + len).min(buf.len());
    let d = &buf[base..end];

    let mut p = 0usize;
    let dict_entries = bsp::i32le(d, p).max(0) as usize; p += 4;
    if dict_entries > 1 << 20 { return Ok((Vec::new(), PropStats::default())); }

    let mut names = Vec::with_capacity(dict_entries);
    for _ in 0..dict_entries {
        if p + 128 > d.len() { return Ok((Vec::new(), PropStats::default())); }
        let raw = &d[p..p + 128];
        let n = raw.iter().position(|&c| c == 0).unwrap_or(128);
        names.push(String::from_utf8_lossy(&raw[..n]).to_string());
        p += 128;
    }

    // leaf array — we don't need it, but it has to be stepped over
    if p + 4 > d.len() { return Ok((Vec::new(), PropStats::default())); }
    let leaf_entries = bsp::i32le(d, p).max(0) as usize; p += 4;
    p += leaf_entries * 2;

    if p + 4 > d.len() { return Ok((Vec::new(), PropStats::default())); }
    let prop_entries = bsp::i32le(d, p).max(0) as usize; p += 4;
    if prop_entries == 0 || p >= d.len() {
        return Ok((Vec::new(), PropStats { dict_entries, lump_version: version, ..Default::default() }));
    }

    // StaticPropLump_t grew with every version (56 bytes at v4, 76 by v11), and CS:GO ships
    // several. Deriving the stride from the remaining bytes parses versions we've never seen.
    let entry_size = (d.len() - p) / prop_entries;
    if entry_size < 56 { return Ok((Vec::new(), PropStats { dict_entries, prop_entries, lump_version: version, entry_size, ..Default::default() })); }

    let mut out = Vec::with_capacity(prop_entries);
    let mut solid_props = 0usize;
    for i in 0..prop_entries {
        let o = p + i * entry_size;
        if o + 26 > d.len() { break; }
        let origin = [bsp::f32le(d, o) as f64, bsp::f32le(d, o + 4) as f64, bsp::f32le(d, o + 8) as f64];
        let angles = [bsp::f32le(d, o + 12) as f64, bsp::f32le(d, o + 16) as f64, bsp::f32le(d, o + 20) as f64];
        let idx = bsp::u16le(d, o + 24) as usize;
        // Solid sits after PropType/FirstLeaf/LeafCount: u16 + u16 + u16 = offset 30
        let solid = if o + 31 <= d.len() { d[o + 30] } else { 6 };
        if solid != 0 { solid_props += 1; }
        out.push(Prop {
            origin, angles,
            model: names.get(idx).cloned().unwrap_or_default(),
            solid,
        });
    }

    Ok((out, PropStats { dict_entries, prop_entries, solid_props, lump_version: version, entry_size }))
}

// ---------------------------------------------------------------- model hulls

/// studiohdr_t: "IDST", version, checksum, name[64], dataLength, eyeposition, illumposition,
/// then hull_min at 104 and hull_max at 116. That box is the model's collision extent, which
/// for the beams and slabs props are usually made of is very close to the real `.phy` hull.
const MDL_HULL_MIN: usize = 104;
const MDL_HULL_MAX: usize = 116;

pub fn model_hull(vpks: &[crate::vpk::Vpk], model: &str) -> Option<([f64; 3], [f64; 3])> {
    let head = vpks.iter().find_map(|v| v.read_prefix(model, 160))?;
    if head.len() < MDL_HULL_MAX + 12 || &head[0..4] != b"IDST" { return None; }
    let g = |o: usize| -> [f64; 3] {
        [crate::bsp::f32le(&head, o) as f64, crate::bsp::f32le(&head, o + 4) as f64,
         crate::bsp::f32le(&head, o + 8) as f64]
    };
    let (lo, hi) = (g(MDL_HULL_MIN), g(MDL_HULL_MAX));
    // a zero or inverted box means the model has no usable extent
    if (0..3).any(|k| !(hi[k] > lo[k])) { return None; }
    Some((lo, hi))
}

/// A prop's collision box placed in the world.
///
/// The hull is axis-aligned in model space, so yaw has to be applied and the result re-boxed.
/// That over-estimates a rotated non-square prop, which is the right way to be wrong here: it
/// is better to offer a spot that turns out to be an inch off than to hide it entirely.
pub fn world_box(prop: &Prop, lo: [f64; 3], hi: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    let yaw = prop.angles[1].to_radians();
    let (s, c) = (yaw.sin(), yaw.cos());
    let (mut wlo, mut whi) = ([f64::MAX; 3], [f64::MIN; 3]);
    for &x in &[lo[0], hi[0]] {
        for &y in &[lo[1], hi[1]] {
            let rx = x * c - y * s;
            let ry = x * s + y * c;
            wlo[0] = wlo[0].min(rx); whi[0] = whi[0].max(rx);
            wlo[1] = wlo[1].min(ry); whi[1] = whi[1].max(ry);
        }
    }
    wlo[2] = lo[2]; whi[2] = hi[2];
    for k in 0..3 { wlo[k] += prop.origin[k]; whi[k] += prop.origin[k]; }
    (wlo, whi)
}
