//! CLI front end. Everything the GUI shows is computed here first, so the engine can be
//! exercised and validated without any UI in the way.

use pixelsurf_core::consts::*;
use pixelsurf_core::spots::{Kind, ScanOptions};
use pixelsurf_core::{jumptable, maps, scan_map, spots};

fn usage() -> ! {
    eprintln!(
"pixelsurf — CS:GO pixel surf / pixel walk / boost spot finder

  pixelsurf maps
        list every map found on disk (Steam libraries are read from Steam's own index,
        so any drive works)

  pixelsurf add-dir <path>
        add your own maps folder, remembered for next time

  pixelsurf scan <map> [--ground] [--no-surf] [--min-oob N] [--json] [--force] [--top N]
        scan a whole map for pixel surfs, pixel walks, surf ramps and out-of-bounds spots
        results are cached per map; --force rescans

  pixelsurf solve <ledgeZ> [--min N] [--max N] [--tick 64|128]
        manual mode: given one ledge height, every way to reach it

  pixelsurf jumps [--crouch]
        the discrete heights a jump passes through, and the tickrates each exists on

  pixelsurf clear-cache
");
    std::process::exit(2)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() { usage(); }
    let has = |f: &str| args.iter().any(|a| a == f);
    let val = |f: &str| -> Option<f64> {
        args.iter().position(|a| a == f).and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok())
    };

    match args[0].as_str() {
        "maps" => {
            let dirs = maps::map_dirs();
            if dirs.is_empty() {
                eprintln!("No maps folder found.\n\
                    Point it at yours:  pixelsurf add-dir \"D:\\path\\to\\csgo\\maps\"\n\
                    or set PIXELSURF_MAPS (';'-separated).");
                std::process::exit(1);
            }
            for d in &dirs { eprintln!("searching {}", d.display()); }
            let list = maps::list_maps(&dirs);
            eprintln!("{} maps  (add a folder with: pixelsurf add-dir <path>)\n", list.len());
            for m in list { println!("{m}"); }
        }

        "scan" => {
            let Some(name) = args.get(1).filter(|s| !s.starts_with("--")) else { usage() };
            let opts = ScanOptions {
                include_ground: has("--ground"),
                include_surf: !has("--no-surf"),
                include_trim: has("--trim"),
                min_oob_height: val("--min-oob").unwrap_or(40.0),
            };
            let json = match scan_map(name, &opts, has("--force")) {
                Ok(j) => j,
                Err(e) => { eprintln!("error: {e}"); std::process::exit(1); }
            };
            if has("--json") { println!("{json}"); return; }
            print_scan(name, &opts, has("--force"), val("--top").unwrap_or(40.0) as usize);
        }

        "solve" => {
            let Some(ledge) = args.get(1).and_then(|v| v.parse::<f64>().ok()) else { usage() };
            let min = val("--min").unwrap_or(ledge - 80.0);
            let max = val("--max").unwrap_or(ledge + 80.0);
            let tick = val("--tick").map(|v| v as u32);
            let sols = jumptable::solutions(ledge, min, max, tick);
            println!("ledge z = {ledge:.2}   searching stand-eye {min:.2}..{max:.2}");
            println!("{} way(s) in:\n", sols.len());
            println!("  {:<28} {:>10}  {:<14} {}", "boost", "stand eye", "jump", "tickrate");
            for s in sols.iter().take(60) {
                let jump = match s.jump {
                    None => "walk off".to_string(),
                    Some(h) => format!("{}{:.2}u", if s.crouch { "crouch " } else { "" }, h),
                };
                println!("  {:<28} {:>10.2}  {:<14} {}", s.label, s.stand_eye, jump,
                    if s.tickrates.contains(&64) { "64 + 128" } else { "128 only" });
            }
            if sols.len() > 60 { println!("  ... and {} more", sols.len() - 60); }
            println!("\n(stand eye = the z cl_showpos must read. eye offsets: {EYE_STAND} standing, {EYE_DUCK} crouched)");
        }

        "jumps" => {
            let crouch = has("--crouch");
            println!("{} jump — feet height above where you took off:",
                if crouch { "crouch" } else { "normal" });
            for j in jumptable::table(crouch, 14.0) {
                println!("  {:>6.2}u   {}", j.h,
                    if j.tickrates.contains(&64) { "64 + 128" } else { "128 only" });
            }
        }

        "add-dir" => {
            let Some(dir) = args.get(1) else {
                eprintln!("usage: pixelsurf add-dir <folder containing .bsp files>");
                eprintln!("config: {}", maps::config_path().display());
                for d in maps::configured_dirs() { eprintln!("  {}", d.display()); }
                std::process::exit(2);
            };
            match maps::add_map_dir(dir) {
                Ok(true) => println!("added {dir}\nsaved to {}", maps::config_path().display()),
                Ok(false) => println!("{dir} was already configured"),
                Err(e) => { eprintln!("error: {e}"); std::process::exit(1); }
            }
        }

        "view" => {
            let Some(name) = args.get(1).filter(|s| !s.starts_with("--")) else { usage() };
            let opts = ScanOptions {
                include_ground: has("--ground"),
                include_surf: !has("--no-surf"),
                include_trim: has("--trim"),
                min_oob_height: val("--min-oob").unwrap_or(40.0),
            };
            let out = args.iter().position(|a| a == "-o").and_then(|i| args.get(i + 1))
                .cloned().unwrap_or_else(|| format!("{name}.html"));
            match pixelsurf_core::build_viewer(name, &opts) {
                Ok((html, tris, spots)) => {
                    if let Err(e) = std::fs::write(&out, html) {
                        eprintln!("could not write {out}: {e}");
                        std::process::exit(1);
                    }
                    let kb = std::fs::metadata(&out).map(|m| m.len() / 1024).unwrap_or(0);
                    println!("wrote {out}  ({tris} triangles, {spots} spots, {kb} KB)");
                    println!("open it in a browser — no server needed");
                }
                Err(e) => { eprintln!("error: {e}"); std::process::exit(1); }
            }
        }

        "clear-cache" => {
            match maps::clear_cache() {
                Ok(()) => println!("cache cleared ({})", maps::cache_dir().display()),
                Err(e) => { eprintln!("could not clear cache: {e}"); std::process::exit(1); }
            }
        }

        _ => usage(),
    }
}

