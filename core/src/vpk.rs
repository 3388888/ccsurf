//! Just enough VPK to pull a model out of the game archives.
//!
//! Valve Pak: `<name>_dir.vpk` holds a directory tree, file data lives either inline in the
//! dir file (archive index 0x7fff) or in sibling `<name>_000.vpk`, `_001.vpk`, … The tree is
//! three nested levels of NUL-terminated strings — extension, path, filename — each level
//! ended by an empty string:
//!
//! ```text
//!   ext\0 { path\0 { file\0 { crc:u32 preload:u16 archive:u16 offset:u32 length:u32 0xffff
//!                             <preload bytes> } } }
//! ```
//!
//! Only the lookup path is implemented: we want a handful of `.mdl` headers, not extraction.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const SIGNATURE: u32 = 0x55aa_1234;
const INLINE_ARCHIVE: u16 = 0x7fff;

#[derive(Clone, Copy, Debug)]
struct Entry {
    archive: u16,
    offset: u32,
    length: u32,
    preload_offset: u32,
    preload_len: u16,
}

pub struct Vpk {
    dir_path: PathBuf,
    /// Where file data begins inside the _dir file, for inline entries.
    data_base: u64,
    entries: HashMap<String, Entry>,
}

fn u32le(b: &[u8], o: usize) -> u32 {
    if o + 4 > b.len() { return 0; }
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn u16le(b: &[u8], o: usize) -> u16 {
    if o + 2 > b.len() { return 0; }
    u16::from_le_bytes(b[o..o + 2].try_into().unwrap())
}

/// Read a NUL-terminated string, advancing `p`.
fn cstr(b: &[u8], p: &mut usize) -> String {
    let start = *p;
    while *p < b.len() && b[*p] != 0 { *p += 1; }
    let s = String::from_utf8_lossy(&b[start..*p]).to_string();
    *p += 1;   // skip the NUL
    s
}

impl Vpk {
    pub fn open(dir_vpk: &Path) -> Result<Vpk, String> {
        let mut f = File::open(dir_vpk).map_err(|e| format!("{}: {e}", dir_vpk.display()))?;
        let mut head = [0u8; 28];
        f.read_exact(&mut head[..12]).map_err(|e| e.to_string())?;
        if u32le(&head, 0) != SIGNATURE { return Err("not a VPK".into()); }
        let version = u32le(&head, 4);
        let tree_size = u32le(&head, 8) as usize;
        let header_size = match version {
            1 => 12,
            2 => { f.read_exact(&mut head[12..28]).map_err(|e| e.to_string())?; 28 }
            v => return Err(format!("unsupported VPK version {v}")),
        };

        let mut tree = vec![0u8; tree_size];
        f.seek(SeekFrom::Start(header_size as u64)).map_err(|e| e.to_string())?;
        f.read_exact(&mut tree).map_err(|e| e.to_string())?;

        let mut entries = HashMap::new();
        let mut p = 0usize;
        'ext: loop {
            let ext = cstr(&tree, &mut p);
            if ext.is_empty() || p >= tree.len() { break 'ext; }
            loop {
                let dir = cstr(&tree, &mut p);
                if dir.is_empty() || p >= tree.len() { break; }
                loop {
                    let name = cstr(&tree, &mut p);
                    if name.is_empty() || p >= tree.len() { break; }
                    if p + 18 > tree.len() { break 'ext; }
                    let preload_len = u16le(&tree, p + 4);
                    let e = Entry {
                        archive: u16le(&tree, p + 6),
                        offset: u32le(&tree, p + 8),
                        length: u32le(&tree, p + 12),
                        preload_offset: (p + 18) as u32,
                        preload_len,
                    };
                    p += 18 + preload_len as usize;
                    // path is " " for the archive root
                    let full = if dir == " " { format!("{name}.{ext}") } else { format!("{dir}/{name}.{ext}") };
                    entries.insert(full.to_lowercase(), e);
                }
            }
        }

        Ok(Vpk {
            dir_path: dir_vpk.to_path_buf(),
            data_base: header_size as u64 + tree_size as u64,
            entries,
        })
    }

    pub fn len(&self) -> usize { self.entries.len() }
    pub fn is_empty(&self) -> bool { self.entries.is_empty() }
    pub fn contains(&self, path: &str) -> bool { self.entries.contains_key(&path.to_lowercase()) }

    /// Read the first `max` bytes of a file. Headers are all we need, so this never pulls a
    /// whole model off disk.
    pub fn read_prefix(&self, path: &str, max: usize) -> Option<Vec<u8>> {
        let e = *self.entries.get(&path.to_lowercase())?;
        let want = (e.length as usize).min(max);

        // small files can live entirely in the preload block inside the dir file
        if e.length == 0 && e.preload_len > 0 {
            let mut f = File::open(&self.dir_path).ok()?;
            let mut buf = vec![0u8; (e.preload_len as usize).min(max)];
            f.seek(SeekFrom::Start(e.preload_offset as u64)).ok()?;
            f.read_exact(&mut buf).ok()?;
            return Some(buf);
        }

        let (path_buf, base) = if e.archive == INLINE_ARCHIVE {
            (self.dir_path.clone(), self.data_base + e.offset as u64)
        } else {
            // pak01_dir.vpk -> pak01_000.vpk
            let stem = self.dir_path.file_name()?.to_string_lossy().replace("_dir.vpk", "");
            let sibling = self.dir_path.with_file_name(format!("{stem}_{:03}.vpk", e.archive));
            (sibling, e.offset as u64)
        };

        let mut f = File::open(&path_buf).ok()?;
        let mut buf = vec![0u8; want];
        f.seek(SeekFrom::Start(base)).ok()?;
        f.read_exact(&mut buf).ok()?;
        Some(buf)
    }
}

/// Every `*_dir.vpk` sitting beside a map's game folder, newest-looking first.
pub fn find_archives(maps_dir: &Path) -> Vec<PathBuf> {
    let Some(game_dir) = maps_dir.parent() else { return Vec::new() };
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(game_dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.file_name().map_or(false, |n| n.to_string_lossy().ends_with("_dir.vpk")) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}
