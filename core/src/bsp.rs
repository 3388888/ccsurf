//! Minimal VBSP lump reader — only the byte ranges we actually need.
//!
//! For a plain .bsp this never pulls the (huge) pakfile or lighting lumps off disk at all,
//! which is why a 400MB map still opens in milliseconds.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub mod lump {
    pub const ENTITIES: usize = 0;
    pub const PLANES: usize = 1;
    pub const VERTEXES: usize = 3;
    pub const TEXINFO: usize = 6;
    pub const FACES: usize = 7;
    pub const EDGES: usize = 12;
    pub const SURFEDGES: usize = 13;
    pub const MODELS: usize = 14;
    pub const BRUSHES: usize = 18;
    pub const BRUSHSIDES: usize = 19;
    pub const DISPINFO: usize = 26;
    pub const DISP_VERTS: usize = 33;
}

#[derive(Clone, Copy)]
struct LumpDir { ofs: i32, len: i32 }

pub struct Bsp {
    file: File,
    lumps: [LumpDir; 64],
    pub version: i32,
}

impl Bsp {
    pub fn open(path: &Path) -> Result<Bsp, String> {
        if path.extension().map_or(false, |e| e.eq_ignore_ascii_case("bz2")) {
            return Err("compressed .bsp.bz2 is not supported — extract it first".into());
        }
        let mut file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut head = [0u8; 1036];
        file.read_exact(&mut head).map_err(|e| format!("short header: {e}"))?;

        let magic = &head[0..4];
        if magic != b"VBSP" {
            return Err(if magic == b"rBSP" {
                "Respawn bsp variant not supported".into()
            } else {
                format!("not a VBSP map (magic {:?})", String::from_utf8_lossy(magic))
            });
        }
        let version = i32le(&head, 4);
        let mut lumps = [LumpDir { ofs: 0, len: 0 }; 64];
        for (i, l) in lumps.iter_mut().enumerate() {
            let o = 8 + i * 16;
            *l = LumpDir { ofs: i32le(&head, o), len: i32le(&head, o + 4) };
        }
        Ok(Bsp { file, lumps, version })
    }

    /// Byte offset of a lump in the file. Needed because game-lump sub-offsets are absolute
    /// in most maps but relative in a few.
    pub fn lump_offset(&self, index: usize) -> usize {
        self.lumps.get(index).map(|l| l.ofs.max(0) as usize).unwrap_or(0)
    }

    pub fn read(&mut self, index: usize) -> Result<Vec<u8>, String> {
        let l = match self.lumps.get(index) { Some(l) => *l, None => return Ok(Vec::new()) };
        if l.len <= 0 || l.ofs < 0 { return Ok(Vec::new()); }
        let mut buf = vec![0u8; l.len as usize];
        self.file.seek(SeekFrom::Start(l.ofs as u64)).map_err(|e| e.to_string())?;
        self.file.read_exact(&mut buf).map_err(|e| format!("lump {index}: {e}"))?;
        // some console/workshop maps compress individual lumps; we can't inflate those
        if buf.len() >= 4 && &buf[0..4] == b"LZMA" {
            return Err(format!("lump {index} is LZMA-compressed, not supported"));
        }
        Ok(buf)
    }
}

// ---------------------------------------------------------------- little-endian readers

#[inline] pub fn i32le(b: &[u8], o: usize) -> i32 {
    if o + 4 > b.len() { return 0; }
    i32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
#[inline] pub fn u16le(b: &[u8], o: usize) -> u16 {
    if o + 2 > b.len() { return 0; }
    u16::from_le_bytes(b[o..o + 2].try_into().unwrap())
}
#[inline] pub fn i16le(b: &[u8], o: usize) -> i16 {
    if o + 2 > b.len() { return 0; }
    i16::from_le_bytes(b[o..o + 2].try_into().unwrap())
}
#[inline] pub fn f32le(b: &[u8], o: usize) -> f32 {
    if o + 4 > b.len() { return 0.0; }
    f32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

// ---------------------------------------------------------------- entities

/// Minimal keyvalue scan of the entity lump — we only want classnames and origins.
pub fn parse_entities(buf: &[u8]) -> Vec<std::collections::HashMap<String, String>> {
    let txt = String::from_utf8_lossy(buf);
    let mut out = Vec::new();
    let mut cur: Option<std::collections::HashMap<String, String>> = None;
    for line in txt.lines() {
        let line = line.trim();
        if line == "{" { cur = Some(std::collections::HashMap::new()); continue; }
        if line == "}" { if let Some(e) = cur.take() { out.push(e); } continue; }
        let Some(ref mut ent) = cur else { continue };
        // "key" "value"
        let mut parts = line.splitn(4, '"');
        let (_, key, _, rest) = (parts.next(), parts.next(), parts.next(), parts.next());
        if let (Some(k), Some(r)) = (key, rest) {
            let v = r.trim_end_matches('"');
            ent.insert(k.to_ascii_lowercase(), v.to_string());
        }
    }
    out
}

pub fn vec3(s: &str) -> Option<[f64; 3]> {
    let p: Vec<f64> = s.split_whitespace().filter_map(|x| x.parse().ok()).collect();
    if p.len() >= 3 { Some([p[0], p[1], p[2]]) } else { None }
}