/// Re-runs the scan (cheap: it will hit the cache the CLI just wrote) and prints it readably.
fn print_scan(name: &str, opts: &ScanOptions, force: bool, top: usize) {
    let dirs = maps::map_dirs();
    let Some(path) = maps::find_bsp(name, &dirs) else { return };
    let geo = match pixelsurf_core::collide::extract(&path) {
        Ok(g) => g, Err(e) => { eprintln!("error: {e}"); std::process::exit(1); }
    };
    let res = spots::scan(&geo, opts);
    let found = &res.spots;
    let _ = force;

    let count = |k: Kind| found.iter().filter(|x| x.kind == k).count();
    let oob: Vec<_> = found.iter().filter(|x| !x.reachable).collect();

    // Print the actual file. Several games ship a cs_italy, and which one you get depends on
    // folder order — without this the header silently changes map underneath you.
    println!("\n{name}  (bsp v{}, {} spawns)", geo.version, geo.spawns.len());
    println!("  {}", path.display());
    println!("  pixelsurf {}   pixelwalk {}   surf {}   out-of-bounds {}",
        count(Kind::PixelSurf), count(Kind::PixelWalk), count(Kind::Surf), oob.len());
    println!("  rejected {} surfaces where no player hull fits (wall or ceiling in the way)", res.blocked);
    println!("  NOT SCANNED: static prop collision (.phy) — spots on crates, awnings and pipes are missing.");

    if !oob.is_empty() {
        println!("\n  OUT OF BOUNDS — surfaces the fill never reached, highest first:");
        println!("    {:<12} {:>9} {:>9} {:>9}  {:>7} {:<20} {}", "kind", "x", "y", "z", "above", "why", "width");
        for s in oob.iter().take(top) {
            println!("    {:<12} {:>9.1} {:>9.1} {:>9.1}  {:>7} {:<20} {:.1}u",
                s.kind.as_str(), s.pos[0], s.pos[1], s.pos[2],
                if s.height_above_reachable < 0.0 { "-".into() }
                    else { format!("{:.0}u", s.height_above_reachable) },
                s.oob_class.unwrap_or("-"), s.width);
        }
    }

    let narrow: Vec<_> = found.iter()
        .filter(|s| s.reachable && matches!(s.kind, Kind::PixelSurf | Kind::PixelWalk)).collect();
    if !narrow.is_empty() {
        println!("\n  REACHABLE LEDGES — narrowest first:");
        println!("    {:<12} {:>9} {:>9} {:>9}  {:>7}  {}", "kind", "x", "y", "z", "width", "best way in");
        for s in narrow.iter().take(top) {
            let e = s.entries.first();
            let how = match e {
                None => "-".to_string(),
                Some(e) => format!("{} @ eye {:.2}{}", e.label, e.stand_eye,
                    match e.jump { None => " (walk off)".into(),
                        Some(j) => format!(", {}{:.2}u", if e.crouch { "crouch " } else { "" }, j) }),
            };
            println!("    {:<12} {:>9.1} {:>9.1} {:>9.1}  {:>6.1}u  {}",
                s.kind.as_str(), s.pos[0], s.pos[1], s.pos[2], s.width, how);
        }
    }
    println!("\n  cache: {}", maps::cache_dir().display());
}
