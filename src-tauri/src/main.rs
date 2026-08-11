// Tauri shell. All the work happens in pixelsurf-core; this only exposes it to the webview.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use pixelsurf_core::{maps, scan_map, spots::ScanOptions};

/// Every map found on disk, plus the folders searched (so an empty list is explainable
/// rather than just empty).
#[tauri::command]
fn list_maps() -> Result<Vec<String>, String> {
    let dirs = maps::map_dirs();
    if dirs.is_empty() {
        return Err("No maps folder found. Set the PIXELSURF_MAPS environment variable to a \
                    folder containing .bsp files, then restart.".into());
    }
    Ok(maps::list_maps(&dirs))
}

#[tauri::command]
fn map_folders() -> Vec<String> {
    maps::map_dirs().iter().map(|p| p.display().to_string()).collect()
}

/// Scan a map. Cached per .bsp, so repeat calls for the same map return instantly.
#[tauri::command]
fn scan(map: String, include_ground: bool, include_trim: bool, include_surf: bool,
        min_oob: f64, force: bool) -> Result<String, String> {
    let opts = ScanOptions { include_ground, include_trim, include_surf, min_oob_height: min_oob };
    scan_map(&map, &opts, force)
}

#[tauri::command]
fn clear_cache() -> Result<String, String> {
    maps::clear_cache().map_err(|e| e.to_string())?;
    Ok(maps::cache_dir().display().to_string())
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![list_maps, map_folders, scan, clear_cache])
        .run(tauri::generate_context!())
        .expect("failed to start Pixelsurf Calc");
}
