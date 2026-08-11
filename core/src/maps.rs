//! Finding maps on disk, and caching scan results so the same map isn't rebuilt every time.
//!
//! The cache stores the finished spot list, not the geometry. Geometry is tens of megabytes
//! and cheap to rebuild (~70ms); the spot list is small and is what anyone actually wants
//! back. Entries are keyed on the .bsp's size and mtime, so replacing a map file invalidates
//! its cache without anyone having to remember to clear it.

use std::fs;
use std::path::{Path, PathBuf};

/// Bump when the scan output format or the analysis changes, so stale entries are ignored
/// rather than silently served.
pub const CACHE_VERSION: u32 = 1;

/// Where CS:GO / Classic Counter keep their maps. Checked in order; all that exist are used.
pub fn map_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut add = |p: PathBuf| { if p.is_dir() && !out.contains(&p) { out.push(p); } };

    if let Some(home) = std::env::var_os("USERPROFILE").map(PathBuf::from) {
        add(home.join("Desktop/ClassicCounter/csgo/maps"));
    }
    for base in ["C:/Program Files (x86)/Steam/steamapps/common",
                 "C:/Program Files/Steam/steamapps/common", "D:/Steam/steamapps/common"] {
        let b = Path::new(base);
        add(b.join("Counter-Strike Global Offensive/csgo/maps"));
        add(b.join("Counter-Strike Source/cstrike/maps"));
    }
    if let Ok(extra) = std::env::var("PIXELSURF_MAPS") {
        for p in extra.split(';').filter(|s| !s.is_empty()) { add(PathBuf::from(p)); }
    }
    out
}

/// Every map name available across the search dirs, sorted and de-duplicated.
pub fn list_maps(dirs: &[PathBuf]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for d in dirs {
        let Ok(rd) = fs::read_dir(d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().map_or(true, |x| !x.eq_ignore_ascii_case("bsp")) { continue; }
            if let Some(s) = p.file_stem() {
                let n = s.to_string_lossy().to_lowercase();
                if !names.contains(&n) { names.push(n); }
            }
        }
    }
    names.sort();
    names
}

pub fn find_bsp(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    let safe: String = name.chars().filter(|c| c.is_alphanumeric() || "_-.".contains(*c)).collect();
    if safe.is_empty() { return None; }
    for d in dirs {
        let p = d.join(format!("{safe}.bsp"));
        if p.is_file() { return Some(p); }
    }
    None
}

/// Size + mtime of the .bsp, so a replaced map invalidates its own cache entry.
pub fn signature(bsp: &Path) -> String {
    let Ok(m) = fs::metadata(bsp) else { return "0-0".into() };
    let secs = m.modified().ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs()).unwrap_or(0);
    format!("{}-{}-v{}", m.len(), secs, CACHE_VERSION)
}

pub fn cache_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join("AppData/Local")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("pixelsurf-calc/cache")
}

fn cache_path(name: &str) -> PathBuf { cache_dir().join(format!("{name}.json")) }

/// Returns the cached payload if it was built from this exact .bsp.
pub fn load(name: &str, sig: &str) -> Option<String> {
    let txt = fs::read_to_string(cache_path(name)).ok()?;
    // the signature is the first line; the payload is everything after it
    let (head, body) = txt.split_once('\n')?;
    if head.trim() != sig { return None; }
    Some(body.to_string())
}

pub fn store(name: &str, sig: &str, payload: &str) {
    let dir = cache_dir();
    if fs::create_dir_all(&dir).is_err() { return; }
    let path = cache_path(name);
    let tmp = path.with_extension("tmp");
    if fs::write(&tmp, format!("{sig}\n{payload}")).is_ok() {
        let _ = fs::rename(&tmp, &path);
    }
}

pub fn clear_cache() -> std::io::Result<()> {
    let d = cache_dir();
    if d.is_dir() { fs::remove_dir_all(&d)?; }
    Ok(())
}

// ---------------------------------------------------------------- json (no serde)

/// Minimal JSON string escaping — map names and class labels only, so this is sufficient.
pub fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""), '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"), '\r' => o.push_str("\\r"), '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o
}

/// Fixed-precision number that never emits NaN/Infinity (which are not valid JSON).
pub fn num(v: f64, places: usize) -> String {
    if !v.is_finite() { return "null".into(); }
    format!("{v:.places$}")
}
