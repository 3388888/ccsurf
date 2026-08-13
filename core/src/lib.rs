//! Pixel surf, pixel walk and out-of-bounds spot finding for CS:GO (Source 1) maps.
//!
//! Two questions, one geometry layer:
//!
//!   * where are the pixel surfs on this map, and exactly how do I get onto each one
//!   * which surfaces can a player stand or surf on that the mapmaker never meant them to
//!
//! `demo-reader` answers the retrospective version of the first ("did someone do this in this
//! demo"). This crate answers the prospective one, from the map alone.
//!
//! Deliberately zero-dependency, same discipline as `native/csgo-rs`.

pub mod bsp;
pub mod collide;
pub mod consts;
pub mod jumptable;
pub mod maps;
pub mod render;
pub mod spots;

use spots::{Kind, ScanOptions};

/// Scan a map by name, using the cache when the .bsp hasn't changed.
///
/// Returns JSON. `force` bypasses a valid cache entry (for when the analysis itself changed
/// during development).
pub fn scan_map(name: &str, opts: &ScanOptions, force: bool) -> Result<String, String> {
    let dirs = maps::map_dirs();
    let Some(path) = maps::find_bsp(name, &dirs) else {
        return Err(format!("map \"{name}\" not found in any maps folder"));
    };
    let sig = maps::signature(&path);
    if !force {
        if let Some(hit) = maps::load(name, &sig) { return Ok(hit); }
    }

    let t0 = std::time::Instant::now();
    let geo = collide::extract(&path)?;
    let res = spots::scan(&geo, opts);
    let found = &res.spots;
    let ms = t0.elapsed().as_millis();

    let json = to_json(name, &geo, found, res.blocked, ms);
    maps::store(name, &sig, &json);
    Ok(json)
}

fn to_json(name: &str, geo: &collide::Geometry, found: &[spots::Spot], blocked: usize, ms: u128) -> String {
    use maps::{esc, num};
    let mut s = String::with_capacity(found.len() * 220 + 512);

    let count = |k: Kind| found.iter().filter(|x| x.kind == k).count();
    let oob = found.iter().filter(|x| !x.reachable).count();

    s.push_str(&format!(
        "{{\"map\":\"{}\",\"bspVersion\":{},\"scanMs\":{},\
         \"counts\":{{\"total\":{},\"pixelsurf\":{},\"pixelwalk\":{},\"surf\":{},\"ground\":{},\"outOfBounds\":{},\"blockedNoHullFit\":{}}},\
         \"stats\":{{\"brushes\":{},\"brushesSolid\":{},\"brushesKept\":{},\"upFaces\":{},\"dispFaces\":{},\
         \"unbounded\":{},\"movingBrushEnts\":{},\"spawns\":{}}},\
         \"propsScanned\":{},\
         \"limitations\":[\"static prop collision (.phy) is not read - spots on crates, awnings and pipes are NOT found\"",
        esc(name), geo.version, ms,
        found.len(), count(Kind::PixelSurf), count(Kind::PixelWalk), count(Kind::Surf), count(Kind::Ground), oob, blocked,
        geo.stats.brushes, geo.stats.brushes_solid, geo.stats.brushes_kept,
        geo.stats.up_faces, geo.stats.disp_faces, geo.stats.unbounded,
        geo.stats.moving_brush_ents, geo.spawns.len(),
        geo.props_scanned));
    if geo.stats.moving_brush_ents > 0 {
        s.push_str(&format!(",\"{} brush entities with a moving origin (doors, lifts) are at their compiled position\"",
            geo.stats.moving_brush_ents));
    }
    s.push_str("],\"spots\":[");

    for (i, sp) in found.iter().enumerate() {
        if i > 0 { s.push(','); }
        s.push_str(&format!(
            "{{\"kind\":\"{}\",\"x\":{},\"y\":{},\"z\":{},\"eyeZ\":{},\"width\":{},\"area\":{},\
             \"slopeDeg\":{},\"isClip\":{},\"isDisp\":{},\"duckOnly\":{},\"reachable\":{},\"heightAboveReachable\":{}",
            sp.kind.as_str(),
            num(sp.pos[0], 2), num(sp.pos[1], 2), num(sp.pos[2], 2), num(sp.eye_z, 2),
            num(sp.width, 2), num(sp.area, 1), num(sp.slope_deg, 1),
            sp.is_clip, sp.is_disp, sp.duck_only, sp.reachable,
            if sp.height_above_reachable < 0.0 { "null".into() } else { num(sp.height_above_reachable, 1) }));
        if let Some(c) = sp.oob_class { s.push_str(&format!(",\"oobClass\":\"{c}\"")); }
        s.push_str(",\"entries\":[");
        for (j, e) in sp.entries.iter().enumerate() {
            if j > 0 { s.push(','); }
            s.push_str(&format!(
                "{{\"label\":\"{}\",\"players\":{},\"jump\":{},\"crouch\":{},\"standEye\":{},\"tick64\":{}}}",
                esc(e.label), e.players,
                e.jump.map_or("null".to_string(), |v| num(v, 2)),
                e.crouch, num(e.stand_eye, 2), e.tickrates.contains(&64)));
        }
        s.push_str("]}");
    }
    s.push_str("]}");
    s
}

// ---------------------------------------------------------------- 3D viewer

/// Base64, hand-rolled to keep the crate dependency-free.
fn b64(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for c in data.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18 & 63) as usize] as char);
        out.push(T[(n >> 12 & 63) as usize] as char);
        out.push(if c.len() > 1 { T[(n >> 6 & 63) as usize] as char } else { '=' });
        out.push(if c.len() > 2 { T[(n & 63) as usize] as char } else { '=' });
    }
    out
}

/// Build a standalone HTML viewer for a map: the geometry and the spot list are embedded, so
/// the file opens by double-click with no server, no toolchain and no Tauri.
pub fn build_viewer(name: &str, opts: &ScanOptions) -> Result<(String, usize, usize), String> {
    let dirs = maps::map_dirs();
    let Some(path) = maps::find_bsp(name, &dirs) else {
        return Err(format!("map \"{name}\" not found in any maps folder"));
    };
    let mesh = render::extract(&path)?;
    let geo = collide::extract(&path)?;
    let res = spots::scan(&geo, opts);

    // i16 little-endian, which is what the page reinterprets as an Int16Array
    let mut bytes = Vec::with_capacity(mesh.pos.len() * 2);
    for v in &mesh.pos { bytes.extend_from_slice(&v.to_le_bytes()); }

    let spots_json = {
        let full = to_json(name, &geo, &res.spots, res.blocked, 0);
        // the page only wants the spots array
        let start = full.find("\"spots\":[").map(|i| i + 8).unwrap_or(0);
        full[start..full.len().saturating_sub(1)].to_string()
    };

    let html = include_str!("../../frontend/viewer_template.html")
        .replace("__MAP__", &maps::esc(name))
        .replace("__GEO__", &b64(&bytes))
        .replace("__SPOTS__", &spots_json);
    Ok((html, mesh.tri_count, res.spots.len()))
}
