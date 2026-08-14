// Tauri shell. All the work happens in pixelsurf-core; this only exposes it to the webview.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use pixelsurf_core::{maps, scan_map, spots::ScanOptions};

/// Every map found on disk, plus the folders searched (so an empty list is explainable
/// rather than just empty).
#[tauri::command]
fn list_maps() -> Result<Vec<String>, String> {
    let dirs = maps::map_dirs();
    if dirs.is_empty() {
        return Err("No maps folder found. Use \"Add folder\" and point it at your \
                    csgo/maps (or cstrike/maps) directory.".into());
    }
    Ok(maps::list_maps(&dirs))
}

#[tauri::command]
fn map_folders() -> Vec<String> {
    maps::map_dirs().iter().map(|p| p.display().to_string()).collect()
}

/// Remember an extra maps folder — the escape hatch for installs the search doesn't find.
#[tauri::command]
fn add_map_dir(dir: String) -> Result<bool, String> {
    maps::add_map_dir(&dir)
}

/// Scan a map. Cached per .bsp, so repeat calls for the same map return instantly.
#[tauri::command]
fn scan(map: String, include_ground: bool, include_trim: bool, include_surf: bool,
        min_oob: f64, force: bool) -> Result<String, String> {
    let opts = ScanOptions { include_ground, include_trim, include_surf, min_oob_height: min_oob };
    scan_map(&map, &opts, force)
}

/// Render geometry for the 3D tab: base64 i16 triples, uploaded straight to WebGL.
#[tauri::command]
fn map_mesh(map: String) -> Result<String, String> {
    pixelsurf_core::map_mesh(&map)
}

#[tauri::command]
fn clear_cache() -> Result<String, String> {
    maps::clear_cache().map_err(|e| e.to_string())?;
    Ok(maps::cache_dir().display().to_string())
}

/// Is the Microsoft Edge WebView2 runtime installed?
///
/// Tauri renders through the system WebView2 rather than shipping its own browser — that is
/// why the app is a few MB instead of a few hundred. The cost is that on Windows 10 machines
/// that never got it, there is nothing to render into and the process dies with no window and
/// no message. The NSIS installer handles this (see `webviewInstallMode` in tauri.conf.json),
/// but someone running the raw .exe out of target/release gets nothing, so check explicitly.
///
/// Uses reg.exe rather than a registry crate, same as the Steam lookup in core::maps.
#[cfg(windows)]
fn webview2_installed() -> bool {
    const CLIENT: &str = "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";
    for hive in ["HKLM\\SOFTWARE\\WOW6432Node\\Microsoft\\EdgeUpdate\\Clients",
                 "HKLM\\SOFTWARE\\Microsoft\\EdgeUpdate\\Clients",
                 "HKCU\\SOFTWARE\\Microsoft\\EdgeUpdate\\Clients"] {
        let Ok(out) = std::process::Command::new("reg")
            .args(["query", &format!("{hive}\\{CLIENT}"), "/v", "pv"]).output() else { continue };
        if !out.status.success() { continue; }
        let text = String::from_utf8_lossy(&out.stdout);
        // an empty or 0.0.0.0 version means the key exists but nothing is actually installed
        if let Some(line) = text.lines().find(|l| l.contains("pv")) {
            if let Some(i) = line.find("REG_SZ") {
                let v = line[i + 6..].trim();
                if !v.is_empty() && v != "0.0.0.0" { return true; }
            }
        }
    }
    false
}

/// No console is attached (windows_subsystem = "windows"), so stderr goes nowhere. Put the
/// message somewhere the user will actually see it.
#[cfg(windows)]
fn show_error(msg: &str) {
    let script = format!(
        "Add-Type -AssemblyName PresentationFramework; \
         [System.Windows.MessageBox]::Show('{}','Pixelsurf Calc',0,16) | Out-Null",
        msg.replace('\'', "''"));
    let _ = std::process::Command::new("powershell")
        .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &script])
        .status();
}

fn main() {
    #[cfg(windows)]
    if !webview2_installed() {
        show_error(
            "Pixelsurf Calc needs the Microsoft Edge WebView2 runtime, which is not installed \
             on this PC.\n\nInstall it with:\n    winget install Microsoft.EdgeWebView2Runtime\n\n\
             Or download the Evergreen Standalone Installer from:\n\
             https://developer.microsoft.com/microsoft-edge/webview2/\n\n\
             (The .msi/.exe installer for this app installs it for you — this only affects \
             running the raw .exe.)");
        std::process::exit(1);
    }

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![list_maps, map_folders, add_map_dir, scan, map_mesh, clear_cache])
        .run(tauri::generate_context!())
        .expect("failed to start Pixelsurf Calc");
}
