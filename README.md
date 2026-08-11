# Pixelsurf Calc

Finds **pixel surfs**, **pixel walks** and **out-of-bounds / flashboost spots** in CS:GO maps
by reading the map's collision geometry — then tells you exactly where to stand and what to
press to get onto each one.

The reference tool for this, [Pixurf](https://github.com/HackerPide/Pixurf), does the
arithmetic but knows nothing about maps: you read the ledge height off the wireframe in-game,
read your own eye height off `cl_showpos`, and type both in. This reads both out of the `.bsp`
instead, so you pick a map and get a list.

> **Status: early.** The scan engine, the CLI and the calculator work and are tested. The GUI
> is written but has not been run yet — it needs a C toolchain this machine doesn't have (see
> [Install](#install)). The 3D view, the approach/speed tab and the input-route simulator are
> not built. See [Limitations](#limitations) — some of them matter.

---

## What it finds

| Kind | Meaning |
|---|---|
| **pixelsurf** | A sliver too small for the engine to give you ground. You are technically airborne and perched. |
| **pixelwalk** | Standable (`normal.z >= 0.7`) but narrower than the 32u player hull — a thin ledge you can stand and walk on. |
| **surf** | `normal.z < 0.7`. You slide instead of standing (e.g. the de_biome dome). |
| **out of bounds** | Any of the above that a walk/jump flood-fill from the spawns never reached — i.e. somewhere you need a boost or a flashboost to get to. |

Out-of-bounds spots are tagged with *why*:

- `unclipped-geometry` — real geometry up high that nobody clipped off; the "mapmaker never
  thought anyone would stand here" case
- `clip-gap` — a player-clip brush you can stand on (an invisible ledge)
- `surf-ramp` — a large steep surface you ride rather than stand on

---

## Install

Needs [Rust](https://rustup.rs/).

### CLI only — no C toolchain needed, builds in seconds

`core/` has zero dependencies, so it links with the `lld` that rustup already ships. Nothing
else to install:

```bash
cd core
cargo build --release
./target/release/pixelsurf maps
```

### GUI — needs a working system linker

Tauri pulls in `windows-sys`, `parking_lot` and friends, which need a real C linker. Rust
alone is **not** enough on Windows. Install one of:

**Option A — MSVC (recommended on Windows).** Install
[Visual Studio Build Tools](https://visualstudio.microsoft.com/downloads/) and tick
*Desktop development with C++*, then:

```bash
rustup default stable-x86_64-pc-windows-msvc
```

and delete `src-tauri/rust-toolchain.toml` (it pins the GNU target).

**Option B — MinGW-w64.** Install [MSYS2](https://www.msys2.org/), then
`pacman -S mingw-w64-x86_64-toolchain`, and put `C:\msys64\mingw64\bin` on `PATH`. The
`rust-toolchain.toml` pin then works as-is.

Then:

```bash
cd src-tauri
cargo run --release
```

> **Known failure.** If you see `dlltool.exe: CreateProcess` or
> `link.exe not found`, you have Rust but no C toolchain — that is exactly the case above.
> rustup's `self-contained` folder ships `dlltool` *without* the GNU assembler it shells out
> to, so the GNU target fails even though `dlltool.exe` exists on disk.

---

## Usage

```bash
pixelsurf maps                       # list every map found on disk
pixelsurf scan cs_italy              # scan a whole map
pixelsurf scan de_biome --top 20     # show more rows
pixelsurf scan cs_italy --force      # ignore the cache and rebuild
pixelsurf scan cs_italy --json       # machine-readable output
pixelsurf solve 1000                 # manual mode: one ledge height
pixelsurf jumps --crouch             # the discrete heights a jump passes through
pixelsurf clear-cache
```

Scan flags: `--ground` (include ordinary floor), `--trim` (include window sills and step
edges — thousands per map), `--no-surf`, `--min-oob N` (default 40u).

### Where maps are found

Checked in order, all that exist are used:

- `%USERPROFILE%\Desktop\ClassicCounter\csgo\maps`
- `…\Steam\steamapps\common\Counter-Strike Global Offensive\csgo\maps`
- `…\Steam\steamapps\common\Counter-Strike Source\cstrike\maps`

Override or extend with the `PIXELSURF_MAPS` environment variable (`;`-separated).

### Caching

Scans are cached in `%LOCALAPPDATA%\pixelsurf-calc\cache`, keyed on the `.bsp`'s size and
mtime — replace a map file and its cache invalidates itself. A cached map returns instantly;
a cold scan of a 400 MB map takes well under a second.

---

## Reading the output

```
kind              x         y         z    above  why                 width
pixelwalk     621.0     232.0     754.0     722u  unclipped-geometry  16.0u
```

`above` is the height over the nearest surface the flood fill *could* reach — roughly how big
a boost you need. `width` is the narrowest horizontal extent of the merged surface.

For reachable ledges you also get the way in:

```
pixelsurf     732.0    -756.5    -109.5   1.0u   Solo @ eye -111.41, crouch 66.00u
```

Stand where `cl_showpos` reads **-111.41**, then crouch jump.

---

## How it works

- **Collision geometry, not render geometry.** The engine collides the player hull against
  brush *planes*, and the perch you want is often exactly where collision and rendering
  disagree. Each brush is rebuilt as a convex polyhedron from its own side planes, at full
  float precision — a pixel surf is a sub-unit feature, so nothing is quantised.
- **Faces are merged into patches first.** The BSP compiler shatters one flat surface into
  many faces; classifying raw faces turns a single wall top into hundreds of "ledges".
- **The jump table is derived, not copied.** Source integrates gravity in two half-steps
  around the move, from `v = sqrt(2·800·57)`. Sampling that at 1/128 and reading down from the
  apex reproduces Pixurf's 42 reference heights within 0.01u and its 64-vs-128 tickrate column
  exactly. (Pixurf's numbers turn out to be in-game *measurements* — their residuals scatter
  in both directions, which a computed table cannot do — so the derived values are the more
  accurate of the two.)

---

## Limitations

Read these before trusting a scan.

- **Static prop collision is not read.** `.phy` parsing is not implemented, so spots on
  crates, awnings, pipes and other props are **missed, not merely unranked**. On maps like
  `cs_italy` that is a large fraction of the real spots.
- **The reachability fill is approximate.** It models walking, stepping and a crouch jump with
  a ballistic horizontal reach. It does not model bhopping, surfing to a destination, ladders,
  or boosts, so some spots marked out-of-bounds are reachable by a good player.
- **Moving brush entities** (doors, lifts) are analysed at their compiled position, not their
  in-game one.
- **CS:GO / Source 1 only.** VBSP maps. CS2 (Source 2) is a different format and is not
  supported. `.bsp.bz2` must be extracted first.
- **Nothing here is verified in-game.** The geometry and the jump maths are tested; whether a
  given spot is actually standable has not been confirmed by playing it.

---

## Layout

```
core/         zero-dependency Rust engine + CLI
  src/bsp.rs        VBSP lump reader
  src/collide.rs    brushes -> convex polyhedra, displacements
  src/spots.rs      patch merging, classification, reachability
  src/jumptable.rs  derived jump heights + boost stacks
  src/maps.rs       map discovery + result cache
src-tauri/    Tauri 2 shell (thin: exposes core over IPC)
frontend/     the UI
lib/, test/   JavaScript reference implementation, kept as the conformance oracle
              for the Rust port (234 assertions)
```

## Credits

- [HackerPide/Pixurf](https://github.com/HackerPide/Pixurf) (GPL-3.0) — the original
  calculator. Used here only as a test oracle; the jump table in this project is derived from
  the engine constants, not copied.

## Licence

MIT — see [LICENSE](LICENSE).
