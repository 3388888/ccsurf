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

/// Game folders to look for inside any Steam library, as (app folder, maps subpath).
const GAME_DIRS: &[(&str, &str)] = &[
    ("Counter-Strike Global Offensive", "csgo/maps"),
    ("Counter-Strike Source", "cstrike/maps"),
    ("Half-Life 2", "hl2/maps"),
];

/// Ask Windows where Steam is installed. Uses reg.exe rather than a registry crate so the
/// crate stays dependency-free; a missing key or a non-Windows host just yields nothing.
fn steam_root_from_registry() -> Option<PathBuf> {
    for (hive, key) in [("HKCU", r"Software\Valve\Steam"), ("HKLM", r"SOFTWARE\WOW6432Node\Valve\Steam")] {
        let value = if hive == "HKCU" { "SteamPath" } else { "InstallPath" };
        let out = std::process::Command::new("reg")
            .args(["query", &format!("{hive}\\{key}"), "/v", value])
            .output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            if !line.contains(value) { continue; }
            // "    SteamPath    REG_SZ    C:\Program Files (x86)\Steam"
            if let Some(idx) = line.find("REG_SZ") {
                let p = line[idx + 6..].trim();
                if !p.is_empty() { return Some(PathBuf::from(p.replace('/', "\\"))); }
            }
        }
    }
    None
}

/// Every Steam library folder, read from Steam's own index.
///
/// This is the part that matters for other people: `libraryfolders.vdf` lists every library
/// Steam knows about, on every drive, so a user with the game on D:\SteamLibrary is found
/// without guessing. Parsed by hand — the file is a simple quoted key/value tree and pulling
/// in a VDF crate for one field would not be worth it.
fn steam_libraries() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut push = |p: PathBuf| { if p.is_dir() && !roots.contains(&p) { roots.push(p); } };

    if let Some(r) = steam_root_from_registry() { push(r); }
    for p in ["C:/Program Files (x86)/Steam", "C:/Program Files/Steam"] { push(PathBuf::from(p)); }
    // last resort: Steam parked on another drive without a registry entry
    for d in 'C'..='Z' {
        for suffix in ["Steam", "SteamLibrary", "Games/Steam"] {
            push(PathBuf::from(format!("{d}:/{suffix}")));
        }
    }

    let mut libs = Vec::new();
    let mut add_lib = |p: PathBuf| { if p.is_dir() && !libs.contains(&p) { libs.push(p); } };
    for root in &roots {
        add_lib(root.clone());
        // libraryfolders.vdf moved between Steam versions; try both homes
        for rel in ["steamapps/libraryfolders.vdf", "config/libraryfolders.vdf"] {
            let Ok(text) = fs::read_to_string(root.join(rel)) else { continue };
            for line in text.lines() {
                let t = line.trim();
                // entries look like:   "path"    "D:\\SteamLibrary"
                if !t.starts_with("\"path\"") { continue; }
                let mut parts = t.split('"').filter(|s| !s.trim().is_empty());
                let (_, val) = (parts.next(), parts.next());
                if let Some(v) = val {
                    let cleaned = v.trim().replace("\\\\", "\\");
                    if !cleaned.is_empty() { add_lib(PathBuf::from(cleaned)); }
                }
            }
        }
    }
    libs
}

/// Where CS:GO / CS:S / Classic Counter keep their maps.
///
/// Order: user-configured folders first (they win), then every Steam library Steam itself
/// knows about, then non-Steam installs like Classic Counter. Nothing here is specific to one
/// machine — the previous version hardcoded a Desktop path and three Steam roots, which found
/// nothing for anyone whose setup differed.
pub fn map_dirs() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    // Windows paths are case-insensitive and the registry hands back a different casing than
    // the literal fallbacks, so a plain contains() lets the same folder in twice — which then
    // scans it twice and double-lists every map in it.
    let add = |out: &mut Vec<PathBuf>, p: PathBuf| {
        if !p.is_dir() { return; }
        let canon = p.canonicalize().unwrap_or_else(|_| p.clone());
        let key = canon.to_string_lossy().to_lowercase().replace('/', "\\");
        let dup = out.iter().any(|q| {
            let qc = q.canonicalize().unwrap_or_else(|_| q.clone());
            qc.to_string_lossy().to_lowercase().replace('/', "\\") == key
        });
        if !dup { out.push(p); }
    };

    // 1. explicit config — the escape hatch for anything the search misses
    for p in configured_dirs() { add(&mut out, p); }
    if let Ok(extra) = std::env::var("PIXELSURF_MAPS") {
        for p in extra.split(';').filter(|s| !s.trim().is_empty()) {
            add(&mut out, PathBuf::from(p.trim()));
        }
    }

    // 2. every Steam library, from Steam's own index
    for lib in steam_libraries() {
        for (game, maps) in GAME_DIRS {
            add(&mut out, lib.join("steamapps/common").join(game).join(maps));
        }
    }

    // 3. non-Steam installs (Classic Counter and friends) in the usual places
    let mut bases: Vec<PathBuf> = Vec::new();
    for var in ["USERPROFILE", "PROGRAMFILES", "PROGRAMFILES(X86)"] {
        if let Some(v) = std::env::var_os(var) {
            let b = PathBuf::from(v);
            bases.push(b.clone());
            bases.push(b.join("Desktop"));
            bases.push(b.join("Downloads"));
        }
    }
    for d in 'C'..='Z' { bases.push(PathBuf::from(format!("{d}:/"))); bases.push(PathBuf::from(format!("{d}:/Games"))); }
    for b in bases {
        for name in ["ClassicCounter", "cc", "csgo"] {
            add(&mut out, b.join(name).join("csgo/maps"));
        }
    }

    out
}

// ---------------------------------------------------------------- user config

pub fn config_path() -> PathBuf { cache_dir().parent().unwrap_or(&cache_dir()).join("mapdirs.txt") }

/// Extra folders the user pointed us at, one per line. Blank lines and `#` comments ignored.
pub fn configured_dirs() -> Vec<PathBuf> {
    let Ok(text) = fs::read_to_string(config_path()) else { return Vec::new() };
    text.lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(PathBuf::from)
        .collect()
}

/// Remember a folder so it is searched from now on. Returns whether it was actually added.
pub fn add_map_dir(dir: &str) -> Result<bool, String> {
    let p = PathBuf::from(dir.trim());
    if !p.is_dir() { return Err(format!("not a folder: {}", p.display())); }
    let mut dirs = configured_dirs();
    if dirs.contains(&p) { return Ok(false); }
    dirs.push(p);
    let path = config_path();
    if let Some(parent) = path.parent() { fs::create_dir_all(parent).map_err(|e| e.to_string())?; }
    let body = dirs.iter().map(|d| d.display().to_string()).collect::<Vec<_>>().join("\n");
    fs::write(&path, format!("# Extra maps folders for Pixelsurf Calc, one per line.\n{body}\n"))
        .map_err(|e| e.to_string())?;
    Ok(true)
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
