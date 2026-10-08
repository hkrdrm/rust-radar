# rust-radar Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A local Rust service that polls NOAA MRMS radar and NHC hurricane data, archives it, and serves a zoomable, clickable MapLibre map with live view and replay.

**Architecture:**
- **Background pollers.** Tokio tasks fetch MRMS composite reflectivity (GRIB2) and NHC storm data through a mockable `Fetcher` trait. They store validated files in a rolling on-disk archive.
- **HTTP server.** An axum server renders 256px Web Mercator PNG tiles from decoded grids on demand (with LRU caches). It serves storm GeoJSON for any moment in time, and serves a static MapLibre frontend.

**Tech Stack:** Rust 2021 (toolchain 1.99), tokio, axum 0.8, reqwest 0.12 (rustls), grib 0.19 (PNG unpacking), png 0.17, flate2, lru 0.12, shapefile 0.6 + zip 2, serde/serde_json, toml, clap 4, chrono, tracing; MapLibre GL JS 4.7.1 from unpkg.

**Spec:** `docs/superpowers/specs/2026-10-08-rust-radar-design.md`

**Deviations from the spec, decided while planning:**
- **Archive split.** `archive.rs` is split into `mrms_archive.rs` and `nhc_archive.rs` (one job each).
- **New endpoint.** `GET /api/config` is added so the frontend can read the configured basemap URL.
- **Compact grid storage.** Decoded grids store dBZ packed into one byte per cell (0.5 dBZ steps). Storing `f32` would take 98 MB per frame, so a 10-frame cache would be about 1 GB. Packed, it is 24.5 MB per frame.
- **Limited zip fallback.** The NHC zip fallback reads only the cone, forecast track and forecast points (the 5-day zip). Wind radii and past track come only from the map service.

**Facts verified on 2026-10-08 (against live endpoints):**
- **`grib` decoding.** `grib` 0.19 with `png-unpack-with-png-crate` decodes a live MRMS file in under 1 s (release build):
  - `grid_shape()` returns `(7000, 3500)`, meaning `(nx, ny)`.
  - The first lat/lon is `(54.995, -129.995)`, and rows run north to south.
  - Values range from −999 to about 60.
- **MRMS live listing.** `https://mrms.ncep.noaa.gov/2D/MergedReflectivityQCComposite/` lists `MRMS_MergedReflectivityQCComposite_00.50_YYYYMMDD-HHMMSS.grib2.gz`. It also lists a `...latest...` file, which must be ignored.
- **MRMS on S3.** `https://noaa-mrms-pds.s3.amazonaws.com/?list-type=2&prefix=CONUS/MergedReflectivityQCComposite_00.50/YYYYMMDD/` returns XML with the same file names. There are about 720 files per day, which fits in one page (the S3 page limit is 1000).
- **NHC `CurrentStorms.json`.**
  - Each entry has `activeStorms[].{id, binNumber, name, classification, intensity (kt, string), pressure (string), latitudeNumeric, longitudeNumeric, movementDir, movementSpeed, lastUpdate, publicAdvisory.{advNum, issuance, url}, forecastTrack.zipFile}`.
  - Numbers are sometimes strings.
- **NHC map service.**
  - The map service is at `https://mapservices.weather.noaa.gov/tropical/rest/services/tropical/NHC_tropical_weather/MapServer`.
  - `?f=json` lists layers named like `"AT4 Forecast Cone"`, keyed by `binNumber`.
  - `/{id}/query?where=1%3D1&outFields=*&f=geojson` returns a GeoJSON FeatureCollection.
- **Map service field names.**
  - Forecast points use `maxwind` (kt) and past points use `intensity` (kt).
  - Wind radii have `radii` (34/50/64).
  - The cone and forecast points have `advisnum` (for example `"8"`).
- **5-day zip.** The 5-day zip contains `*_5day_pgn.{shp,dbf}` (cone), `*_5day_lin.*` (track) and `*_5day_pts.*` (points).

## Global Constraints

- Rust edition 2021. Must build and test with `cargo` 1.99 on Linux.
- Server binds `127.0.0.1` only (local, single user; no auth).
- Config defaults, verbatim from the spec:
  - `port = 8080`
  - `archive_dir = "./archive"`
  - `retention_hours = 72`
  - `backfill_hours = 6`
  - `mrms_poll_secs = 60`
  - `nhc_poll_secs = 300`
  - `decoded_cache_frames = 10`
  - `basemap_style = "https://tiles.openfreemap.org/styles/dark"`
- MRMS product: `MergedReflectivityQCComposite_00.50`.
- MRMS `-999` (no coverage) and `-99` (missing) render transparent, never as "no precipitation".
- Tiles are 256×256 PNG, Web Mercator. Max zoom is 12.
- Files enter the archive only after they decode successfully: write to a temp file, then atomically rename.
- Status badge thresholds: amber if radar is more than 10 min old or NHC more than 60 min old.
- Tests never touch the network. Pollers are tested through `FakeFetcher`.
- Upstream fetch failures never crash the process. Pollers retry with capped exponential backoff (max 15 min).
- Every commit message ends with a blank line and `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. The `git commit` commands below pass it as a second `-m`.

## Review Focus

These are the most likely ways a real user could break the app. None of them is the main path of any task, so each one gets a dedicated test in the task listed.

1. **Malformed or unknown `time` in a tile or storm URL.** Examples: `yesterday`, or a frame that was pruned. Expected: 400 or 404 with a JSON error, never a 500 or a panic. Covered in Task 12.
2. **Out-of-range tile coordinates.** Examples: `x ≥ 2^z`, `z > 12`, a missing `.png` suffix, or non-numeric `z`. Expected: 400. Covered in Tasks 4 and 12.
3. **Odd NHC feed entries.** Examples: numbers sent as strings, an empty `intensity`, or a storm entry missing required fields. Expected: the other storms still parse. Covered in Task 8.
4. **Crash leftovers or stray files in the archive.** Examples: `*.tmp` files or unrelated files. Expected: ignored by listings and cleaned on open or prune. Covered in Tasks 5 and 9.
5. **A backfill window that spans UTC midnight.** Expected: both days' S3 listings are fetched. Covered in Task 7.

Also pinned:
- **Path traversal.** A storm id like `../..` returns 404 (Tasks 9 and 12).
- **Lagging map service.** If the map service still shows the previous advisory, fall back to the zip (Task 11).

---

## File Structure

```
rust-radar/
  Cargo.toml
  rust-radar.toml           example config (Task 13)
  README.md                 (Task 13)
  .gitignore                (Task 1)
  src/
    lib.rs                  module list (grows each task)
    main.rs                 CLI entry: config, pollers, prune loop, server (Task 13)
    config.rs               Config (TOML + CLI overrides) + validation (Task 1)
    palette.rs              dBZ -> RGBA, NWS reflectivity scale (Task 2)
    grid.rs                 GRIB2 -> Grid (packed u8 dBZ), sampling, bounds (Task 3)
    tiles.rs                Web Mercator maths, tile rendering, PNG encode (Task 4)
    mrms_archive.rs         on-disk MRMS frames, decoded-grid LRU, prune (Task 5)
    fetch.rs                Fetcher trait, HttpFetcher, FakeFetcher (Task 6)
    status.rs               per-source status + backoff (Task 6)
    sources/mod.rs          (Task 7)
    sources/mrms.rs         listing parse, live poll, S3 backfill, run loop (Task 7)
    sources/nhc.rs          StormInfo + parsing (Task 8); map-service poller (Task 11)
    sources/nhc_zip.rs      shapefile-zip fallback (Task 10)
    nhc_archive.rs          per-storm advisory snapshots, time lookup, prune (Task 9)
    server.rs               axum routes, tile cache, API errors (Task 12)
  web/
    index.html, app.js, style.css   (Task 14)
  tests/
    common/mod.rs           fixture loaders + helpers (Task 3, extended in 9, 10)
    fixtures/mrms.grib2.gz  real MRMS frame (Task 3)
    fixtures/nhc_5day.zip   real NHC advisory zip (Task 10)
    grid.rs, tiles.rs, mrms_archive.rs, mrms_source.rs,
    nhc_parse.rs, nhc_archive.rs, nhc_zip.rs, nhc_source.rs, server.rs
```

---

### Task 1: Project scaffold and configuration

**Files:**
- Create: `Cargo.toml`, `.gitignore`, `src/lib.rs`, `src/main.rs`, `src/config.rs`

**Interfaces:**
- Produces:
  - `rust_radar::config::Config { port: u16, archive_dir: PathBuf, web_dir: PathBuf, retention_hours: u64, backfill_hours: u64, mrms_poll_secs: u64, nhc_poll_secs: u64, decoded_cache_frames: usize, basemap_style: String }`
  - `Config::load(&Cli) -> anyhow::Result<Config>` and `Config::validate(&self) -> anyhow::Result<()>`
  - `rust_radar::config::Cli` (clap; `Cli::parse()`)

- [ ] **Step 1: Create `Cargo.toml` with every dependency the project needs**

```toml
[package]
name = "rust-radar"
version = "0.1.0"
edition = "2021"

[dependencies]
anyhow = "1"
async-trait = "0.1"
axum = "0.8"
chrono = { version = "0.4", features = ["serde"] }
clap = { version = "4", features = ["derive"] }
flate2 = "1"
grib = { version = "0.19", default-features = false, features = ["png-unpack-with-png-crate"] }
lru = "0.12"
png = "0.17"
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
shapefile = "0.6"
tokio = { version = "1", features = ["full"] }
toml = "0.8"
tower-http = { version = "0.6", features = ["fs"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
zip = { version = "2", default-features = false, features = ["deflate"] }

[dev-dependencies]
http-body-util = "0.1"
tempfile = "3"
tower = { version = "0.5", features = ["util"] }

# Decoding a 24.5M-cell GRIB2 grid is slow without optimisation; keep test runs fast.
[profile.dev]
opt-level = 1

[profile.dev.package."*"]
opt-level = 3
```

`.gitignore`:

```
/target
/archive
```

`src/lib.rs`:

```rust
pub mod config;
```

`src/main.rs` (temporary; replaced in Task 13):

```rust
use clap::Parser;
use rust_radar::config::{Cli, Config};

fn main() -> anyhow::Result<()> {
    let config = Config::load(&Cli::parse())?;
    println!("{config:#?}");
    Ok(())
}
```

- [ ] **Step 2: Write the failing config tests**

Create `src/config.rs` with only the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn cli(args: &[&str]) -> Cli {
        Cli::parse_from(std::iter::once("rust-radar").chain(args.iter().copied()))
    }

    #[test]
    fn defaults_when_file_missing() {
        let c = Config::load(&cli(&["--config", "/nonexistent/rust-radar.toml"])).unwrap();
        assert_eq!(c, Config::default());
        assert_eq!(c.port, 8080);
        assert_eq!(c.archive_dir, std::path::PathBuf::from("./archive"));
        assert_eq!(c.retention_hours, 72);
        assert_eq!(c.backfill_hours, 6);
        assert_eq!(c.mrms_poll_secs, 60);
        assert_eq!(c.nhc_poll_secs, 300);
        assert_eq!(c.decoded_cache_frames, 10);
        assert_eq!(c.basemap_style, "https://tiles.openfreemap.org/styles/dark");
    }

    #[test]
    fn file_values_then_cli_overrides() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.toml");
        std::fs::write(&path, "port = 9000\nretention_hours = 24\n").unwrap();
        let c = Config::load(&cli(&[
            "--config",
            path.to_str().unwrap(),
            "--retention-hours",
            "48",
        ]))
        .unwrap();
        assert_eq!(c.port, 9000);
        assert_eq!(c.retention_hours, 48);
        assert_eq!(c.nhc_poll_secs, 300);
    }

    #[test]
    fn rejects_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.toml");
        std::fs::write(&path, "prot = 9000\n").unwrap();
        assert!(Config::load(&cli(&["--config", path.to_str().unwrap()])).is_err());
    }

    #[test]
    fn rejects_invalid_values() {
        let missing = "/nonexistent/rust-radar.toml";
        assert!(Config::load(&cli(&["--config", missing, "--decoded-cache-frames", "0"])).is_err());
        assert!(Config::load(&cli(&["--config", missing, "--mrms-poll-secs", "0"])).is_err());
        assert!(Config::load(&cli(&[
            "--config", missing, "--backfill-hours", "100", "--retention-hours", "24",
        ]))
        .is_err());
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test --lib config`
Expected: compile errors such as `cannot find type Cli` and `cannot find type Config`.

- [ ] **Step 4: Implement `Config` and `Cli` above the test module**

```rust
//! Runtime configuration: `rust-radar.toml`, overridden by CLI flags.

use std::path::PathBuf;

use anyhow::{ensure, Context};
use clap::Parser;
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub port: u16,
    pub archive_dir: PathBuf,
    pub web_dir: PathBuf,
    pub retention_hours: u64,
    pub backfill_hours: u64,
    pub mrms_poll_secs: u64,
    pub nhc_poll_secs: u64,
    pub decoded_cache_frames: usize,
    pub basemap_style: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            port: 8080,
            archive_dir: PathBuf::from("./archive"),
            web_dir: PathBuf::from("./web"),
            retention_hours: 72,
            backfill_hours: 6,
            mrms_poll_secs: 60,
            nhc_poll_secs: 300,
            decoded_cache_frames: 10,
            basemap_style: "https://tiles.openfreemap.org/styles/dark".to_string(),
        }
    }
}

#[derive(Debug, Parser)]
#[command(name = "rust-radar", about = "Local hurricane-tracking radar map")]
pub struct Cli {
    /// Path to the TOML config file (missing file = defaults)
    #[arg(long, default_value = "rust-radar.toml")]
    pub config: PathBuf,
    #[arg(long)]
    pub port: Option<u16>,
    #[arg(long)]
    pub archive_dir: Option<PathBuf>,
    #[arg(long)]
    pub web_dir: Option<PathBuf>,
    #[arg(long)]
    pub retention_hours: Option<u64>,
    #[arg(long)]
    pub backfill_hours: Option<u64>,
    #[arg(long)]
    pub mrms_poll_secs: Option<u64>,
    #[arg(long)]
    pub nhc_poll_secs: Option<u64>,
    #[arg(long)]
    pub decoded_cache_frames: Option<usize>,
    #[arg(long)]
    pub basemap_style: Option<String>,
}

impl Config {
    pub fn load(cli: &Cli) -> anyhow::Result<Config> {
        let mut config = match std::fs::read_to_string(&cli.config) {
            Ok(text) => toml::from_str(&text)
                .with_context(|| format!("parsing {}", cli.config.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Config::default(),
            Err(e) => {
                return Err(e).with_context(|| format!("reading {}", cli.config.display()))
            }
        };
        if let Some(v) = cli.port { config.port = v; }
        if let Some(v) = &cli.archive_dir { config.archive_dir = v.clone(); }
        if let Some(v) = &cli.web_dir { config.web_dir = v.clone(); }
        if let Some(v) = cli.retention_hours { config.retention_hours = v; }
        if let Some(v) = cli.backfill_hours { config.backfill_hours = v; }
        if let Some(v) = cli.mrms_poll_secs { config.mrms_poll_secs = v; }
        if let Some(v) = cli.nhc_poll_secs { config.nhc_poll_secs = v; }
        if let Some(v) = cli.decoded_cache_frames { config.decoded_cache_frames = v; }
        if let Some(v) = &cli.basemap_style { config.basemap_style = v.clone(); }
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        ensure!(self.mrms_poll_secs > 0, "mrms_poll_secs must be > 0");
        ensure!(self.nhc_poll_secs > 0, "nhc_poll_secs must be > 0");
        ensure!(self.decoded_cache_frames > 0, "decoded_cache_frames must be > 0");
        ensure!(self.retention_hours > 0, "retention_hours must be > 0");
        ensure!(
            self.backfill_hours <= self.retention_hours,
            "backfill_hours ({}) must not exceed retention_hours ({})",
            self.backfill_hours,
            self.retention_hours
        );
        Ok(())
    }
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib config`
Expected: 4 passed. The first build downloads and compiles all dependencies, which takes a few minutes.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock .gitignore src/
git commit -m "feat: scaffold crate with config loading" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Reflectivity colour palette

**Files:**
- Create: `src/palette.rs`
- Modify: `src/lib.rs` (add `pub mod palette;`)

**Interfaces:**
- Produces: `rust_radar::palette::dbz_to_rgba(dbz: f32) -> [u8; 4]`. Returns transparent `[0,0,0,0]` below 5 dBZ or for non-finite input. Otherwise returns the NWS colour with alpha 255.

- [ ] **Step 1: Write the failing tests**

Add `pub mod palette;` to `src/lib.rs`. Create `src/palette.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const TRANSPARENT: [u8; 4] = [0, 0, 0, 0];

    #[test]
    fn sentinels_and_weak_echo_are_transparent() {
        assert_eq!(dbz_to_rgba(-999.0), TRANSPARENT);
        assert_eq!(dbz_to_rgba(-99.0), TRANSPARENT);
        assert_eq!(dbz_to_rgba(4.9), TRANSPARENT);
        assert_eq!(dbz_to_rgba(f32::NAN), TRANSPARENT);
    }

    #[test]
    fn steps_follow_nws_scale() {
        assert_eq!(dbz_to_rgba(5.0), [4, 233, 231, 255]);
        assert_eq!(dbz_to_rgba(22.0), [2, 253, 2, 255]);
        assert_eq!(dbz_to_rgba(50.0), [253, 0, 0, 255]);
        assert_eq!(dbz_to_rgba(75.0), [253, 253, 253, 255]);
        assert_eq!(dbz_to_rgba(90.0), [253, 253, 253, 255]);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib palette`
Expected: compile error `cannot find function dbz_to_rgba`.

- [ ] **Step 3: Implement the palette above the test module**

```rust
//! NWS-style reflectivity colour scale.

/// (lower bound in dBZ, colour) in ascending order.
const STOPS: [(f32, [u8; 3]); 15] = [
    (5.0, [4, 233, 231]),
    (10.0, [1, 159, 244]),
    (15.0, [3, 0, 244]),
    (20.0, [2, 253, 2]),
    (25.0, [1, 197, 1]),
    (30.0, [0, 142, 0]),
    (35.0, [253, 248, 2]),
    (40.0, [229, 188, 0]),
    (45.0, [253, 149, 0]),
    (50.0, [253, 0, 0]),
    (55.0, [212, 0, 0]),
    (60.0, [188, 0, 0]),
    (65.0, [248, 0, 253]),
    (70.0, [152, 84, 198]),
    (75.0, [253, 253, 253]),
];

pub fn dbz_to_rgba(dbz: f32) -> [u8; 4] {
    if !dbz.is_finite() || dbz < STOPS[0].0 {
        return [0, 0, 0, 0];
    }
    let mut colour = STOPS[0].1;
    for (threshold, rgb) in STOPS {
        if dbz < threshold {
            break;
        }
        colour = rgb;
    }
    [colour[0], colour[1], colour[2], 255]
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib palette`
Expected: 2 passed.

- [ ] **Step 5: Commit**

```bash
git add src/lib.rs src/palette.rs
git commit -m "feat: NWS reflectivity palette" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: GRIB2 decoding into a compact grid

**Files:**
- Create: `src/grid.rs`, `tests/common/mod.rs`, `tests/grid.rs`, `tests/fixtures/mrms.grib2.gz`
- Modify: `src/lib.rs` (add `pub mod grid;`)

**Interfaces:**
- Produces:
  - `rust_radar::grid::Grid { nx: usize, ny: usize, lat0: f64, lon0: f64, dlat: f64, dlon: f64, values: Vec<u8> }`. Row 0 is the north row. `lat0`/`lon0` are the centre of cell (0,0). `dlat` is positive and rows go south.
  - `Grid::from_grib2_gz(&[u8]) -> anyhow::Result<Grid>` and `Grid::from_grib2(&[u8]) -> anyhow::Result<Grid>`
  - `Grid::sample(&self, lat: f64, lon: f64) -> Option<f32>` (nearest neighbour; `None` outside the grid or where there is no echo)
  - `Grid::bounds(&self) -> (f64, f64, f64, f64)`, returning `(south, west, north, east)` outer edges
  - `Grid::cell_center(&self, index: usize) -> (f64, f64)`, returning `(lat, lon)`
  - `grid::NO_DATA: u8 = 0`, `grid::encode_dbz(f32) -> u8`, `grid::decode_dbz(u8) -> Option<f32>`
  - Test helpers in `tests/common/mod.rs`:
    - `mrms_fixture() -> Vec<u8>`
    - `decode_png(&[u8]) -> Vec<u8>` (RGBA)
    - `opaque_pixels(&[u8]) -> usize`
    - `rainy_tile(&Grid, z: u8) -> (u32, u32)`. Its body is filled in during Task 4, when `tiles::latlon_to_tile` exists.

- [ ] **Step 1: Download the MRMS fixture**

```bash
mkdir -p tests/fixtures
F=$(curl -sL https://mrms.ncep.noaa.gov/2D/MergedReflectivityQCComposite/ | grep -o 'MRMS_MergedReflectivityQCComposite_00\.50_[0-9-]*\.grib2\.gz' | tail -1)
curl -sL -o tests/fixtures/mrms.grib2.gz "https://mrms.ncep.noaa.gov/2D/MergedReflectivityQCComposite/$F"
ls -l tests/fixtures/mrms.grib2.gz   # expect roughly 1 MB
```

- [ ] **Step 2: Write the failing tests**

Add `pub mod grid;` to `src/lib.rs`.

`tests/common/mod.rs`:

```rust
#![allow(dead_code)]

use rust_radar::grid::{decode_dbz, Grid};

pub fn mrms_fixture() -> Vec<u8> {
    std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/mrms.grib2.gz"))
        .expect("tests/fixtures/mrms.grib2.gz missing - see Task 3 Step 1")
}

/// Decodes a PNG into raw RGBA bytes.
pub fn decode_png(png_bytes: &[u8]) -> Vec<u8> {
    let decoder = png::Decoder::new(png_bytes);
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size()];
    reader.next_frame(&mut buf).unwrap();
    buf
}

pub fn opaque_pixels(rgba: &[u8]) -> usize {
    rgba.chunks(4).filter(|p| p[3] > 0).count()
}

/// Index of the first cell with at least 30 dBZ in the grid.
pub fn rainy_cell(grid: &Grid) -> usize {
    grid.values
        .iter()
        .position(|&v| decode_dbz(v).is_some_and(|d| d >= 30.0))
        .expect("fixture has no echo >= 30 dBZ; re-download it when it is raining somewhere")
}
```

`tests/grid.rs`:

```rust
mod common;

use rust_radar::grid::{decode_dbz, Grid};

#[test]
fn decodes_mrms_fixture() {
    let g = Grid::from_grib2_gz(&common::mrms_fixture()).unwrap();
    assert_eq!((g.nx, g.ny), (7000, 3500));
    assert!((g.lat0 - 54.995).abs() < 1e-6, "lat0 = {}", g.lat0);
    assert!((g.lon0 + 129.995).abs() < 1e-6, "lon0 = {}", g.lon0);
    assert!((g.dlat - 0.01).abs() < 1e-6, "dlat = {}", g.dlat);
    assert!((g.dlon - 0.01).abs() < 1e-6, "dlon = {}", g.dlon);
    let (s, w, n, e) = g.bounds();
    assert!((s - 20.0).abs() < 1e-3 && (n - 55.0).abs() < 1e-3, "lat bounds {s}..{n}");
    assert!((w + 130.0).abs() < 1e-3 && (e + 60.0).abs() < 1e-3, "lon bounds {w}..{e}");
    let max = g.values.iter().filter_map(|&v| decode_dbz(v)).fold(0.0f32, f32::max);
    assert!(max > 20.0 && max <= 90.0, "max dBZ {max}");
    common::rainy_cell(&g);
}

#[test]
fn rejects_garbage() {
    assert!(Grid::from_grib2_gz(b"definitely not gzip").is_err());
}

#[test]
fn rejects_truncated_file() {
    let bytes = common::mrms_fixture();
    assert!(Grid::from_grib2_gz(&bytes[..bytes.len() / 2]).is_err());
}
```

Unit tests at the bottom of `src/grid.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// 3x2 grid: centres at lat 30/29, lon -90/-89/-88.
    fn tiny() -> Grid {
        Grid {
            nx: 3,
            ny: 2,
            lat0: 30.0,
            lon0: -90.0,
            dlat: 1.0,
            dlon: 1.0,
            values: vec![
                encode_dbz(10.0), encode_dbz(20.0), NO_DATA,
                encode_dbz(30.0), encode_dbz(40.0), encode_dbz(50.0),
            ],
        }
    }

    #[test]
    fn encode_decode_roundtrip() {
        assert_eq!(decode_dbz(encode_dbz(58.5)), Some(58.5));
        assert_eq!(decode_dbz(encode_dbz(0.0)), Some(0.0));
        assert_eq!(decode_dbz(encode_dbz(500.0)), Some(127.0));
        assert_eq!(encode_dbz(-999.0), NO_DATA);
        assert_eq!(encode_dbz(-99.0), NO_DATA);
        assert_eq!(encode_dbz(-5.0), NO_DATA);
        assert_eq!(encode_dbz(f32::NAN), NO_DATA);
    }

    #[test]
    fn sample_picks_nearest_cell() {
        let g = tiny();
        assert_eq!(g.sample(30.2, -89.9), Some(10.0));
        assert_eq!(g.sample(29.0, -88.0), Some(50.0));
        assert_eq!(g.sample(30.0, -88.0), None, "NO_DATA cell");
    }

    #[test]
    fn sample_outside_grid_is_none() {
        let g = tiny();
        assert_eq!(g.sample(30.6, -90.0), None);
        assert_eq!(g.sample(28.4, -90.0), None);
        assert_eq!(g.sample(30.0, -90.6), None);
        assert_eq!(g.sample(30.0, -87.4), None);
    }

    #[test]
    fn bounds_and_cell_centres() {
        let g = tiny();
        assert_eq!(g.bounds(), (28.5, -90.5, 30.5, -87.5));
        assert_eq!(g.cell_center(4), (29.0, -89.0));
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test --lib grid && cargo test --test grid`
Expected: compile errors such as `cannot find type Grid` and `cannot find function encode_dbz`.

- [ ] **Step 4: Implement `src/grid.rs` above the test module**

```rust
//! Decoding MRMS GRIB2 files into a compact lat/lon grid of reflectivity.

use std::io::Read;

use anyhow::{anyhow, bail, Context};
use grib::LatLons;

/// Packed value for "no echo": no coverage (-999), missing (-99) or below 0 dBZ.
pub const NO_DATA: u8 = 0;

/// Packs dBZ into one byte: 0.5 dBZ steps from 0 to 127 dBZ.
pub fn encode_dbz(dbz: f32) -> u8 {
    if !dbz.is_finite() || dbz < 0.0 {
        return NO_DATA;
    }
    ((dbz * 2.0).round() + 1.0).min(255.0) as u8
}

pub fn decode_dbz(value: u8) -> Option<f32> {
    if value == NO_DATA {
        None
    } else {
        Some(f32::from(value - 1) / 2.0)
    }
}

/// A regular lat/lon grid. Row 0 is the northernmost row; column 0 the westernmost.
#[derive(Debug, Clone)]
pub struct Grid {
    pub nx: usize,
    pub ny: usize,
    /// Latitude of the centre of row 0, degrees.
    pub lat0: f64,
    /// Longitude of the centre of column 0, degrees (-180..180).
    pub lon0: f64,
    /// Row spacing in degrees (positive; rows go south).
    pub dlat: f64,
    /// Column spacing in degrees.
    pub dlon: f64,
    /// Row-major `encode_dbz` values, `nx * ny` long.
    pub values: Vec<u8>,
}

impl Grid {
    pub fn from_grib2_gz(gz: &[u8]) -> anyhow::Result<Grid> {
        let mut raw = Vec::new();
        flate2::read::GzDecoder::new(gz)
            .read_to_end(&mut raw)
            .context("decompressing gzip")?;
        Grid::from_grib2(&raw)
    }

    pub fn from_grib2(raw: &[u8]) -> anyhow::Result<Grid> {
        let grib2 = grib::from_bytes(raw).map_err(|e| anyhow!("parsing GRIB2: {e}"))?;
        let (_, sub) = grib2.iter().next().context("GRIB2 file has no submessages")?;
        let (nx, ny) = sub.grid_shape().map_err(|e| anyhow!("reading grid shape: {e}"))?;
        if nx < 2 || ny < 2 {
            bail!("grid too small: {nx}x{ny}");
        }
        let mut points = sub.latlons().map_err(|e| anyhow!("computing lat/lons: {e}"))?;
        let (lat0, lon0) = points.next().context("grid has no points")?;
        let (_, lon1) = points.next().context("grid has one point")?;
        // Index `nx` is the first cell of row 1; two points are already consumed.
        let (lat_row1, _) = points.nth(nx - 2).context("grid has one row")?;
        let decoder = grib::Grib2SubmessageDecoder::from(sub)
            .map_err(|e| anyhow!("creating decoder: {e}"))?;
        let values: Vec<u8> = decoder
            .dispatch()
            .map_err(|e| anyhow!("decoding values: {e}"))?
            .map(encode_dbz)
            .collect();
        if values.len() != nx * ny {
            bail!("expected {} values, got {}", nx * ny, values.len());
        }
        Ok(Grid { nx, ny, lat0, lon0, dlat: lat0 - lat_row1, dlon: lon1 - lon0, values })
    }

    pub fn sample(&self, lat: f64, lon: f64) -> Option<f32> {
        let row = ((self.lat0 - lat) / self.dlat).round();
        let col = ((lon - self.lon0) / self.dlon).round();
        if row < 0.0 || col < 0.0 || row >= self.ny as f64 || col >= self.nx as f64 {
            return None;
        }
        decode_dbz(self.values[row as usize * self.nx + col as usize])
    }

    pub fn bounds(&self) -> (f64, f64, f64, f64) {
        let north = self.lat0 + self.dlat / 2.0;
        let south = self.lat0 - (self.ny as f64 - 0.5) * self.dlat;
        let west = self.lon0 - self.dlon / 2.0;
        let east = self.lon0 + (self.nx as f64 - 0.5) * self.dlon;
        (south, west, north, east)
    }

    pub fn cell_center(&self, index: usize) -> (f64, f64) {
        let (row, col) = (index / self.nx, index % self.nx);
        (self.lat0 - row as f64 * self.dlat, self.lon0 + col as f64 * self.dlon)
    }
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib grid && cargo test --test grid`
Expected: 4 unit tests and 3 integration tests pass. If `rainy_cell` panics, the fixture was captured during dry weather. Re-run Step 1 later.

- [ ] **Step 6: Commit**

```bash
git add src/lib.rs src/grid.rs tests/
git commit -m "feat: decode MRMS GRIB2 into packed grid" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Web Mercator tiles

**Files:**
- Create: `src/tiles.rs`, `tests/tiles.rs`
- Modify: `src/lib.rs` (add `pub mod tiles;`), `tests/common/mod.rs` (add `rainy_tile`)

**Interfaces:**
- Consumes: `Grid::sample`, `Grid::bounds`, `Grid::cell_center`, `palette::dbz_to_rgba`
- Produces:
  - `tiles::TILE_SIZE: u32 = 256`, `tiles::MAX_ZOOM: u8 = 12`
  - `tiles::valid_tile(z: u8, x: u32, y: u32) -> bool`
  - `tiles::tile_point_to_latlon(z, x, y, fx: f64, fy: f64) -> (f64, f64)`
  - `tiles::latlon_to_tile(z: u8, lat: f64, lon: f64) -> (u32, u32)`
  - `tiles::tile_bounds(z, x, y) -> (south, west, north, east)`
  - `tiles::render_tile(&Grid, z, x, y) -> Vec<u8>` (PNG)
  - `tiles::empty_tile_png() -> &'static [u8]`
  - `common::rainy_tile(&Grid, z) -> (u32, u32)`

- [ ] **Step 1: Write the failing tests**

Add `pub mod tiles;` to `src/lib.rs`. Append to `tests/common/mod.rs`:

```rust
/// A tile at zoom `z` that contains at least one cell with 30+ dBZ.
pub fn rainy_tile(grid: &Grid, z: u8) -> (u32, u32) {
    let (lat, lon) = grid.cell_center(rainy_cell(grid));
    rust_radar::tiles::latlon_to_tile(z, lat, lon)
}
```

`tests/tiles.rs`:

```rust
mod common;

use rust_radar::grid::Grid;
use rust_radar::tiles;

#[test]
fn fixture_rain_renders_coloured_pixels() {
    let g = Grid::from_grib2_gz(&common::mrms_fixture()).unwrap();
    let (x, y) = common::rainy_tile(&g, 7);
    let rgba = common::decode_png(&tiles::render_tile(&g, 7, x, y));
    assert_eq!(rgba.len(), 256 * 256 * 4);
    assert!(common::opaque_pixels(&rgba) > 0);
}

#[test]
fn fixture_tile_over_europe_is_transparent() {
    let g = Grid::from_grib2_gz(&common::mrms_fixture()).unwrap();
    assert_eq!(tiles::render_tile(&g, 3, 4, 2), tiles::empty_tile_png());
}
```

Unit tests at the bottom of `src/tiles.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::{encode_dbz, Grid};
    use crate::palette::dbz_to_rgba;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-4
    }

    fn decode(png_bytes: &[u8]) -> Vec<u8> {
        let mut reader = png::Decoder::new(png_bytes).read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size()];
        reader.next_frame(&mut buf).unwrap();
        buf
    }

    /// 2°x2° of uniform echo centred near (30, -90).
    fn uniform_grid(dbz: f32) -> Grid {
        Grid {
            nx: 200,
            ny: 200,
            lat0: 31.0,
            lon0: -91.0,
            dlat: 0.01,
            dlon: 0.01,
            values: vec![encode_dbz(dbz); 200 * 200],
        }
    }

    #[test]
    fn world_tile_centre_is_origin() {
        let (lat, lon) = tile_point_to_latlon(0, 0, 0, 0.5, 0.5);
        assert!(close(lat, 0.0) && close(lon, 0.0), "{lat}, {lon}");
    }

    #[test]
    fn top_left_is_mercator_limit() {
        let (lat, lon) = tile_point_to_latlon(1, 0, 0, 0.0, 0.0);
        assert!(close(lat, 85.05113) && close(lon, -180.0), "{lat}, {lon}");
    }

    #[test]
    fn latlon_to_tile_lands_inside_its_tile() {
        for z in [0u8, 4, 7, 12] {
            let (x, y) = latlon_to_tile(z, 29.95, -90.07);
            let (s, w, n, e) = tile_bounds(z, x, y);
            assert!(s <= 29.95 && 29.95 <= n && w <= -90.07 && -90.07 <= e, "z{z}");
        }
    }

    #[test]
    fn valid_tile_ranges() {
        assert!(valid_tile(0, 0, 0));
        assert!(valid_tile(3, 7, 7));
        assert!(!valid_tile(3, 8, 0));
        assert!(!valid_tile(3, 0, 8));
        assert!(!valid_tile(MAX_ZOOM + 1, 0, 0));
    }

    #[test]
    fn renders_palette_colour_where_grid_has_echo() {
        let g = uniform_grid(40.0);
        let (x, y) = latlon_to_tile(8, 30.0, -90.0);
        let rgba = decode(&render_tile(&g, 8, x, y));
        assert!(rgba.chunks(4).any(|p| p == &dbz_to_rgba(40.0)[..]));
    }

    #[test]
    fn tile_outside_grid_is_the_empty_tile() {
        assert_eq!(render_tile(&uniform_grid(40.0), 3, 4, 2), empty_tile_png());
        assert!(decode(empty_tile_png()).iter().all(|&b| b == 0));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib tiles && cargo test --test tiles`
Expected: compile errors such as `cannot find function tile_point_to_latlon`.

- [ ] **Step 3: Implement `src/tiles.rs` above the test module**

```rust
//! Web Mercator tile maths and radar tile rendering.

use std::f64::consts::PI;
use std::sync::OnceLock;

use crate::grid::Grid;
use crate::palette::dbz_to_rgba;

pub const TILE_SIZE: u32 = 256;
pub const MAX_ZOOM: u8 = 12;

pub fn valid_tile(z: u8, x: u32, y: u32) -> bool {
    z <= MAX_ZOOM && u64::from(x) < (1u64 << z) && u64::from(y) < (1u64 << z)
}

/// Lat/lon of a point in tile (z, x, y); `fx`/`fy` run 0..=1 from the tile's top-left corner.
pub fn tile_point_to_latlon(z: u8, x: u32, y: u32, fx: f64, fy: f64) -> (f64, f64) {
    let n = f64::from(1u32 << z);
    let lon = (f64::from(x) + fx) / n * 360.0 - 180.0;
    let lat = (PI * (1.0 - 2.0 * (f64::from(y) + fy) / n)).sinh().atan().to_degrees();
    (lat, lon)
}

pub fn latlon_to_tile(z: u8, lat: f64, lon: f64) -> (u32, u32) {
    let n = f64::from(1u32 << z);
    let x = ((lon + 180.0) / 360.0 * n).floor();
    let lat_r = lat.to_radians();
    let y = ((1.0 - (lat_r.tan() + 1.0 / lat_r.cos()).ln() / PI) / 2.0 * n).floor();
    let max = n - 1.0;
    (x.clamp(0.0, max) as u32, y.clamp(0.0, max) as u32)
}

/// (south, west, north, east) of a tile, degrees.
pub fn tile_bounds(z: u8, x: u32, y: u32) -> (f64, f64, f64, f64) {
    let (north, west) = tile_point_to_latlon(z, x, y, 0.0, 0.0);
    let (south, east) = tile_point_to_latlon(z, x, y, 1.0, 1.0);
    (south, west, north, east)
}

pub fn render_tile(grid: &Grid, z: u8, x: u32, y: u32) -> Vec<u8> {
    let (s, w, n, e) = tile_bounds(z, x, y);
    let (gs, gw, gn, ge) = grid.bounds();
    if s >= gn || n <= gs || w >= ge || e <= gw {
        return empty_tile_png().to_vec();
    }
    let size = TILE_SIZE as usize;
    // Latitude depends only on the pixel row and longitude only on the column.
    let centre = |p: usize| (p as f64 + 0.5) / size as f64;
    let lons: Vec<f64> = (0..size).map(|px| tile_point_to_latlon(z, x, y, centre(px), 0.0).1).collect();
    let lats: Vec<f64> = (0..size).map(|py| tile_point_to_latlon(z, x, y, 0.0, centre(py)).0).collect();
    let mut rgba = vec![0u8; size * size * 4];
    for (py, &lat) in lats.iter().enumerate() {
        for (px, &lon) in lons.iter().enumerate() {
            if let Some(dbz) = grid.sample(lat, lon) {
                let i = (py * size + px) * 4;
                rgba[i..i + 4].copy_from_slice(&dbz_to_rgba(dbz));
            }
        }
    }
    encode_png(&rgba)
}

pub fn empty_tile_png() -> &'static [u8] {
    static EMPTY: OnceLock<Vec<u8>> = OnceLock::new();
    EMPTY.get_or_init(|| encode_png(&vec![0u8; (TILE_SIZE * TILE_SIZE * 4) as usize]))
}

fn encode_png(rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, TILE_SIZE, TILE_SIZE);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("PNG header to memory cannot fail");
        writer.write_image_data(rgba).expect("PNG data to memory cannot fail");
    }
    out
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib tiles && cargo test --test tiles`
Expected: 6 unit tests and 2 integration tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/lib.rs src/tiles.rs tests/
git commit -m "feat: render Web Mercator radar tiles" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: MRMS frame archive

**Files:**
- Create: `src/mrms_archive.rs`, `tests/mrms_archive.rs`
- Modify: `src/lib.rs` (add `pub mod mrms_archive;`)

**Interfaces:**
- Consumes: `Grid::from_grib2_gz`
- Produces:
  - `MrmsArchive::open(dir: impl Into<PathBuf>, cache_frames: usize) -> anyhow::Result<MrmsArchive>` (creates the dir and deletes leftover `*.tmp` files)
  - `frames(&self) -> anyhow::Result<Vec<DateTime<Utc>>>` (ascending)
  - `latest(&self) -> anyhow::Result<Option<DateTime<Utc>>>`
  - `contains(&self, t) -> bool`
  - `store(&self, t, gz: &[u8]) -> anyhow::Result<Arc<Grid>>`. Validates the bytes first, so nothing is written on failure.
  - `load(&self, t) -> anyhow::Result<Arc<Grid>>`
  - `prune(&self, cutoff: DateTime<Utc>) -> anyhow::Result<usize>`
  - `mrms_archive::frame_file_name(t) -> String` (`YYYYMMDD-HHMMSS.grib2.gz`)
  - `MrmsArchive` is `Send + Sync`. It is shared as `Arc<MrmsArchive>`.

- [ ] **Step 1: Write the failing tests**

Add `pub mod mrms_archive;` to `src/lib.rs`. `tests/mrms_archive.rs`:

```rust
mod common;

use chrono::{DateTime, TimeZone, Utc};
use rust_radar::mrms_archive::{frame_file_name, MrmsArchive};

fn t(h: u32, m: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 8, h, m, 0).unwrap()
}

#[test]
fn store_then_list_and_load() {
    let dir = tempfile::tempdir().unwrap();
    let archive = MrmsArchive::open(dir.path(), 2).unwrap();
    assert_eq!(archive.latest().unwrap(), None);
    let grid = archive.store(t(17, 0), &common::mrms_fixture()).unwrap();
    assert_eq!(grid.nx, 7000);
    assert_eq!(archive.frames().unwrap(), vec![t(17, 0)]);
    assert_eq!(archive.latest().unwrap(), Some(t(17, 0)));
    assert!(archive.contains(t(17, 0)));
    assert!(!archive.contains(t(17, 2)));

    // A fresh instance has an empty cache, so this load reads from disk.
    let reopened = MrmsArchive::open(dir.path(), 2).unwrap();
    assert_eq!(reopened.load(t(17, 0)).unwrap().ny, 3500);
}

#[test]
fn corrupt_file_is_rejected_and_not_written() {
    let dir = tempfile::tempdir().unwrap();
    let archive = MrmsArchive::open(dir.path(), 2).unwrap();
    assert!(archive.store(t(17, 0), b"garbage").is_err());
    assert!(archive.frames().unwrap().is_empty());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn stray_and_leftover_files_are_ignored_and_cleaned() {
    let dir = tempfile::tempdir().unwrap();
    let leftover = dir.path().join(format!("{}.tmp", frame_file_name(t(16, 0))));
    std::fs::write(&leftover, b"partial").unwrap();
    std::fs::write(dir.path().join("notes.txt"), b"hi").unwrap();
    let archive = MrmsArchive::open(dir.path(), 2).unwrap();
    assert!(!leftover.exists(), "leftover temp file should be removed on open");
    assert!(archive.frames().unwrap().is_empty());
    assert!(archive.load(t(16, 0)).is_err());
}

#[test]
fn prune_removes_only_older_frames() {
    let dir = tempfile::tempdir().unwrap();
    let archive = MrmsArchive::open(dir.path(), 2).unwrap();
    let bytes = common::mrms_fixture();
    for m in [0, 2, 4] {
        archive.store(t(17, m), &bytes).unwrap();
    }
    assert_eq!(archive.prune(t(17, 3)).unwrap(), 2);
    assert_eq!(archive.frames().unwrap(), vec![t(17, 4)]);
    assert!(archive.load(t(17, 0)).is_err(), "pruned frame must not be served from cache");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --test mrms_archive`
Expected: compile error `could not find mrms_archive` or `cannot find ... MrmsArchive`.

- [ ] **Step 3: Implement `src/mrms_archive.rs`**

```rust
//! Rolling on-disk store of MRMS frames with an LRU of decoded grids.

use std::fs;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Context;
use chrono::{DateTime, NaiveDateTime, Utc};
use lru::LruCache;

use crate::grid::Grid;

const SUFFIX: &str = ".grib2.gz";
const TIME_FORMAT: &str = "%Y%m%d-%H%M%S";

pub fn frame_file_name(t: DateTime<Utc>) -> String {
    format!("{}{SUFFIX}", t.format(TIME_FORMAT))
}

fn parse_frame_file_name(name: &str) -> Option<DateTime<Utc>> {
    let stem = name.strip_suffix(SUFFIX)?;
    NaiveDateTime::parse_from_str(stem, TIME_FORMAT).ok().map(|n| n.and_utc())
}

pub struct MrmsArchive {
    dir: PathBuf,
    cache: Mutex<LruCache<DateTime<Utc>, Arc<Grid>>>,
}

impl MrmsArchive {
    pub fn open(dir: impl Into<PathBuf>, cache_frames: usize) -> anyhow::Result<MrmsArchive> {
        let dir = dir.into();
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        for entry in fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.extension().is_some_and(|e| e == "tmp") {
                fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
            }
        }
        let capacity = NonZeroUsize::new(cache_frames.max(1)).expect("max(1) is non-zero");
        Ok(MrmsArchive { dir, cache: Mutex::new(LruCache::new(capacity)) })
    }

    pub fn frames(&self) -> anyhow::Result<Vec<DateTime<Utc>>> {
        let mut frames = Vec::new();
        for entry in fs::read_dir(&self.dir).with_context(|| format!("listing {}", self.dir.display()))? {
            if let Some(t) = entry?.file_name().to_str().and_then(parse_frame_file_name) {
                frames.push(t);
            }
        }
        frames.sort();
        Ok(frames)
    }

    pub fn latest(&self) -> anyhow::Result<Option<DateTime<Utc>>> {
        Ok(self.frames()?.pop())
    }

    pub fn contains(&self, t: DateTime<Utc>) -> bool {
        self.dir.join(frame_file_name(t)).exists()
    }

    /// Validates by decoding, then writes atomically. Nothing is written if decoding fails.
    pub fn store(&self, t: DateTime<Utc>, gz: &[u8]) -> anyhow::Result<Arc<Grid>> {
        let grid = Arc::new(Grid::from_grib2_gz(gz).with_context(|| format!("validating frame {t}"))?);
        let path = self.dir.join(frame_file_name(t));
        let tmp = self.dir.join(format!("{}.tmp", frame_file_name(t)));
        fs::write(&tmp, gz).with_context(|| format!("writing {}", tmp.display()))?;
        fs::rename(&tmp, &path).with_context(|| format!("renaming to {}", path.display()))?;
        self.cache.lock().unwrap().put(t, Arc::clone(&grid));
        Ok(grid)
    }

    pub fn load(&self, t: DateTime<Utc>) -> anyhow::Result<Arc<Grid>> {
        if let Some(grid) = self.cache.lock().unwrap().get(&t) {
            return Ok(Arc::clone(grid));
        }
        let path = self.dir.join(frame_file_name(t));
        let bytes = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        // Decode outside the lock; two concurrent misses just decode twice.
        let grid = Arc::new(Grid::from_grib2_gz(&bytes).with_context(|| format!("decoding {}", path.display()))?);
        self.cache.lock().unwrap().put(t, Arc::clone(&grid));
        Ok(grid)
    }

    pub fn prune(&self, cutoff: DateTime<Utc>) -> anyhow::Result<usize> {
        let mut removed = 0;
        for t in self.frames()?.into_iter().filter(|&t| t < cutoff) {
            let path = self.dir.join(frame_file_name(t));
            fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
            self.cache.lock().unwrap().pop(&t);
            removed += 1;
        }
        Ok(removed)
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --test mrms_archive`
Expected: 4 passed.

- [ ] **Step 5: Commit**

```bash
git add src/lib.rs src/mrms_archive.rs tests/mrms_archive.rs
git commit -m "feat: on-disk MRMS frame archive with decoded cache" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: Fetcher abstraction and source status

**Files:**
- Create: `src/fetch.rs`, `src/status.rs`
- Modify: `src/lib.rs` (add `pub mod fetch;` and `pub mod status;`)

**Interfaces:**
- Produces:
  - `#[async_trait] pub trait Fetcher: Send + Sync { async fn get(&self, url: &str) -> anyhow::Result<Vec<u8>>; }`. Non-2xx responses are errors.
  - `HttpFetcher::new() -> anyhow::Result<HttpFetcher>`
  - `FakeFetcher::new()`, `.ok(&self, url: &str, body: impl Into<Vec<u8>>)`, `.fail(&self, url: &str, msg: &str)`, `.calls_to(&self, url: &str) -> usize`. URLs without a canned response return an error.
  - `status::Status` (`Clone`, shared):
    - `.success(&self, source: &str, at: DateTime<Utc>)`
    - `.failure(&self, source: &str, error: &anyhow::Error) -> u32`, which returns the number of consecutive failures
    - `.snapshot(&self) -> BTreeMap<String, SourceStatus>`
  - `SourceStatus { last_success: Option<DateTime<Utc>>, last_error: Option<String>, consecutive_failures: u32 }` (Serialize)
  - `status::backoff_delay(base: Duration, failures: u32) -> Duration` and `status::MAX_BACKOFF` (15 min)

- [ ] **Step 1: Write the failing tests**

Add both modules to `src/lib.rs`. At the bottom of `src/status.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn backoff_doubles_and_caps() {
        let base = Duration::from_secs(60);
        assert_eq!(backoff_delay(base, 0), base);
        assert_eq!(backoff_delay(base, 1), Duration::from_secs(120));
        assert_eq!(backoff_delay(base, 3), Duration::from_secs(480));
        assert_eq!(backoff_delay(base, 20), MAX_BACKOFF);
        let long = Duration::from_secs(1200);
        assert_eq!(backoff_delay(long, 1), long, "never shorter than the base interval");
    }

    #[test]
    fn success_resets_failures() {
        let status = Status::default();
        assert_eq!(status.failure("mrms", &anyhow::anyhow!("boom")), 1);
        assert_eq!(status.failure("mrms", &anyhow::anyhow!("boom")), 2);
        let at = Utc.with_ymd_and_hms(2026, 10, 8, 17, 0, 0).unwrap();
        status.success("mrms", at);
        let snap = status.snapshot();
        assert_eq!(
            snap["mrms"],
            SourceStatus { last_success: Some(at), last_error: None, consecutive_failures: 0 }
        );
    }
}
```

At the bottom of `src/fetch.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fake_fetcher_serves_canned_responses_and_counts_calls() {
        let fake = FakeFetcher::new();
        fake.ok("https://a.test/x", "hello");
        fake.fail("https://a.test/y", "503");
        assert_eq!(fake.get("https://a.test/x").await.unwrap(), b"hello");
        assert!(fake.get("https://a.test/y").await.is_err());
        assert!(fake.get("https://a.test/unknown").await.is_err());
        assert_eq!(fake.calls_to("https://a.test/x"), 1);
        assert_eq!(fake.calls_to("https://a.test/y"), 1);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib -- status:: fetch::`
Expected: compile errors such as `cannot find function backoff_delay` and `cannot find type FakeFetcher`.

- [ ] **Step 3: Implement `src/status.rs` above its tests**

```rust
//! Per-source health, shared between pollers and the API.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Serialize;

pub const MAX_BACKOFF: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct SourceStatus {
    pub last_success: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub consecutive_failures: u32,
}

#[derive(Debug, Clone, Default)]
pub struct Status(Arc<RwLock<BTreeMap<String, SourceStatus>>>);

impl Status {
    pub fn success(&self, source: &str, at: DateTime<Utc>) {
        let mut map = self.0.write().unwrap();
        let entry = map.entry(source.to_string()).or_default();
        entry.last_success = Some(at);
        entry.last_error = None;
        entry.consecutive_failures = 0;
    }

    pub fn failure(&self, source: &str, error: &anyhow::Error) -> u32 {
        let mut map = self.0.write().unwrap();
        let entry = map.entry(source.to_string()).or_default();
        entry.last_error = Some(format!("{error:#}"));
        entry.consecutive_failures += 1;
        entry.consecutive_failures
    }

    pub fn snapshot(&self) -> BTreeMap<String, SourceStatus> {
        self.0.read().unwrap().clone()
    }
}

/// Delay before the next poll: `base` normally, doubling per consecutive failure, capped.
pub fn backoff_delay(base: Duration, failures: u32) -> Duration {
    if failures == 0 {
        return base;
    }
    base.saturating_mul(1u32 << failures.min(10)).min(MAX_BACKOFF).max(base)
}
```

- [ ] **Step 4: Implement `src/fetch.rs` above its tests**

```rust
//! HTTP access behind a trait so pollers can be tested without the network.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{anyhow, Context};
use async_trait::async_trait;

#[async_trait]
pub trait Fetcher: Send + Sync {
    /// GET `url` and return the body. Non-2xx responses are errors.
    async fn get(&self, url: &str) -> anyhow::Result<Vec<u8>>;
}

pub struct HttpFetcher {
    client: reqwest::Client,
}

impl HttpFetcher {
    pub fn new() -> anyhow::Result<HttpFetcher> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .user_agent("rust-radar/0.1 (personal hurricane tracker)")
            .build()
            .context("building HTTP client")?;
        Ok(HttpFetcher { client })
    }
}

#[async_trait]
impl Fetcher for HttpFetcher {
    async fn get(&self, url: &str) -> anyhow::Result<Vec<u8>> {
        let response = self.client.get(url).send().await?.error_for_status()?;
        Ok(response.bytes().await?.to_vec())
    }
}

/// Test double: canned responses keyed by exact URL.
#[derive(Default)]
pub struct FakeFetcher {
    responses: Mutex<HashMap<String, Result<Vec<u8>, String>>>,
    calls: Mutex<Vec<String>>,
}

impl FakeFetcher {
    pub fn new() -> FakeFetcher {
        FakeFetcher::default()
    }

    pub fn ok(&self, url: &str, body: impl Into<Vec<u8>>) {
        self.responses.lock().unwrap().insert(url.to_string(), Ok(body.into()));
    }

    pub fn fail(&self, url: &str, message: &str) {
        self.responses.lock().unwrap().insert(url.to_string(), Err(message.to_string()));
    }

    pub fn calls_to(&self, url: &str) -> usize {
        self.calls.lock().unwrap().iter().filter(|c| *c == url).count()
    }
}

#[async_trait]
impl Fetcher for FakeFetcher {
    async fn get(&self, url: &str) -> anyhow::Result<Vec<u8>> {
        self.calls.lock().unwrap().push(url.to_string());
        match self.responses.lock().unwrap().get(url) {
            Some(Ok(body)) => Ok(body.clone()),
            Some(Err(message)) => Err(anyhow!("{url}: {message}")),
            None => Err(anyhow!("FakeFetcher has no response for {url}")),
        }
    }
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib -- status:: fetch::`
Expected: 3 passed.

- [ ] **Step 6: Commit**

```bash
git add src/lib.rs src/fetch.rs src/status.rs
git commit -m "feat: fetcher trait, fake fetcher, source status and backoff" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: MRMS source (live poll and S3 backfill)

**Files:**
- Create: `src/sources/mod.rs`, `src/sources/mrms.rs`, `tests/mrms_source.rs`
- Modify: `src/lib.rs` (add `pub mod sources;`)

**Interfaces:**
- Consumes: `Fetcher`, `MrmsArchive::{contains, store}`, `Status`, `backoff_delay`
- Produces (in `rust_radar::sources::mrms`):
  - Constants: `SOURCE = "mrms"`, `PRODUCT`, `LIVE_DIR_URL`, `S3_BASE_URL`
  - URL helpers: `remote_file_name(t) -> String`, `live_url(t) -> String`, `s3_list_url(day: NaiveDate) -> String`, `s3_url(t) -> String`
  - `parse_listing(&str) -> Vec<DateTime<Utc>>`
  - `days_in_window(start, end) -> Vec<NaiveDate>`
  - `poll_once(&dyn Fetcher, &Arc<MrmsArchive>) -> anyhow::Result<Option<DateTime<Utc>>>`
  - `backfill(&dyn Fetcher, &Arc<MrmsArchive>, now: DateTime<Utc>, hours: u64) -> anyhow::Result<usize>`
  - `run(Arc<dyn Fetcher>, Arc<MrmsArchive>, Status, interval: Duration)`, which loops forever

- [ ] **Step 1: Write the failing tests**

Add `pub mod sources;` to `src/lib.rs`. Create `src/sources/mod.rs` with `pub mod mrms;`. `tests/mrms_source.rs`:

```rust
mod common;

use std::sync::Arc;

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use rust_radar::fetch::FakeFetcher;
use rust_radar::mrms_archive::MrmsArchive;
use rust_radar::sources::mrms::*;

fn t(h: u32, m: u32, s: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 8, h, m, s).unwrap()
}

const HTML: &str = r#"
<a href="MRMS_MergedReflectivityQCComposite_00.50_20261008-171438.grib2.gz">MRMS_MergedReflectivityQCComposite_00.50_20261008-171438.grib2.gz</a>
<a href="MRMS_MergedReflectivityQCComposite_00.50_20261008-171834.grib2.gz">x</a>
<a href="MRMS_MergedReflectivityQCComposite_00.50_20261008-171635.grib2.gz">x</a>
<a href="MRMS_MergedReflectivityQCComposite.latest.grib2.gz">latest</a>
<a href="MRMS_MergedReflectivityQCComposite_00.50_garbage.grib2.gz">bad</a>"#;

fn s3_listing(times: &[DateTime<Utc>]) -> String {
    let keys: String = times
        .iter()
        .map(|&t| format!("<Contents><Key>CONUS/{PRODUCT}/{}/{}</Key></Contents>", t.format("%Y%m%d"), remote_file_name(t)))
        .collect();
    format!("<ListBucketResult>{keys}</ListBucketResult>")
}

fn archive() -> (tempfile::TempDir, Arc<MrmsArchive>) {
    let dir = tempfile::tempdir().unwrap();
    let archive = Arc::new(MrmsArchive::open(dir.path(), 2).unwrap());
    (dir, archive)
}

#[test]
fn parses_html_listing_sorted_and_deduplicated() {
    assert_eq!(parse_listing(HTML), vec![t(17, 14, 38), t(17, 16, 35), t(17, 18, 34)]);
}

#[test]
fn parses_s3_listing() {
    assert_eq!(parse_listing(&s3_listing(&[t(0, 0, 39)])), vec![t(0, 0, 39)]);
}

#[test]
fn builds_urls() {
    assert_eq!(
        s3_url(t(0, 0, 39)),
        "https://noaa-mrms-pds.s3.amazonaws.com/CONUS/MergedReflectivityQCComposite_00.50/20261008/MRMS_MergedReflectivityQCComposite_00.50_20261008-000039.grib2.gz"
    );
    assert_eq!(
        live_url(t(17, 18, 34)),
        "https://mrms.ncep.noaa.gov/2D/MergedReflectivityQCComposite/MRMS_MergedReflectivityQCComposite_00.50_20261008-171834.grib2.gz"
    );
    assert_eq!(
        s3_list_url(NaiveDate::from_ymd_opt(2026, 10, 8).unwrap()),
        "https://noaa-mrms-pds.s3.amazonaws.com/?list-type=2&prefix=CONUS/MergedReflectivityQCComposite_00.50/20261008/"
    );
}

#[test]
fn window_across_midnight_lists_both_days() {
    let start = Utc.with_ymd_and_hms(2026, 10, 7, 22, 0, 0).unwrap();
    let end = Utc.with_ymd_and_hms(2026, 10, 8, 2, 0, 0).unwrap();
    assert_eq!(
        days_in_window(start, end),
        vec![NaiveDate::from_ymd_opt(2026, 10, 7).unwrap(), NaiveDate::from_ymd_opt(2026, 10, 8).unwrap()]
    );
}

#[tokio::test]
async fn poll_stores_newest_frame_once() {
    let (_dir, archive) = archive();
    let fake = FakeFetcher::new();
    fake.ok(LIVE_DIR_URL, HTML);
    fake.ok(&live_url(t(17, 18, 34)), common::mrms_fixture());
    assert_eq!(poll_once(&fake, &archive).await.unwrap(), Some(t(17, 18, 34)));
    assert_eq!(archive.frames().unwrap(), vec![t(17, 18, 34)]);
    assert_eq!(poll_once(&fake, &archive).await.unwrap(), None);
    assert_eq!(fake.calls_to(&live_url(t(17, 18, 34))), 1);
}

#[tokio::test]
async fn poll_rejects_corrupt_frame() {
    let (_dir, archive) = archive();
    let fake = FakeFetcher::new();
    fake.ok(LIVE_DIR_URL, HTML);
    fake.ok(&live_url(t(17, 18, 34)), b"garbage".to_vec());
    assert!(poll_once(&fake, &archive).await.is_err());
    assert!(archive.frames().unwrap().is_empty());
}

#[tokio::test]
async fn poll_reports_listing_failure_and_empty_listing() {
    let (_dir, archive) = archive();
    let fake = FakeFetcher::new();
    fake.fail(LIVE_DIR_URL, "503");
    assert!(poll_once(&fake, &archive).await.is_err());
    fake.ok(LIVE_DIR_URL, "<html>nothing here</html>");
    assert!(poll_once(&fake, &archive).await.is_err());
}

#[tokio::test]
async fn backfill_fetches_only_missing_frames_in_window() {
    let (_dir, archive) = archive();
    let bytes = common::mrms_fixture();
    archive.store(t(17, 10, 0), &bytes).unwrap();
    let fake = FakeFetcher::new();
    let day = NaiveDate::from_ymd_opt(2026, 10, 8).unwrap();
    fake.ok(
        &s3_list_url(day),
        s3_listing(&[t(16, 30, 0), t(17, 10, 0), t(17, 20, 0), t(17, 30, 0)]),
    );
    fake.ok(&s3_url(t(17, 20, 0)), bytes.clone());
    fake.fail(&s3_url(t(17, 30, 0)), "404");

    let stored = backfill(&fake, &archive, t(18, 0, 0), 1).await.unwrap();

    assert_eq!(stored, 1, "one failed download must not abort the rest");
    assert_eq!(archive.frames().unwrap(), vec![t(17, 10, 0), t(17, 20, 0)]);
    assert_eq!(fake.calls_to(&s3_url(t(16, 30, 0))), 0, "outside the window");
    assert_eq!(fake.calls_to(&s3_url(t(17, 10, 0))), 0, "already archived");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --test mrms_source`
Expected: compile errors such as `cannot find function parse_listing`.

- [ ] **Step 3: Implement `src/sources/mrms.rs`**

```rust
//! MRMS composite reflectivity: live polling and S3 backfill.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context};
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use tracing::{info, warn};

use crate::fetch::Fetcher;
use crate::mrms_archive::MrmsArchive;
use crate::status::{backoff_delay, Status};

pub const SOURCE: &str = "mrms";
pub const PRODUCT: &str = "MergedReflectivityQCComposite_00.50";
pub const LIVE_DIR_URL: &str = "https://mrms.ncep.noaa.gov/2D/MergedReflectivityQCComposite/";
pub const S3_BASE_URL: &str = "https://noaa-mrms-pds.s3.amazonaws.com";
const FILE_PREFIX: &str = "MRMS_MergedReflectivityQCComposite_00.50_";
const FILE_SUFFIX: &str = ".grib2.gz";
const TIME_FORMAT: &str = "%Y%m%d-%H%M%S";

pub fn remote_file_name(t: DateTime<Utc>) -> String {
    format!("{FILE_PREFIX}{}{FILE_SUFFIX}", t.format(TIME_FORMAT))
}

pub fn live_url(t: DateTime<Utc>) -> String {
    format!("{LIVE_DIR_URL}{}", remote_file_name(t))
}

/// One S3 page holds 1000 keys; MRMS publishes ~720 files a day, so no pagination is needed.
pub fn s3_list_url(day: NaiveDate) -> String {
    format!("{S3_BASE_URL}/?list-type=2&prefix=CONUS/{PRODUCT}/{}/", day.format("%Y%m%d"))
}

pub fn s3_url(t: DateTime<Utc>) -> String {
    format!("{S3_BASE_URL}/CONUS/{PRODUCT}/{}/{}", t.format("%Y%m%d"), remote_file_name(t))
}

/// Every frame time named in an HTML directory listing or S3 XML listing, sorted and de-duplicated.
pub fn parse_listing(text: &str) -> Vec<DateTime<Utc>> {
    let mut times = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find(FILE_PREFIX) {
        rest = &rest[i + FILE_PREFIX.len()..];
        let has_suffix = rest.get(15..).is_some_and(|s| s.starts_with(FILE_SUFFIX));
        if let (Some(stamp), true) = (rest.get(..15), has_suffix) {
            if let Ok(naive) = NaiveDateTime::parse_from_str(stamp, TIME_FORMAT) {
                times.push(naive.and_utc());
            }
        }
    }
    times.sort();
    times.dedup();
    times
}

/// UTC calendar days touched by [start, end].
pub fn days_in_window(start: DateTime<Utc>, end: DateTime<Utc>) -> Vec<NaiveDate> {
    let mut days = Vec::new();
    let mut day = start.date_naive();
    while day <= end.date_naive() {
        days.push(day);
        day = day.succ_opt().expect("date overflow");
    }
    days
}

/// Downloads the newest live frame if it is not archived yet. Returns its time if stored.
pub async fn poll_once(fetcher: &dyn Fetcher, archive: &Arc<MrmsArchive>) -> anyhow::Result<Option<DateTime<Utc>>> {
    let listing = fetcher.get(LIVE_DIR_URL).await.context("fetching MRMS listing")?;
    let Some(&newest) = parse_listing(&String::from_utf8_lossy(&listing)).last() else {
        bail!("MRMS listing contains no {PRODUCT} files");
    };
    if archive.contains(newest) {
        return Ok(None);
    }
    let bytes = fetcher
        .get(&live_url(newest))
        .await
        .with_context(|| format!("downloading MRMS frame {newest}"))?;
    store(archive, newest, bytes).await?;
    Ok(Some(newest))
}

/// Fills gaps in the last `hours` from S3. Individual frame failures are logged and skipped.
pub async fn backfill(fetcher: &dyn Fetcher, archive: &Arc<MrmsArchive>, now: DateTime<Utc>, hours: u64) -> anyhow::Result<usize> {
    let start = now - chrono::Duration::hours(hours as i64);
    let mut stored = 0;
    for day in days_in_window(start, now) {
        let listing = fetcher
            .get(&s3_list_url(day))
            .await
            .with_context(|| format!("listing MRMS on S3 for {day}"))?;
        for t in parse_listing(&String::from_utf8_lossy(&listing)) {
            if t < start || t > now || archive.contains(t) {
                continue;
            }
            let result = match fetcher.get(&s3_url(t)).await {
                Ok(bytes) => store(archive, t, bytes).await,
                Err(e) => Err(e),
            };
            match result {
                Ok(()) => stored += 1,
                Err(e) => warn!("backfill skipped MRMS frame {t}: {e:#}"),
            }
        }
    }
    Ok(stored)
}

async fn store(archive: &Arc<MrmsArchive>, t: DateTime<Utc>, bytes: Vec<u8>) -> anyhow::Result<()> {
    let archive = Arc::clone(archive);
    tokio::task::spawn_blocking(move || archive.store(t, &bytes).map(|_| ()))
        .await
        .context("MRMS store task panicked")?
}

pub async fn run(fetcher: Arc<dyn Fetcher>, archive: Arc<MrmsArchive>, status: Status, interval: Duration) {
    loop {
        let failures = match poll_once(fetcher.as_ref(), &archive).await {
            Ok(new_frame) => {
                if let Some(t) = new_frame {
                    info!("stored MRMS frame {t}");
                }
                status.success(SOURCE, Utc::now());
                0
            }
            Err(e) => {
                warn!("MRMS poll failed: {e:#}");
                status.failure(SOURCE, &e)
            }
        };
        tokio::time::sleep(backoff_delay(interval, failures)).await;
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --test mrms_source`
Expected: 8 passed.

- [ ] **Step 5: Commit**

```bash
git add src/lib.rs src/sources/ tests/mrms_source.rs
git commit -m "feat: MRMS live polling and S3 backfill" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 8: NHC feed parsing

**Files:**
- Create: `src/sources/nhc.rs`, `tests/nhc_parse.rs`
- Modify: `src/sources/mod.rs` (add `pub mod nhc;`)

**Interfaces:**
- Produces (in `rust_radar::sources::nhc`):
  - `StormInfo { id: String, bin_number: String, name: String, classification: String, intensity_kt: u32, pressure_mb: u32, lat: f64, lon: f64, movement_dir: u32, movement_speed_mph: u32, advisory_num: String, issuance: DateTime<Utc>, public_advisory_url: Option<String>, forecast_zip_url: Option<String> }`. Derives `Debug, Clone, PartialEq, Serialize, Deserialize`.
  - `parse_current_storms(&[u8]) -> anyhow::Result<Vec<StormInfo>>`. Malformed entries are skipped. A missing `activeStorms` key is an error.
  - `parse_layer_index(&[u8]) -> anyhow::Result<HashMap<String, u32>>` (layer name to id)
  - `tag_features(collection: Value, storm_id: &str, layer: &str) -> anyhow::Result<Vec<Value>>`. Adds `storm_id` and `layer` properties, plus `wind_kt` taken from `maxwind` or `intensity`.
  - `category(kt: u32) -> &'static str`, returning one of `"TD" | "TS" | "1".."5"`
  - `advisory_number(&str) -> Option<u32>`
  - Constants: `SOURCE = "nhc"`, `CURRENT_STORMS_URL`, `MAPSERVER_URL`, and `STORM_LAYERS: [(&str, &str); 6]`
  - URL helpers: `layer_index_url() -> String`, `layer_query_url(id: u32) -> String`

- [ ] **Step 1: Write the failing tests**

Add `pub mod nhc;` to `src/sources/mod.rs`. `tests/nhc_parse.rs`:

```rust
use chrono::{TimeZone, Utc};
use serde_json::json;
use rust_radar::sources::nhc::*;

const CURRENT_STORMS: &str = r#"{"activeStorms":[
 {"id":"al092026","binNumber":"AT4","name":"Isaias","classification":"HU","intensity":"75","pressure":"975",
  "latitudeNumeric":23.7,"longitudeNumeric":-90.2,"movementDir":60,"movementSpeed":10,
  "lastUpdate":"2026-10-08T15:00:00.000Z",
  "publicAdvisory":{"advNum":"008","issuance":"2026-10-08T15:00:00.000Z","url":"https://www.nhc.noaa.gov/text/MIATCPAT4.shtml"},
  "forecastTrack":{"advNum":"008","zipFile":"https://www.nhc.noaa.gov/gis/forecast/archive/al092026_5day_008.zip"}},
 {"id":"EP182026","binNumber":"EP3","name":"Rosa","classification":"PTC","intensity":"","pressure":"1006",
  "latitudeNumeric":"19.1","longitudeNumeric":-121.4,"movementDir":270,"movementSpeed":"5",
  "lastUpdate":"2026-10-08T15:00:00.000Z"},
 {"id":"ep202026","name":"Broken"}
]}"#;

#[test]
fn parses_storms_leniently_and_skips_malformed() {
    let storms = parse_current_storms(CURRENT_STORMS.as_bytes()).unwrap();
    assert_eq!(storms.len(), 2);

    let isaias = &storms[0];
    assert_eq!(isaias.id, "al092026");
    assert_eq!(isaias.bin_number, "AT4");
    assert_eq!(isaias.intensity_kt, 75);
    assert_eq!(isaias.pressure_mb, 975);
    assert_eq!((isaias.lat, isaias.lon), (23.7, -90.2));
    assert_eq!(isaias.advisory_num, "008");
    assert_eq!(isaias.issuance, Utc.with_ymd_and_hms(2026, 10, 8, 15, 0, 0).unwrap());
    assert_eq!(isaias.forecast_zip_url.as_deref(), Some("https://www.nhc.noaa.gov/gis/forecast/archive/al092026_5day_008.zip"));

    let rosa = &storms[1];
    assert_eq!(rosa.id, "ep182026", "ids are lower-cased");
    assert_eq!(rosa.intensity_kt, 0, "empty intensity becomes 0");
    assert_eq!(rosa.lat, 19.1, "numeric strings are accepted");
    assert_eq!(rosa.movement_speed_mph, 5);
    assert_eq!(rosa.advisory_num, "");
    assert_eq!(rosa.public_advisory_url, None);
}

#[test]
fn empty_and_invalid_feeds() {
    assert!(parse_current_storms(br#"{"activeStorms":[]}"#).unwrap().is_empty());
    assert!(parse_current_storms(b"{}").is_err());
    assert!(parse_current_storms(b"<html>").is_err());
}

#[test]
fn saffir_simpson_categories() {
    let cases = [(33, "TD"), (34, "TS"), (63, "TS"), (64, "1"), (82, "1"), (83, "2"), (95, "2"),
                 (96, "3"), (112, "3"), (113, "4"), (136, "4"), (137, "5")];
    for (kt, expected) in cases {
        assert_eq!(category(kt), expected, "{kt} kt");
    }
}

#[test]
fn advisory_numbers() {
    assert_eq!(advisory_number("008"), Some(8));
    assert_eq!(advisory_number("8"), Some(8));
    assert_eq!(advisory_number("008A"), Some(8));
    assert_eq!(advisory_number(""), None);
}

#[test]
fn layer_index_maps_names_to_ids() {
    let index = parse_layer_index(br#"{"layers":[{"id":86,"name":"AT4 Forecast Cone","parentLayerId":81},{"id":4,"name":"AT1","parentLayerId":-1}]}"#).unwrap();
    assert_eq!(index.get("AT4 Forecast Cone"), Some(&86));
    assert_eq!(index.get("AT1"), Some(&4));
    assert!(parse_layer_index(b"{}").is_err());
    assert_eq!(
        layer_query_url(86),
        "https://mapservices.weather.noaa.gov/tropical/rest/services/tropical/NHC_tropical_weather/MapServer/86/query?where=1%3D1&outFields=*&f=geojson"
    );
}

#[test]
fn tag_features_adds_ids_and_wind() {
    let fc = json!({"type": "FeatureCollection", "features": [
        {"type": "Feature", "geometry": null, "properties": {"maxwind": 75}},
        {"type": "Feature", "geometry": null, "properties": null},
        {"type": "Feature", "geometry": null, "properties": {"intensity": 15}},
        "not a feature"
    ]});
    let tagged = tag_features(fc, "al092026", "forecast_points").unwrap();
    assert_eq!(tagged.len(), 3);
    for f in &tagged {
        assert_eq!(f["properties"]["storm_id"], "al092026");
        assert_eq!(f["properties"]["layer"], "forecast_points");
    }
    assert_eq!(tagged[0]["properties"]["wind_kt"], 75.0);
    assert!(tagged[1]["properties"].get("wind_kt").is_none());
    assert_eq!(tagged[2]["properties"]["wind_kt"], 15.0);

    // ArcGIS reports query errors as HTTP 200 with an "error" object.
    assert!(tag_features(json!({"error": {"code": 400}}), "x", "cone").is_err());
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --test nhc_parse`
Expected: compile errors such as `cannot find function parse_current_storms`.

- [ ] **Step 3: Implement `src/sources/nhc.rs`**

```rust
//! National Hurricane Center: active storms and their forecast geometry.

use std::collections::HashMap;

use anyhow::{bail, Context};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::warn;

pub const SOURCE: &str = "nhc";
pub const CURRENT_STORMS_URL: &str = "https://www.nhc.noaa.gov/CurrentStorms.json";
pub const MAPSERVER_URL: &str =
    "https://mapservices.weather.noaa.gov/tropical/rest/services/tropical/NHC_tropical_weather/MapServer";

/// Map-service layer name suffix -> our layer tag. Full names look like "AT4 Forecast Cone".
pub const STORM_LAYERS: [(&str, &str); 6] = [
    ("Forecast Cone", "cone"),
    ("Forecast Track", "forecast_track"),
    ("Forecast Points", "forecast_points"),
    ("Past Track", "past_track"),
    ("Past Points", "past_points"),
    ("Forecast Wind Radii", "wind_radii"),
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StormInfo {
    pub id: String,
    pub bin_number: String,
    pub name: String,
    pub classification: String,
    pub intensity_kt: u32,
    pub pressure_mb: u32,
    pub lat: f64,
    pub lon: f64,
    pub movement_dir: u32,
    pub movement_speed_mph: u32,
    pub advisory_num: String,
    pub issuance: DateTime<Utc>,
    pub public_advisory_url: Option<String>,
    pub forecast_zip_url: Option<String>,
}

pub fn layer_index_url() -> String {
    format!("{MAPSERVER_URL}?f=json")
}

pub fn layer_query_url(layer_id: u32) -> String {
    format!("{MAPSERVER_URL}/{layer_id}/query?where=1%3D1&outFields=*&f=geojson")
}

pub fn parse_current_storms(bytes: &[u8]) -> anyhow::Result<Vec<StormInfo>> {
    let root: Value = serde_json::from_slice(bytes).context("parsing CurrentStorms.json")?;
    let storms = root
        .get("activeStorms")
        .and_then(Value::as_array)
        .context("CurrentStorms.json has no activeStorms array")?;
    Ok(storms
        .iter()
        .filter_map(|raw| {
            let storm = parse_storm(raw);
            if storm.is_none() {
                warn!("skipping malformed NHC storm entry: {raw}");
            }
            storm
        })
        .collect())
}

fn str_field<'a>(v: &'a Value, pointer: &str) -> Option<&'a str> {
    v.pointer(pointer)?.as_str()
}

/// NHC sends numbers both as JSON numbers and as strings (sometimes empty).
fn num_field(v: &Value, pointer: &str) -> Option<f64> {
    match v.pointer(pointer)? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn parse_storm(v: &Value) -> Option<StormInfo> {
    let issuance = str_field(v, "/publicAdvisory/issuance").or_else(|| str_field(v, "/lastUpdate"))?;
    let issuance = DateTime::parse_from_rfc3339(issuance).ok()?.with_timezone(&Utc);
    let whole = |pointer: &str| num_field(v, pointer).unwrap_or(0.0).max(0.0) as u32;
    Some(StormInfo {
        id: str_field(v, "/id")?.to_lowercase(),
        bin_number: str_field(v, "/binNumber")?.to_string(),
        name: str_field(v, "/name").unwrap_or("Unnamed").to_string(),
        classification: str_field(v, "/classification").unwrap_or("").to_string(),
        intensity_kt: whole("/intensity"),
        pressure_mb: whole("/pressure"),
        lat: num_field(v, "/latitudeNumeric")?,
        lon: num_field(v, "/longitudeNumeric")?,
        movement_dir: whole("/movementDir"),
        movement_speed_mph: whole("/movementSpeed"),
        advisory_num: str_field(v, "/publicAdvisory/advNum").unwrap_or("").to_string(),
        issuance,
        public_advisory_url: str_field(v, "/publicAdvisory/url").map(String::from),
        forecast_zip_url: str_field(v, "/forecastTrack/zipFile").map(String::from),
    })
}

pub fn parse_layer_index(bytes: &[u8]) -> anyhow::Result<HashMap<String, u32>> {
    let root: Value = serde_json::from_slice(bytes).context("parsing map service index")?;
    let layers = root.get("layers").and_then(Value::as_array).context("map service index has no layers")?;
    Ok(layers
        .iter()
        .filter_map(|l| {
            let name = l.get("name")?.as_str()?.to_string();
            let id = u32::try_from(l.get("id")?.as_u64()?).ok()?;
            Some((name, id))
        })
        .collect())
}

/// Unwraps a FeatureCollection and tags each feature for the frontend.
pub fn tag_features(collection: Value, storm_id: &str, layer: &str) -> anyhow::Result<Vec<Value>> {
    let Value::Object(mut object) = collection else {
        bail!("expected a GeoJSON object");
    };
    let Some(Value::Array(features)) = object.remove("features") else {
        bail!("GeoJSON has no features array: {}", Value::Object(object));
    };
    Ok(features
        .into_iter()
        .filter_map(|mut feature| {
            let props = feature.as_object_mut()?.entry("properties").or_insert_with(|| json!({}));
            if !props.is_object() {
                *props = json!({});
            }
            let props = props.as_object_mut()?;
            let wind = props.get("maxwind").or_else(|| props.get("intensity")).and_then(Value::as_f64);
            props.insert("storm_id".into(), json!(storm_id));
            props.insert("layer".into(), json!(layer));
            if let Some(wind) = wind {
                props.insert("wind_kt".into(), json!(wind));
            }
            Some(feature)
        })
        .collect())
}

/// Saffir-Simpson category from max sustained wind in knots.
pub fn category(kt: u32) -> &'static str {
    match kt {
        0..=33 => "TD",
        34..=63 => "TS",
        64..=82 => "1",
        83..=95 => "2",
        96..=112 => "3",
        113..=136 => "4",
        _ => "5",
    }
}

/// "008" / "008A" -> 8.
pub fn advisory_number(advisory: &str) -> Option<u32> {
    let digits: String = advisory.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --test nhc_parse`
Expected: 6 passed.

- [ ] **Step 5: Commit**

```bash
git add src/sources/ tests/nhc_parse.rs
git commit -m "feat: parse NHC active storms and map-service layers" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 9: NHC advisory archive

**Files:**
- Create: `src/nhc_archive.rs`, `tests/nhc_archive.rs`
- Modify: `src/lib.rs` (add `pub mod nhc_archive;`), `tests/common/mod.rs` (add `isaias()`)

**Interfaces:**
- Consumes: `sources::nhc::StormInfo`
- Produces:
  - `StormSnapshot { storm: StormInfo, features: Vec<Value>, geometry_source: String }`. `geometry_source` is `"mapserver"`, `"zip"` or `GEOMETRY_NONE`.
  - `nhc_archive::GEOMETRY_NONE: &str = "none"` and `STALE_AFTER_HOURS: i64 = 12`
  - `NhcArchive::open(dir: impl Into<PathBuf>) -> anyhow::Result<NhcArchive>`
  - `store(&self, &StormSnapshot) -> anyhow::Result<()>`
  - `has_geometry(&self, storm_id: &str, issuance) -> bool` (false when the snapshot is missing or `"none"`)
  - `get(&self, storm_id: &str, t) -> anyhow::Result<Option<StormSnapshot>>`. Returns the newest snapshot issued at or before `t` and less than 12 h old. Invalid ids give `None`.
  - `at_time(&self, t) -> anyhow::Result<Vec<StormSnapshot>>` (sorted by id)
  - `prune(&self, cutoff) -> anyhow::Result<usize>`
  - `common::isaias() -> StormInfo`, issued 2026-10-08T15:00Z

- [ ] **Step 1: Write the failing tests**

Add `pub mod nhc_archive;` to `src/lib.rs`. Append to `tests/common/mod.rs`:

```rust
pub fn isaias() -> rust_radar::sources::nhc::StormInfo {
    use chrono::TimeZone;
    rust_radar::sources::nhc::StormInfo {
        id: "al092026".into(),
        bin_number: "AT4".into(),
        name: "Isaias".into(),
        classification: "HU".into(),
        intensity_kt: 75,
        pressure_mb: 975,
        lat: 23.7,
        lon: -90.2,
        movement_dir: 60,
        movement_speed_mph: 10,
        advisory_num: "008".into(),
        issuance: chrono::Utc.with_ymd_and_hms(2026, 10, 8, 15, 0, 0).unwrap(),
        public_advisory_url: Some("https://www.nhc.noaa.gov/text/MIATCPAT4.shtml".into()),
        forecast_zip_url: Some("https://www.nhc.noaa.gov/gis/forecast/archive/al092026_5day_008.zip".into()),
    }
}
```

`tests/nhc_archive.rs`:

```rust
mod common;

use chrono::{DateTime, Duration, TimeZone, Utc};
use serde_json::json;
use rust_radar::nhc_archive::{NhcArchive, StormSnapshot, GEOMETRY_NONE};

fn t(d: u32, h: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, d, h, 0, 0).unwrap()
}

fn snapshot(issuance: DateTime<Utc>, advisory: &str, source: &str) -> StormSnapshot {
    let mut storm = common::isaias();
    storm.issuance = issuance;
    storm.advisory_num = advisory.into();
    StormSnapshot {
        storm,
        features: vec![json!({"type": "Feature", "geometry": null, "properties": {"layer": "cone"}})],
        geometry_source: source.into(),
    }
}

#[test]
fn returns_advisory_current_at_time() {
    let dir = tempfile::tempdir().unwrap();
    let archive = NhcArchive::open(dir.path()).unwrap();
    archive.store(&snapshot(t(8, 9), "007", "mapserver")).unwrap();
    archive.store(&snapshot(t(8, 15), "008", "mapserver")).unwrap();

    assert!(archive.at_time(t(8, 8)).unwrap().is_empty(), "before the first advisory");
    assert_eq!(archive.at_time(t(8, 12)).unwrap()[0].storm.advisory_num, "007");
    assert_eq!(archive.at_time(t(8, 15)).unwrap()[0].storm.advisory_num, "008");
    assert_eq!(archive.get("al092026", t(8, 20)).unwrap().unwrap().storm.advisory_num, "008");
    assert!(archive.get("al092026", t(8, 15) + Duration::hours(13)).unwrap().is_none(), "stale after 12h");
    assert!(archive.get("al012026", t(8, 15)).unwrap().is_none(), "unknown storm");
}

#[test]
fn rejects_path_traversal_ids() {
    let dir = tempfile::tempdir().unwrap();
    let archive = NhcArchive::open(dir.path().join("nhc")).unwrap();
    assert!(archive.get("../..", t(8, 15)).unwrap().is_none());
    assert!(archive.get("", t(8, 15)).unwrap().is_none());
}

#[test]
fn has_geometry_only_for_real_geometry() {
    let dir = tempfile::tempdir().unwrap();
    let archive = NhcArchive::open(dir.path()).unwrap();
    assert!(!archive.has_geometry("al092026", t(8, 15)));
    archive.store(&snapshot(t(8, 15), "008", GEOMETRY_NONE)).unwrap();
    assert!(!archive.has_geometry("al092026", t(8, 15)));
    archive.store(&snapshot(t(8, 15), "008", "zip")).unwrap();
    assert!(archive.has_geometry("al092026", t(8, 15)));
}

#[test]
fn prune_removes_old_snapshots_and_leftovers() {
    let dir = tempfile::tempdir().unwrap();
    let archive = NhcArchive::open(dir.path()).unwrap();
    archive.store(&snapshot(t(5, 9), "001", "mapserver")).unwrap();
    archive.store(&snapshot(t(8, 15), "008", "mapserver")).unwrap();
    std::fs::write(dir.path().join("al092026").join("20261008-150000.json.tmp"), b"partial").unwrap();
    std::fs::create_dir_all(dir.path().join("al012026")).unwrap();
    std::fs::write(dir.path().join("al012026").join("notes.txt"), b"stray").unwrap();

    assert_eq!(archive.prune(t(6, 0)).unwrap(), 1);
    assert!(archive.get("al092026", t(5, 10)).unwrap().is_none());
    assert!(archive.get("al092026", t(8, 16)).unwrap().is_some());
    assert!(!dir.path().join("al092026").join("20261008-150000.json.tmp").exists());
    assert_eq!(archive.at_time(t(8, 16)).unwrap().len(), 1, "stray files are ignored");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --test nhc_archive`
Expected: compile error `could not find nhc_archive`.

- [ ] **Step 3: Implement `src/nhc_archive.rs`**

```rust
//! On-disk NHC advisory snapshots: `<dir>/<storm_id>/<issuance>.json`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::sources::nhc::StormInfo;

pub const GEOMETRY_NONE: &str = "none";
/// A storm with no advisory for this long is treated as gone.
pub const STALE_AFTER_HOURS: i64 = 12;
const TIME_FORMAT: &str = "%Y%m%d-%H%M%S";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StormSnapshot {
    pub storm: StormInfo,
    pub features: Vec<Value>,
    pub geometry_source: String,
}

pub struct NhcArchive {
    dir: PathBuf,
}

fn valid_storm_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric())
}

fn snapshot_file_name(issuance: DateTime<Utc>) -> String {
    format!("{}.json", issuance.format(TIME_FORMAT))
}

fn parse_snapshot_file_name(name: &str) -> Option<DateTime<Utc>> {
    let stem = name.strip_suffix(".json")?;
    NaiveDateTime::parse_from_str(stem, TIME_FORMAT).ok().map(|n| n.and_utc())
}

fn read_snapshot(path: &Path) -> anyhow::Result<StormSnapshot> {
    let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))
}

impl NhcArchive {
    pub fn open(dir: impl Into<PathBuf>) -> anyhow::Result<NhcArchive> {
        let dir = dir.into();
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        Ok(NhcArchive { dir })
    }

    fn snapshot_path(&self, storm_id: &str, issuance: DateTime<Utc>) -> PathBuf {
        self.dir.join(storm_id).join(snapshot_file_name(issuance))
    }

    pub fn store(&self, snapshot: &StormSnapshot) -> anyhow::Result<()> {
        let storm_id = &snapshot.storm.id;
        anyhow::ensure!(valid_storm_id(storm_id), "invalid storm id {storm_id:?}");
        let path = self.snapshot_path(storm_id, snapshot.storm.issuance);
        fs::create_dir_all(path.parent().expect("snapshot path has a parent"))?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec(snapshot)?).with_context(|| format!("writing {}", tmp.display()))?;
        fs::rename(&tmp, &path).with_context(|| format!("renaming to {}", path.display()))?;
        Ok(())
    }

    pub fn has_geometry(&self, storm_id: &str, issuance: DateTime<Utc>) -> bool {
        valid_storm_id(storm_id)
            && read_snapshot(&self.snapshot_path(storm_id, issuance))
                .is_ok_and(|s| s.geometry_source != GEOMETRY_NONE)
    }

    fn issuances(&self, storm_id: &str) -> anyhow::Result<Vec<DateTime<Utc>>> {
        let dir = self.dir.join(storm_id);
        if !dir.is_dir() {
            return Ok(Vec::new());
        }
        let mut times = Vec::new();
        for entry in fs::read_dir(&dir).with_context(|| format!("listing {}", dir.display()))? {
            if let Some(t) = entry?.file_name().to_str().and_then(parse_snapshot_file_name) {
                times.push(t);
            }
        }
        times.sort();
        Ok(times)
    }

    pub fn get(&self, storm_id: &str, t: DateTime<Utc>) -> anyhow::Result<Option<StormSnapshot>> {
        if !valid_storm_id(storm_id) {
            return Ok(None);
        }
        let current = self.issuances(storm_id)?.into_iter().filter(|&i| i <= t).last();
        match current {
            Some(issuance) if t - issuance <= Duration::hours(STALE_AFTER_HOURS) => {
                Ok(Some(read_snapshot(&self.snapshot_path(storm_id, issuance))?))
            }
            _ => Ok(None),
        }
    }

    pub fn at_time(&self, t: DateTime<Utc>) -> anyhow::Result<Vec<StormSnapshot>> {
        let mut snapshots = Vec::new();
        for entry in fs::read_dir(&self.dir).with_context(|| format!("listing {}", self.dir.display()))? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            if let Some(id) = entry.file_name().to_str() {
                if let Some(snapshot) = self.get(id, t)? {
                    snapshots.push(snapshot);
                }
            }
        }
        snapshots.sort_by(|a, b| a.storm.id.cmp(&b.storm.id));
        Ok(snapshots)
    }

    /// Removes snapshots issued before `cutoff`, leftover temp files, and emptied storm dirs.
    pub fn prune(&self, cutoff: DateTime<Utc>) -> anyhow::Result<usize> {
        let mut removed = 0;
        for storm_dir in fs::read_dir(&self.dir)? {
            let storm_dir = storm_dir?.path();
            if !storm_dir.is_dir() {
                continue;
            }
            for file in fs::read_dir(&storm_dir)? {
                let path = file?.path();
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                if name.ends_with(".tmp") {
                    fs::remove_file(&path)?;
                } else if parse_snapshot_file_name(name).is_some_and(|t| t < cutoff) {
                    fs::remove_file(&path)?;
                    removed += 1;
                }
            }
            if fs::read_dir(&storm_dir)?.next().is_none() {
                fs::remove_dir(&storm_dir)?;
            }
        }
        Ok(removed)
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --test nhc_archive`
Expected: 4 passed.

- [ ] **Step 5: Commit**

```bash
git add src/lib.rs src/nhc_archive.rs tests/
git commit -m "feat: per-advisory NHC snapshot archive" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 10: NHC shapefile-zip fallback

**Files:**
- Create: `src/sources/nhc_zip.rs`, `tests/nhc_zip.rs`, `tests/fixtures/nhc_5day.zip`
- Modify: `src/sources/mod.rs` (add `pub mod nhc_zip;`), `tests/common/mod.rs` (add `nhc_zip_fixture()`)

**Interfaces:**
- Consumes: `sources::nhc::tag_features`
- Produces:
  - `nhc_zip::features_from_zip(bytes: &[u8], storm_id: &str) -> anyhow::Result<Vec<Value>>`. Returns `cone`, `forecast_track` and `forecast_points` features. It is an error if there is no cone.
  - `common::nhc_zip_fixture() -> Vec<u8>`

- [ ] **Step 1: Download the fixture**

```bash
curl -sL -o tests/fixtures/nhc_5day.zip https://www.nhc.noaa.gov/gis/forecast/archive/al092026_5day_008.zip
unzip -l tests/fixtures/nhc_5day.zip   # expect *_5day_pgn, *_5day_lin, *_5day_pts .shp/.dbf
```

- [ ] **Step 2: Write the failing tests**

Add `pub mod nhc_zip;` to `src/sources/mod.rs`. Append to `tests/common/mod.rs`:

```rust
pub fn nhc_zip_fixture() -> Vec<u8> {
    std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/nhc_5day.zip"))
        .expect("tests/fixtures/nhc_5day.zip missing - see Task 10 Step 1")
}
```

`tests/nhc_zip.rs`:

```rust
mod common;

use rust_radar::sources::nhc_zip::features_from_zip;
use serde_json::Value;

fn of_layer<'a>(features: &'a [Value], layer: &str) -> Vec<&'a Value> {
    features.iter().filter(|f| f["properties"]["layer"] == layer).collect()
}

#[test]
fn reads_cone_track_and_points() {
    let features = features_from_zip(&common::nhc_zip_fixture(), "al092026").unwrap();
    assert!(features.iter().all(|f| f["properties"]["storm_id"] == "al092026"));

    let cone = of_layer(&features, "cone");
    assert!(!cone.is_empty());
    assert!(matches!(cone[0]["geometry"]["type"].as_str(), Some("Polygon" | "MultiPolygon")));

    let track = of_layer(&features, "forecast_track");
    assert!(!track.is_empty());
    assert!(matches!(track[0]["geometry"]["type"].as_str(), Some("LineString" | "MultiLineString")));

    let points = of_layer(&features, "forecast_points");
    assert!(points.len() >= 3);
    assert_eq!(points[0]["geometry"]["type"], "Point");
    assert!(points[0]["properties"]["wind_kt"].is_number(), "MAXWIND is lower-cased and mapped");
}

#[test]
fn rejects_non_zip() {
    assert!(features_from_zip(b"not a zip", "al092026").is_err());
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test --test nhc_zip`
Expected: compile error `cannot find function features_from_zip`.

- [ ] **Step 4: Implement `src/sources/nhc_zip.rs`**

The code below is written against shapefile 0.6 and the `dbase` version it re-exports. If `cargo build` reports a name mismatch, check the actual items with `cargo doc -p shapefile --open` and adjust the code to match. The candidates are `FieldValue` variants, `PolygonRing`, and `Shape::*M`/`*Z` variants. The tests above define the required behaviour.

```rust
//! Fallback geometry: NHC's per-advisory 5-day shapefile zip (cone, track, points).

use std::io::{Cursor, Read};

use anyhow::{bail, Context};
use serde_json::{json, Map, Value};
use shapefile::dbase::{FieldValue, Record};
use shapefile::{PolygonRing, Shape};

use crate::sources::nhc::tag_features;

/// Shapefile name suffix inside the zip -> our layer tag.
const ZIP_LAYERS: [(&str, &str); 3] = [
    ("_5day_pgn", "cone"),
    ("_5day_lin", "forecast_track"),
    ("_5day_pts", "forecast_points"),
];

pub fn features_from_zip(bytes: &[u8], storm_id: &str) -> anyhow::Result<Vec<Value>> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).context("opening advisory zip")?;
    let mut out = Vec::new();
    for (suffix, layer) in ZIP_LAYERS {
        let shp = read_entry(&mut zip, &format!("{suffix}.shp"))?;
        let dbf = read_entry(&mut zip, &format!("{suffix}.dbf"))?;
        let shapes = shapefile::ShapeReader::new(Cursor::new(shp)).with_context(|| format!("reading {suffix}.shp"))?;
        let records = shapefile::dbase::Reader::new(Cursor::new(dbf)).with_context(|| format!("reading {suffix}.dbf"))?;
        let mut reader = shapefile::Reader::new(shapes, records);
        let mut features = Vec::new();
        for item in reader.iter_shapes_and_records() {
            let (shape, record) = item.with_context(|| format!("reading a {suffix} record"))?;
            features.push(json!({
                "type": "Feature",
                "geometry": shape_to_geojson(&shape)?,
                "properties": record_to_json(record),
            }));
        }
        out.extend(tag_features(json!({"type": "FeatureCollection", "features": features}), storm_id, layer)?);
    }
    if !out.iter().any(|f| f["properties"]["layer"] == "cone") {
        bail!("advisory zip has no forecast cone");
    }
    Ok(out)
}

fn read_entry(zip: &mut zip::ZipArchive<Cursor<&[u8]>>, suffix: &str) -> anyhow::Result<Vec<u8>> {
    let name = zip
        .file_names()
        .find(|n| n.ends_with(suffix))
        .map(str::to_owned)
        .with_context(|| format!("advisory zip has no *{suffix}"))?;
    let mut buf = Vec::new();
    zip.by_name(&name)?.read_to_end(&mut buf)?;
    Ok(buf)
}

trait Xy {
    fn xy(&self) -> [f64; 2];
}
impl Xy for shapefile::Point {
    fn xy(&self) -> [f64; 2] { [self.x, self.y] }
}
impl Xy for shapefile::PointM {
    fn xy(&self) -> [f64; 2] { [self.x, self.y] }
}
impl Xy for shapefile::PointZ {
    fn xy(&self) -> [f64; 2] { [self.x, self.y] }
}

fn coords<P: Xy>(points: &[P]) -> Vec<[f64; 2]> {
    points.iter().map(Xy::xy).collect()
}

fn lines<P: Xy>(parts: &[Vec<P>]) -> Value {
    if parts.len() == 1 {
        json!({"type": "LineString", "coordinates": coords(&parts[0])})
    } else {
        json!({"type": "MultiLineString", "coordinates": parts.iter().map(|p| coords(p)).collect::<Vec<_>>()})
    }
}

/// Each outer ring starts a polygon; inner rings are holes in the preceding polygon.
fn polygons<P: Xy>(rings: &[PolygonRing<P>]) -> Value {
    let mut polygons: Vec<Vec<Vec<[f64; 2]>>> = Vec::new();
    for ring in rings {
        match ring {
            PolygonRing::Outer(points) => polygons.push(vec![coords(points)]),
            PolygonRing::Inner(points) => match polygons.last_mut() {
                Some(polygon) => polygon.push(coords(points)),
                None => polygons.push(vec![coords(points)]),
            },
        }
    }
    if polygons.len() == 1 {
        json!({"type": "Polygon", "coordinates": polygons.remove(0)})
    } else {
        json!({"type": "MultiPolygon", "coordinates": polygons})
    }
}

fn shape_to_geojson(shape: &Shape) -> anyhow::Result<Value> {
    Ok(match shape {
        Shape::Point(p) => json!({"type": "Point", "coordinates": p.xy()}),
        Shape::PointM(p) => json!({"type": "Point", "coordinates": p.xy()}),
        Shape::PointZ(p) => json!({"type": "Point", "coordinates": p.xy()}),
        Shape::Polyline(l) => lines(l.parts()),
        Shape::PolylineM(l) => lines(l.parts()),
        Shape::PolylineZ(l) => lines(l.parts()),
        Shape::Polygon(p) => polygons(p.rings()),
        Shape::PolygonM(p) => polygons(p.rings()),
        Shape::PolygonZ(p) => polygons(p.rings()),
        _ => bail!("unsupported shapefile geometry"),
    })
}

/// DBF attributes as JSON with lower-cased keys, matching the map service's field names.
fn record_to_json(record: Record) -> Value {
    let mut props = Map::new();
    for (name, value) in record {
        let value = match value {
            FieldValue::Character(Some(s)) => json!(s.trim()),
            FieldValue::Numeric(Some(n)) => json!(n),
            FieldValue::Float(Some(f)) => json!(f),
            FieldValue::Double(d) => json!(d),
            FieldValue::Integer(i) => json!(i),
            FieldValue::Logical(Some(b)) => json!(b),
            _ => Value::Null,
        };
        props.insert(name.to_lowercase(), value);
    }
    Value::Object(props)
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --test nhc_zip`
Expected: 2 passed.

- [ ] **Step 6: Commit**

```bash
git add src/sources/ tests/
git commit -m "feat: NHC shapefile-zip geometry fallback" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 11: NHC poller

**Files:**
- Modify: `src/sources/nhc.rs` (append poller code)
- Create: `tests/nhc_source.rs`

**Interfaces:**
- Consumes:
  - `Fetcher`
  - `NhcArchive::{store, has_geometry}`, `StormSnapshot`, `GEOMETRY_NONE`
  - `nhc_zip::features_from_zip`
  - The Task 8 parsers
  - `Status`, `backoff_delay`
- Produces:
  - `NhcPollReport { active: usize, stored: usize }` (`Debug, Default, PartialEq`)
  - `poll_once(&dyn Fetcher, &NhcArchive) -> anyhow::Result<NhcPollReport>`
  - `fetch_mapserver_features(&dyn Fetcher, &HashMap<String, u32>, &StormInfo) -> anyhow::Result<Vec<Value>>`
  - `run(Arc<dyn Fetcher>, Arc<NhcArchive>, Status, interval: Duration)`

**Behaviour:**
1. For each active storm without real geometry stored for its advisory, try the map service first.
2. If the map service is unreachable, is missing a layer, or still shows a different advisory number, try the advisory zip.
3. If the zip also fails, store the snapshot with `geometry_source = "none"`, so the storm centre still shows. A later poll retries.

- [ ] **Step 1: Write the failing tests**

`tests/nhc_source.rs`:

```rust
mod common;

use rust_radar::fetch::FakeFetcher;
use rust_radar::nhc_archive::NhcArchive;
use rust_radar::sources::nhc::*;
use serde_json::{json, Value};

const STORMS: &str = r#"{"activeStorms":[{"id":"al092026","binNumber":"AT4","name":"Isaias","classification":"HU",
 "intensity":"75","pressure":"975","latitudeNumeric":23.7,"longitudeNumeric":-90.2,"movementDir":60,"movementSpeed":10,
 "lastUpdate":"2026-10-08T15:00:00.000Z",
 "publicAdvisory":{"advNum":"008","issuance":"2026-10-08T15:00:00.000Z","url":"https://www.nhc.noaa.gov/text/MIATCPAT4.shtml"},
 "forecastTrack":{"advNum":"008","zipFile":"https://www.nhc.noaa.gov/gis/forecast/archive/al092026_5day_008.zip"}}]}"#;
const ZIP_URL: &str = "https://www.nhc.noaa.gov/gis/forecast/archive/al092026_5day_008.zip";

/// Layer ids for the fake map service, in STORM_LAYERS order.
const LAYER_IDS: [u32; 6] = [86, 85, 84, 89, 88, 92];

fn layer_index() -> String {
    let layers: Vec<Value> = STORM_LAYERS
        .iter()
        .zip(LAYER_IDS)
        .map(|((suffix, _), id)| json!({"id": id, "name": format!("AT4 {suffix}")}))
        .collect();
    json!({ "layers": layers }).to_string()
}

fn fake_with_mapserver(cone_advisory: &str) -> FakeFetcher {
    let fake = FakeFetcher::new();
    fake.ok(CURRENT_STORMS_URL, STORMS);
    fake.ok(&layer_index_url(), layer_index());
    for ((_, layer), id) in STORM_LAYERS.iter().zip(LAYER_IDS) {
        let props = if *layer == "cone" { json!({"advisnum": cone_advisory}) } else { json!({}) };
        let fc = json!({"type": "FeatureCollection", "features": [
            {"type": "Feature", "geometry": {"type": "Point", "coordinates": [-90.0, 24.0]}, "properties": props}
        ]});
        fake.ok(&layer_query_url(id), fc.to_string());
    }
    fake
}

fn archive() -> (tempfile::TempDir, NhcArchive) {
    let dir = tempfile::tempdir().unwrap();
    let archive = NhcArchive::open(dir.path()).unwrap();
    (dir, archive)
}

fn stored(archive: &NhcArchive) -> rust_radar::nhc_archive::StormSnapshot {
    archive.get("al092026", common::isaias().issuance).unwrap().expect("snapshot stored")
}

#[tokio::test]
async fn stores_mapserver_geometry_once_per_advisory() {
    let (_dir, archive) = archive();
    let fake = fake_with_mapserver("8");
    assert_eq!(poll_once(&fake, &archive).await.unwrap(), NhcPollReport { active: 1, stored: 1 });
    let snapshot = stored(&archive);
    assert_eq!(snapshot.geometry_source, "mapserver");
    assert_eq!(snapshot.features.len(), 6);
    assert_eq!(snapshot.storm.name, "Isaias");

    assert_eq!(poll_once(&fake, &archive).await.unwrap(), NhcPollReport { active: 1, stored: 0 });
    assert_eq!(fake.calls_to(&layer_query_url(86)), 1, "geometry is not refetched");
}

#[tokio::test]
async fn lagging_mapserver_falls_back_to_zip() {
    let (_dir, archive) = archive();
    let fake = fake_with_mapserver("7");
    fake.ok(ZIP_URL, common::nhc_zip_fixture());
    poll_once(&fake, &archive).await.unwrap();
    assert_eq!(stored(&archive).geometry_source, "zip");
}

#[tokio::test]
async fn total_geometry_failure_still_stores_storm_and_retries() {
    let (_dir, archive) = archive();
    let fake = FakeFetcher::new();
    fake.ok(CURRENT_STORMS_URL, STORMS);
    fake.fail(&layer_index_url(), "503");
    fake.fail(ZIP_URL, "404");
    poll_once(&fake, &archive).await.unwrap();
    let snapshot = stored(&archive);
    assert_eq!(snapshot.geometry_source, "none");
    assert!(snapshot.features.is_empty());

    poll_once(&fake, &archive).await.unwrap();
    assert_eq!(fake.calls_to(ZIP_URL), 2, "retried on the next poll");
}

#[tokio::test]
async fn no_active_storms_skips_map_service() {
    let (_dir, archive) = archive();
    let fake = FakeFetcher::new();
    fake.ok(CURRENT_STORMS_URL, r#"{"activeStorms":[]}"#);
    assert_eq!(poll_once(&fake, &archive).await.unwrap(), NhcPollReport::default());
    assert_eq!(fake.calls_to(&layer_index_url()), 0);
}

#[tokio::test]
async fn feed_failure_is_an_error() {
    let (_dir, archive) = archive();
    let fake = FakeFetcher::new();
    fake.fail(CURRENT_STORMS_URL, "503");
    assert!(poll_once(&fake, &archive).await.is_err());
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --test nhc_source`
Expected: compile errors such as `cannot find function poll_once in module nhc` and `cannot find type NhcPollReport`.

- [ ] **Step 3: Append the poller to `src/sources/nhc.rs`**

Extend the imports at the top of the file:

```rust
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::{info, warn};

use crate::fetch::Fetcher;
use crate::nhc_archive::{NhcArchive, StormSnapshot, GEOMETRY_NONE};
use crate::sources::nhc_zip;
use crate::status::{backoff_delay, Status};
```

Append:

```rust
#[derive(Debug, Default, PartialEq)]
pub struct NhcPollReport {
    pub active: usize,
    pub stored: usize,
}

pub async fn poll_once(fetcher: &dyn Fetcher, archive: &NhcArchive) -> anyhow::Result<NhcPollReport> {
    let body = fetcher.get(CURRENT_STORMS_URL).await.context("fetching CurrentStorms.json")?;
    let storms = parse_current_storms(&body)?;
    let mut report = NhcPollReport { active: storms.len(), stored: 0 };
    let pending: Vec<&StormInfo> = storms.iter().filter(|s| !archive.has_geometry(&s.id, s.issuance)).collect();
    if pending.is_empty() {
        return Ok(report);
    }
    let index = match fetcher.get(&layer_index_url()).await.and_then(|b| parse_layer_index(&b)) {
        Ok(index) => Some(index),
        Err(e) => {
            warn!("NHC map service index unavailable: {e:#}");
            None
        }
    };
    for storm in pending {
        let (features, geometry_source) = fetch_geometry(fetcher, index.as_ref(), storm).await;
        archive.store(&StormSnapshot {
            storm: storm.clone(),
            features,
            geometry_source: geometry_source.to_string(),
        })?;
        info!("stored NHC advisory {} for {} ({geometry_source})", storm.advisory_num, storm.name);
        report.stored += 1;
    }
    Ok(report)
}

async fn fetch_geometry(
    fetcher: &dyn Fetcher,
    index: Option<&HashMap<String, u32>>,
    storm: &StormInfo,
) -> (Vec<Value>, &'static str) {
    if let Some(index) = index {
        match fetch_mapserver_features(fetcher, index, storm).await {
            Ok(features) => return (features, "mapserver"),
            Err(e) => warn!("NHC map service geometry for {} failed: {e:#}", storm.id),
        }
    }
    if let Some(url) = &storm.forecast_zip_url {
        match fetcher.get(url).await.and_then(|b| nhc_zip::features_from_zip(&b, &storm.id)) {
            Ok(features) => return (features, "zip"),
            Err(e) => warn!("NHC advisory zip for {} failed: {e:#}", storm.id),
        }
    }
    (Vec::new(), GEOMETRY_NONE)
}

pub async fn fetch_mapserver_features(
    fetcher: &dyn Fetcher,
    index: &HashMap<String, u32>,
    storm: &StormInfo,
) -> anyhow::Result<Vec<Value>> {
    let mut features = Vec::new();
    for (suffix, layer) in STORM_LAYERS {
        let name = format!("{} {suffix}", storm.bin_number);
        let id = *index.get(&name).with_context(|| format!("map service has no layer {name:?}"))?;
        let body = fetcher.get(&layer_query_url(id)).await.with_context(|| format!("querying {name}"))?;
        let collection: Value = serde_json::from_slice(&body).with_context(|| format!("parsing {name}"))?;
        features.extend(tag_features(collection, &storm.id, layer).with_context(|| format!("reading {name}"))?);
    }
    check_advisory(&features, storm)?;
    Ok(features)
}

/// The map service can lag CurrentStorms.json; refuse geometry from a different advisory.
fn check_advisory(features: &[Value], storm: &StormInfo) -> anyhow::Result<()> {
    let found = features.iter().find_map(|f| {
        let v = f.pointer("/properties/advisnum")?;
        v.as_str().map(String::from).or_else(|| v.as_u64().map(|n| n.to_string()))
    });
    match (advisory_number(&storm.advisory_num), found.as_deref().and_then(advisory_number)) {
        (Some(expected), Some(actual)) if expected != actual => {
            bail!("map service shows advisory {actual}, expected {expected}")
        }
        _ => Ok(()),
    }
}

pub async fn run(fetcher: Arc<dyn Fetcher>, archive: Arc<NhcArchive>, status: Status, interval: Duration) {
    loop {
        let failures = match poll_once(fetcher.as_ref(), &archive).await {
            Ok(report) => {
                if report.stored > 0 {
                    info!("NHC: {} active storms, {} advisories stored", report.active, report.stored);
                }
                status.success(SOURCE, Utc::now());
                0
            }
            Err(e) => {
                warn!("NHC poll failed: {e:#}");
                status.failure(SOURCE, &e)
            }
        };
        tokio::time::sleep(backoff_delay(interval, failures)).await;
    }
}
```

- [ ] **Step 4: Run all NHC tests to verify they pass**

Run: `cargo test --test nhc_source --test nhc_parse --test nhc_zip --test nhc_archive`
Expected: all pass. That is 5 new tests, and the earlier NHC tests still pass.

- [ ] **Step 5: Commit**

```bash
git add src/sources/nhc.rs tests/nhc_source.rs
git commit -m "feat: NHC poller with map-service and zip fallbacks" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 12: HTTP API and tile server

**Files:**
- Create: `src/server.rs`, `tests/server.rs`
- Modify: `src/lib.rs` (add `pub mod server;`)

**Interfaces:**
- Consumes:
  - `MrmsArchive::{frames, latest, contains, load}`
  - `NhcArchive::{at_time, get}`
  - `tiles::{valid_tile, render_tile, MAX_ZOOM}`
  - `nhc::category`
  - `Status::snapshot`
- Produces:
  - `server::AppState::new(mrms: Arc<MrmsArchive>, nhc: Arc<NhcArchive>, status: Status, basemap_style: String) -> AppState`
  - `server::router(state: Arc<AppState>, web_dir: &Path) -> axum::Router`
- Routes:

  | Route | Response |
  |---|---|
  | `GET /api/config` | `{"basemap_style": "..."}` |
  | `GET /api/frames` | `{"frames": [rfc3339...], "latest": rfc3339 or null}` |
  | `GET /api/status` | `{"now": rfc3339, "sources": {"mrms": SourceStatus, "nhc": SourceStatus}}` |
  | `GET /api/storms?time=` | GeoJSON FeatureCollection. Includes each snapshot's features, plus one `layer:"center"` Point per storm with props `{storm_id, name, classification, category, wind_kt, pressure_mb, advisory_num, issuance}`. |
  | `GET /api/storms/{id}?time=` | `{"storm": StormInfo, "category": str, "geometry_source": str}` |
  | `GET /tiles/{time}/{z}/{x}/{y}.png` | `time` is RFC 3339 or `latest`. Explicit times are sent with `Cache-Control: public, max-age=86400`, and `latest` with `no-cache`. |
  | Anything else | Static files from `web_dir` |

- Errors are JSON `{"error": "..."}` with status 400 (bad input), 404 (unknown frame or storm) or 500 (logged).

- [ ] **Step 1: Write the failing tests**

Add `pub mod server;` to `src/lib.rs`. `tests/server.rs`:

```rust
mod common;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::{TimeZone, Utc};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use rust_radar::mrms_archive::MrmsArchive;
use rust_radar::nhc_archive::{NhcArchive, StormSnapshot};
use rust_radar::server::{router, AppState};
use rust_radar::status::Status;

const FRAME: &str = "2026-10-08T17:00:00Z";

struct Harness {
    app: axum::Router,
    rainy: (u32, u32),
    _dir: tempfile::TempDir,
}

fn harness() -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let mrms = Arc::new(MrmsArchive::open(dir.path().join("mrms"), 2).unwrap());
    let frame = Utc.with_ymd_and_hms(2026, 10, 8, 17, 0, 0).unwrap();
    let grid = mrms.store(frame, &common::mrms_fixture()).unwrap();
    let rainy = common::rainy_tile(&grid, 7);
    let nhc = Arc::new(NhcArchive::open(dir.path().join("nhc")).unwrap());
    nhc.store(&StormSnapshot {
        storm: common::isaias(),
        features: vec![json!({"type": "Feature",
            "geometry": {"type": "Polygon", "coordinates": [[[-90.0, 23.0], [-88.0, 23.0], [-88.0, 25.0], [-90.0, 23.0]]]},
            "properties": {"storm_id": "al092026", "layer": "cone"}})],
        geometry_source: "mapserver".into(),
    })
    .unwrap();
    let web = dir.path().join("web");
    std::fs::create_dir_all(&web).unwrap();
    std::fs::write(web.join("index.html"), "hello radar").unwrap();
    let state = Arc::new(AppState::new(mrms, nhc, Status::default(), "https://example.test/style.json".into()));
    Harness { app: router(state, &web), rainy, _dir: dir }
}

struct Reply {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: Vec<u8>,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap()
    }
}

async fn get(h: &Harness, uri: &str) -> Reply {
    let response = h.app.clone().oneshot(Request::get(uri).body(Body::empty()).unwrap()).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = response.into_body().collect().await.unwrap().to_bytes().to_vec();
    Reply { status, headers, body }
}

#[tokio::test]
async fn lists_frames() {
    let h = harness();
    let r = get(&h, "/api/frames").await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.json(), json!({"frames": [FRAME], "latest": FRAME}));
}

#[tokio::test]
async fn serves_rainy_tile_with_long_cache() {
    let h = harness();
    let (x, y) = h.rainy;
    let r = get(&h, &format!("/tiles/{FRAME}/7/{x}/{y}.png")).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.headers["content-type"], "image/png");
    assert_eq!(r.headers["cache-control"], "public, max-age=86400");
    assert!(common::opaque_pixels(&common::decode_png(&r.body)) > 0);

    let again = get(&h, &format!("/tiles/{FRAME}/7/{x}/{y}.png")).await;
    assert_eq!(again.body, r.body, "cached tile is identical");
}

#[tokio::test]
async fn latest_tile_is_not_cached_by_browser() {
    let h = harness();
    let (x, y) = h.rainy;
    let r = get(&h, &format!("/tiles/latest/7/{x}/{y}.png")).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.headers["cache-control"], "no-cache");
}

#[tokio::test]
async fn bad_tile_requests_are_client_errors() {
    let h = harness();
    assert_eq!(get(&h, "/tiles/2026-10-08T16:00:00Z/7/0/0.png").await.status, StatusCode::NOT_FOUND);
    let bad = get(&h, "/tiles/yesterday/7/0/0.png").await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
    assert!(bad.json()["error"].as_str().unwrap().contains("RFC 3339"));
    for uri in [
        format!("/tiles/{FRAME}/2/9/0.png"),
        format!("/tiles/{FRAME}/13/0/0.png"),
        format!("/tiles/{FRAME}/2/0/0"),
        format!("/tiles/{FRAME}/abc/0/0.png"),
    ] {
        assert_eq!(get(&h, &uri).await.status, StatusCode::BAD_REQUEST, "{uri}");
    }
}

#[tokio::test]
async fn storms_at_time() {
    let h = harness();
    let r = get(&h, "/api/storms?time=2026-10-08T18:00:00Z").await;
    assert_eq!(r.status, StatusCode::OK);
    let fc = r.json();
    assert_eq!(fc["type"], "FeatureCollection");
    let features = fc["features"].as_array().unwrap();
    assert_eq!(features.len(), 2);
    let center = features.iter().find(|f| f["properties"]["layer"] == "center").unwrap();
    assert_eq!(center["properties"]["name"], "Isaias");
    assert_eq!(center["properties"]["category"], "1");
    assert_eq!(center["geometry"]["coordinates"], json!([-90.2, 23.7]));

    let before = get(&h, "/api/storms?time=2026-10-01T00:00:00Z").await.json();
    assert_eq!(before["features"], json!([]));
    assert_eq!(get(&h, "/api/storms?time=nope").await.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn storm_detail() {
    let h = harness();
    let r = get(&h, "/api/storms/AL092026?time=2026-10-08T18:00:00Z").await;
    assert_eq!(r.status, StatusCode::OK);
    let body = r.json();
    assert_eq!(body["storm"]["name"], "Isaias");
    assert_eq!(body["category"], "1");
    assert_eq!(body["geometry_source"], "mapserver");
    assert_eq!(get(&h, "/api/storms/zz992026?time=2026-10-08T18:00:00Z").await.status, StatusCode::NOT_FOUND);
    assert_eq!(get(&h, "/api/storms/..%2F..?time=2026-10-08T18:00:00Z").await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn config_status_and_frontend() {
    let h = harness();
    assert_eq!(get(&h, "/api/config").await.json()["basemap_style"], "https://example.test/style.json");
    let status = get(&h, "/api/status").await;
    assert_eq!(status.status, StatusCode::OK);
    assert!(status.json()["sources"].is_object());
    let index = get(&h, "/").await;
    assert_eq!(index.status, StatusCode::OK);
    assert_eq!(index.body, b"hello radar");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --test server`
Expected: compile error `could not find server in rust_radar`.

- [ ] **Step 3: Implement `src/server.rs`**

```rust
//! HTTP API, radar tiles and the static frontend.

use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::{Path as UrlPath, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, SecondsFormat, Utc};
use lru::LruCache;
use serde::Deserialize;
use serde_json::{json, Value};
use tower_http::services::ServeDir;

use crate::mrms_archive::MrmsArchive;
use crate::nhc_archive::NhcArchive;
use crate::sources::nhc::category;
use crate::status::Status;
use crate::tiles;

const TILE_CACHE_TILES: usize = 2048;
type TileKey = (i64, u8, u32, u32);

pub struct AppState {
    pub mrms: Arc<MrmsArchive>,
    pub nhc: Arc<NhcArchive>,
    pub status: Status,
    pub basemap_style: String,
    tile_cache: Mutex<LruCache<TileKey, Bytes>>,
}

impl AppState {
    pub fn new(mrms: Arc<MrmsArchive>, nhc: Arc<NhcArchive>, status: Status, basemap_style: String) -> AppState {
        let capacity = NonZeroUsize::new(TILE_CACHE_TILES).expect("non-zero");
        AppState { mrms, nhc, status, basemap_style, tile_cache: Mutex::new(LruCache::new(capacity)) }
    }
}

pub fn router(state: Arc<AppState>, web_dir: &Path) -> Router {
    Router::new()
        .route("/api/config", get(get_config))
        .route("/api/frames", get(get_frames))
        .route("/api/status", get(get_status))
        .route("/api/storms", get(get_storms))
        .route("/api/storms/{id}", get(get_storm))
        .route("/tiles/{time}/{z}/{x}/{y}", get(get_tile))
        .fallback_service(ServeDir::new(web_dir))
        .with_state(state)
}

#[derive(Debug)]
pub enum ApiError {
    BadRequest(String),
    NotFound(String),
    Internal(anyhow::Error),
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError::Internal(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (code, message) = match self {
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            ApiError::NotFound(m) => (StatusCode::NOT_FOUND, m),
            ApiError::Internal(e) => {
                tracing::error!("{e:#}");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal error".to_string())
            }
        };
        (code, Json(json!({ "error": message }))).into_response()
    }
}

fn rfc3339(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn parse_time(s: &str) -> Result<DateTime<Utc>, ApiError> {
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|_| ApiError::BadRequest(format!("invalid time {s:?}; use RFC 3339, e.g. 2026-10-08T17:00:00Z")))
}

#[derive(Deserialize)]
struct TimeQuery {
    time: Option<String>,
}

impl TimeQuery {
    fn resolve(&self) -> Result<DateTime<Utc>, ApiError> {
        self.time.as_deref().map_or_else(|| Ok(Utc::now()), parse_time)
    }
}

async fn get_config(State(st): State<Arc<AppState>>) -> Json<Value> {
    Json(json!({ "basemap_style": st.basemap_style }))
}

async fn get_frames(State(st): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let frames = st.mrms.frames()?;
    let latest = frames.last().copied().map(rfc3339);
    let frames: Vec<String> = frames.into_iter().map(rfc3339).collect();
    Ok(Json(json!({ "frames": frames, "latest": latest })))
}

async fn get_status(State(st): State<Arc<AppState>>) -> Json<Value> {
    Json(json!({ "now": rfc3339(Utc::now()), "sources": st.status.snapshot() }))
}

async fn get_storms(State(st): State<Arc<AppState>>, Query(q): Query<TimeQuery>) -> Result<Json<Value>, ApiError> {
    let t = q.resolve()?;
    let mut features = Vec::new();
    for snapshot in st.nhc.at_time(t)? {
        let s = &snapshot.storm;
        features.push(json!({
            "type": "Feature",
            "geometry": { "type": "Point", "coordinates": [s.lon, s.lat] },
            "properties": {
                "storm_id": s.id,
                "layer": "center",
                "name": s.name,
                "classification": s.classification,
                "category": category(s.intensity_kt),
                "wind_kt": s.intensity_kt,
                "pressure_mb": s.pressure_mb,
                "advisory_num": s.advisory_num,
                "issuance": rfc3339(s.issuance),
            }
        }));
        features.extend(snapshot.features);
    }
    Ok(Json(json!({ "type": "FeatureCollection", "features": features })))
}

async fn get_storm(
    State(st): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    Query(q): Query<TimeQuery>,
) -> Result<Json<Value>, ApiError> {
    let t = q.resolve()?;
    let snapshot = st
        .nhc
        .get(&id.to_lowercase(), t)?
        .ok_or_else(|| ApiError::NotFound(format!("no advisory for storm {id:?} at {}", rfc3339(t))))?;
    let category = category(snapshot.storm.intensity_kt);
    Ok(Json(json!({ "storm": snapshot.storm, "category": category, "geometry_source": snapshot.geometry_source })))
}

async fn get_tile(
    State(st): State<Arc<AppState>>,
    UrlPath((time, z, x, y)): UrlPath<(String, u8, u32, String)>,
) -> Result<Response, ApiError> {
    let y: u32 = y
        .strip_suffix(".png")
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| ApiError::BadRequest(format!("bad tile row {y:?}; expected e.g. 12.png")))?;
    if !tiles::valid_tile(z, x, y) {
        return Err(ApiError::BadRequest(format!("tile {z}/{x}/{y} is out of range (max zoom {})", tiles::MAX_ZOOM)));
    }
    let is_latest = time == "latest";
    let t = if is_latest {
        st.mrms.latest()?.ok_or_else(|| ApiError::NotFound("no radar frames yet".into()))?
    } else {
        let t = parse_time(&time)?;
        if !st.mrms.contains(t) {
            return Err(ApiError::NotFound(format!("no radar frame at {}", rfc3339(t))));
        }
        t
    };

    let key = (t.timestamp(), z, x, y);
    let cached = st.tile_cache.lock().unwrap().get(&key).cloned();
    let png = match cached {
        Some(png) => png,
        None => {
            let state = Arc::clone(&st);
            let rendered = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
                let grid = state.mrms.load(t)?;
                Ok(tiles::render_tile(&grid, z, x, y))
            })
            .await
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("tile render task failed: {e}")))??;
            let png = Bytes::from(rendered);
            st.tile_cache.lock().unwrap().put(key, png.clone());
            png
        }
    };
    let cache_control = if is_latest { "no-cache" } else { "public, max-age=86400" };
    Ok(([(header::CONTENT_TYPE, "image/png"), (header::CACHE_CONTROL, cache_control)], png).into_response())
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --test server`
Expected: 7 passed.

- [ ] **Step 5: Run the whole suite**

Run: `cargo test`
Expected: every test passes, with no warnings from `cargo build`.

- [ ] **Step 6: Commit**

```bash
git add src/lib.rs src/server.rs tests/server.rs
git commit -m "feat: HTTP API, radar tile server and static frontend hosting" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 13: Wire up `main`, example config and README

**Files:**
- Modify: `src/main.rs` (replace entirely)
- Create: `rust-radar.toml`, `README.md`

**Interfaces:**
- Consumes:
  - `Config::load`, `Cli`
  - `MrmsArchive::open`, `NhcArchive::open`, `HttpFetcher::new`
  - `sources::mrms::{run, backfill}`, `sources::nhc::run`
  - `server::{AppState, router}`
  - `prune` on both archives

- [ ] **Step 1: Replace `src/main.rs`**

```rust
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use chrono::Utc;
use clap::Parser;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use rust_radar::config::{Cli, Config};
use rust_radar::fetch::{Fetcher, HttpFetcher};
use rust_radar::mrms_archive::MrmsArchive;
use rust_radar::nhc_archive::NhcArchive;
use rust_radar::server::{router, AppState};
use rust_radar::sources;
use rust_radar::status::Status;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();
    let config = Config::load(&Cli::parse())?;

    let mrms = Arc::new(MrmsArchive::open(config.archive_dir.join("mrms"), config.decoded_cache_frames)?);
    let nhc = Arc::new(NhcArchive::open(config.archive_dir.join("nhc"))?);
    let status = Status::default();
    let fetcher: Arc<dyn Fetcher> = Arc::new(HttpFetcher::new()?);

    tokio::spawn(sources::mrms::run(
        Arc::clone(&fetcher),
        Arc::clone(&mrms),
        status.clone(),
        Duration::from_secs(config.mrms_poll_secs),
    ));
    tokio::spawn(sources::nhc::run(
        Arc::clone(&fetcher),
        Arc::clone(&nhc),
        status.clone(),
        Duration::from_secs(config.nhc_poll_secs),
    ));
    {
        let (fetcher, mrms, hours) = (Arc::clone(&fetcher), Arc::clone(&mrms), config.backfill_hours);
        tokio::spawn(async move {
            match sources::mrms::backfill(fetcher.as_ref(), &mrms, Utc::now(), hours).await {
                Ok(n) => info!("backfilled {n} radar frames"),
                Err(e) => warn!("radar backfill failed: {e:#}"),
            }
        });
    }
    tokio::spawn(prune_loop(Arc::clone(&mrms), Arc::clone(&nhc), config.retention_hours));

    let state = Arc::new(AppState::new(mrms, nhc, status, config.basemap_style.clone()));
    let app = router(state, &config.web_dir);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", config.port))
        .await
        .with_context(|| format!("binding 127.0.0.1:{}", config.port))?;
    info!("rust-radar listening on http://127.0.0.1:{}", config.port);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn prune_loop(mrms: Arc<MrmsArchive>, nhc: Arc<NhcArchive>, retention_hours: u64) {
    let mut ticker = tokio::time::interval(Duration::from_secs(3600));
    loop {
        ticker.tick().await;
        let cutoff = Utc::now() - chrono::Duration::hours(retention_hours as i64);
        let (mrms, nhc) = (Arc::clone(&mrms), Arc::clone(&nhc));
        let result = tokio::task::spawn_blocking(move || -> anyhow::Result<(usize, usize)> {
            Ok((mrms.prune(cutoff)?, nhc.prune(cutoff)?))
        })
        .await;
        match result {
            Ok(Ok((frames, advisories))) if frames + advisories > 0 => {
                info!("pruned {frames} radar frames and {advisories} advisories")
            }
            Ok(Ok(_)) => {}
            Ok(Err(e)) => warn!("prune failed: {e:#}"),
            Err(e) => warn!("prune task panicked: {e}"),
        }
    }
}
```

- [ ] **Step 2: Create `rust-radar.toml` with the spec defaults**

```toml
# rust-radar configuration. Every key can also be passed as a CLI flag (e.g. --port 9000).
port = 8080
archive_dir = "./archive"
web_dir = "./web"
retention_hours = 72
backfill_hours = 6
mrms_poll_secs = 60
nhc_poll_secs = 300
decoded_cache_frames = 10
basemap_style = "https://tiles.openfreemap.org/styles/dark"
```

- [ ] **Step 3: Create `README.md`**

````markdown
# rust-radar

A local hurricane tracker. It pulls NOAA MRMS composite radar (about every
2 minutes) and National Hurricane Center storm data. It keeps a rolling 72-hour
archive and serves a zoomable, clickable map with live view and replay.

## Run

```bash
cargo run --release
# open http://127.0.0.1:8080
```

On first start the app fills in the last 6 hours of radar from NOAA's AWS
bucket. That takes a few minutes and uses about 200 MB. Configuration lives in
`rust-radar.toml`, and every key also works as a CLI flag (`--help`). Set
`RUST_LOG=debug` for verbose logs.

## Data sources
- **Radar:** MRMS `MergedReflectivityQCComposite_00.50` from `mrms.ncep.noaa.gov`.
  Backfill comes from `noaa-mrms-pds` on AWS. Coverage is the continental US and
  nearby waters.
- **Storms:** `nhc.noaa.gov/CurrentStorms.json`, plus the NHC GIS map service.
  The per-advisory shapefile zip is the fallback.

## Test

```bash
cargo test
```

Tests use the fixtures in `tests/fixtures/` and never touch the network.
````

- [ ] **Step 4: Build and run the full test suite**

Run: `cargo build --release && cargo test`
Expected: the release build succeeds and every test passes.

- [ ] **Step 5: Smoke-test against live NOAA data**

The frontend doesn't exist yet. Create a placeholder so `/` resolves:

```bash
mkdir -p web && echo '<!doctype html><title>rust-radar</title>' > web/index.html
cargo run --release -- --backfill-hours 0 &
sleep 90
curl -s http://127.0.0.1:8080/api/frames | head -c 300; echo
curl -s http://127.0.0.1:8080/api/status; echo
curl -s "http://127.0.0.1:8080/api/storms" | python3 -c 'import json,sys;d=json.load(sys.stdin);print({f["properties"]["layer"] for f in d["features"]})'
curl -s -o /tmp/claude-tile.png -w '%{http_code} %{content_type}\n' http://127.0.0.1:8080/tiles/latest/4/4/6.png
kill %1
```

Expected:
- `frames` has at least one entry.
- `status` shows `mrms` and `nhc` with `last_success` set.
- If any storm is active, `storms` lists layers such as `center`, `cone` and `forecast_track`.
- The tile request returns `200 image/png`.
- If NHC reports `geometry_source` `none` or `zip`, check the warning logs before moving on.

- [ ] **Step 6: Commit**

```bash
git add src/main.rs rust-radar.toml README.md
git commit -m "feat: wire pollers, backfill, pruning and server in main" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

The placeholder `web/index.html` is not committed. Task 14 replaces it.

---

### Task 14: Frontend map

**Files:**
- Create: `web/index.html`, `web/style.css`, `web/app.js` (replacing the Task 13 placeholder)

**Interfaces:**
- Consumes:
  - `/api/config`, `/api/frames`, `/api/status`, `/api/storms?time=`, `/api/storms/{id}?time=`
  - `/tiles/{time}/{z}/{x}/{y}.png`
  - Storm feature properties from Task 12 (`layer`, `storm_id`, `category`, `wind_kt`, `radii`, `name`)

- [ ] **Step 1: Write `web/index.html`**

```html
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>rust-radar</title>
  <link rel="stylesheet" href="https://unpkg.com/maplibre-gl@4.7.1/dist/maplibre-gl.css">
  <link rel="stylesheet" href="style.css">
</head>
<body>
  <div id="map"></div>
  <div id="status" class="badge">Loading…</div>
  <aside id="panel" hidden>
    <button id="panel-close" aria-label="Close">×</button>
    <div id="panel-body"></div>
  </aside>
  <div id="controls">
    <button id="play" aria-label="Play or pause">▶</button>
    <input id="slider" type="range" min="0" max="0" value="0" aria-label="Radar time">
    <span id="time-label">–</span>
    <select id="speed" aria-label="Playback speed">
      <option value="1000">1×</option>
      <option value="500" selected>2×</option>
      <option value="250">4×</option>
    </select>
    <button id="live" class="active">Live</button>
    <label>Radar <input id="opacity" type="range" min="0" max="1" step="0.05" value="0.8"></label>
  </div>
  <script src="https://unpkg.com/maplibre-gl@4.7.1/dist/maplibre-gl.js"></script>
  <script src="app.js"></script>
</body>
</html>
```

- [ ] **Step 2: Write `web/style.css`**

```css
:root {
  color-scheme: dark;
  --panel: rgba(18, 20, 24, 0.88);
  --fg: #e8eaed;
  --muted: #9aa0a6;
  --accent: #4fc3f7;
  --warn: #ffb300;
}
* { box-sizing: border-box; }
html, body { margin: 0; height: 100%; background: #0b0d10; color: var(--fg); font: 14px/1.4 system-ui, sans-serif; }
#map { position: absolute; inset: 0; }
.badge { position: absolute; top: 12px; left: 12px; padding: 6px 10px; border-radius: 6px; background: var(--panel); font-variant-numeric: tabular-nums; }
.badge.stale { background: var(--warn); color: #1a1300; }
#controls { position: absolute; left: 12px; right: 12px; bottom: 24px; display: flex; flex-wrap: wrap; gap: 10px; align-items: center; padding: 8px 12px; border-radius: 8px; background: var(--panel); }
#slider { flex: 1 1 200px; }
#time-label { min-width: 13em; font-variant-numeric: tabular-nums; }
button, select { background: #2a2e35; color: var(--fg); border: 1px solid #3c4049; border-radius: 4px; padding: 4px 10px; font: inherit; cursor: pointer; }
button.active { background: var(--accent); border-color: var(--accent); color: #001018; }
#panel { position: absolute; top: 56px; left: 12px; width: min(320px, calc(100% - 24px)); padding: 12px 16px; border-radius: 8px; background: var(--panel); }
#panel[hidden] { display: none; }
#panel-close { position: absolute; top: 8px; right: 8px; padding: 0 8px; }
#panel h2 { margin: 0 0 2px; font-size: 20px; }
.subtitle { margin: 0 0 10px; color: var(--muted); }
dl { display: grid; grid-template-columns: auto 1fr; gap: 4px 12px; margin: 0 0 10px; }
dt { color: var(--muted); }
dd { margin: 0; }
a { color: var(--accent); }
```

- [ ] **Step 3: Write `web/app.js`**

```js
// rust-radar frontend: MRMS radar tiles + NHC storms on a MapLibre map.
const RADAR_STALE_MIN = 10;
const NHC_STALE_MIN = 60;

const $ = (id) => document.getElementById(id);
const state = { map: null, frames: [], index: -1, live: true, timer: null, stormRequest: 0, status: null, stormCount: null };

const tileUrl = (time) => `/tiles/${encodeURIComponent(time)}/{z}/{x}/{y}.png`;
const currentTime = () => state.frames[state.index];
const isLayer = (name) => ['==', ['get', 'layer'], name];

async function getJson(url) {
  const resp = await fetch(url);
  if (!resp.ok) throw new Error(`${url}: HTTP ${resp.status}`);
  return resp.json();
}

const formatTime = (iso) =>
  new Date(iso).toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', timeZoneName: 'short' });

const CATEGORY_COLORS = ['match', ['get', 'category'],
  'TD', '#5ebaff', 'TS', '#00faf4', '1', '#ffffcc', '2', '#ffe775', '3', '#ffc140', '4', '#ff8f20', '5', '#ff6060', '#cccccc'];
const WIND_COLORS = ['step', ['coalesce', ['get', 'wind_kt'], 0],
  '#5ebaff', 34, '#00faf4', 64, '#ffffcc', 83, '#ffe775', 96, '#ffc140', 113, '#ff8f20', 137, '#ff6060'];
const CLASS_NAMES = {
  TD: 'Tropical Depression', TS: 'Tropical Storm', HU: 'Hurricane', STD: 'Subtropical Depression',
  STS: 'Subtropical Storm', PTC: 'Post-Tropical Cyclone', PC: 'Potential Tropical Cyclone',
};

function addLayers(map) {
  const layers = map.getStyle().layers;
  const firstSymbol = layers.find((l) => l.type === 'symbol')?.id;
  const font = layers.find((l) => Array.isArray(l.layout?.['text-font']))?.layout['text-font'];

  map.addSource('radar', { type: 'raster', tiles: [tileUrl('latest')], tileSize: 256, maxzoom: 10 });
  map.addLayer({ id: 'radar', type: 'raster', source: 'radar',
    paint: { 'raster-opacity': Number($('opacity').value), 'raster-fade-duration': 0 } }, firstSymbol);

  map.addSource('storms', { type: 'geojson', data: { type: 'FeatureCollection', features: [] } });
  map.addLayer({ id: 'storm-cone', type: 'fill', source: 'storms', filter: isLayer('cone'),
    paint: { 'fill-color': '#ffffff', 'fill-opacity': 0.12 } });
  map.addLayer({ id: 'storm-cone-outline', type: 'line', source: 'storms', filter: isLayer('cone'),
    paint: { 'line-color': '#ffffff', 'line-width': 1.5 } });
  map.addLayer({ id: 'storm-wind-radii', type: 'line', source: 'storms', filter: isLayer('wind_radii'),
    paint: { 'line-color': ['match', ['get', 'radii'], 34, '#ffd54f', 50, '#ff9800', 64, '#f44336', '#ffffff'], 'line-width': 1 } });
  map.addLayer({ id: 'storm-past-track', type: 'line', source: 'storms', filter: isLayer('past_track'),
    paint: { 'line-color': '#bbbbbb', 'line-width': 2 } });
  map.addLayer({ id: 'storm-forecast-track', type: 'line', source: 'storms', filter: isLayer('forecast_track'),
    paint: { 'line-color': '#ffffff', 'line-width': 2, 'line-dasharray': [2, 2] } });
  map.addLayer({ id: 'storm-points', type: 'circle', source: 'storms',
    filter: ['any', isLayer('forecast_points'), isLayer('past_points')],
    paint: { 'circle-radius': 4, 'circle-color': WIND_COLORS, 'circle-stroke-color': '#000', 'circle-stroke-width': 1 } });
  map.addLayer({ id: 'storm-center', type: 'circle', source: 'storms', filter: isLayer('center'),
    paint: { 'circle-radius': 9, 'circle-color': CATEGORY_COLORS, 'circle-stroke-color': '#000', 'circle-stroke-width': 2 } });
  map.addLayer({ id: 'storm-label', type: 'symbol', source: 'storms', filter: isLayer('center'),
    layout: { 'text-field': ['get', 'name'], 'text-offset': [0, 1.4], 'text-size': 13, ...(font ? { 'text-font': font } : {}) },
    paint: { 'text-color': '#ffffff', 'text-halo-color': '#000000', 'text-halo-width': 1.5 } });
}

async function refreshFrames() {
  const previous = currentTime();
  const data = await getJson('/api/frames');
  state.frames = data.frames;
  $('slider').max = Math.max(0, state.frames.length - 1);
  if (state.frames.length === 0) {
    $('time-label').textContent = 'Waiting for the first radar frame…';
    return;
  }
  if (state.live || !previous) {
    showFrame(state.frames.length - 1);
  } else {
    // Old frames may have been pruned; keep showing the same moment if it still exists.
    state.index = Math.max(0, state.frames.indexOf(previous));
    $('slider').value = state.index;
  }
}

function showFrame(i) {
  state.index = i;
  const time = currentTime();
  state.map.getSource('radar').setTiles([tileUrl(time)]);
  $('slider').value = i;
  $('time-label').textContent = formatTime(time);
  loadStorms(time);
  renderStatus();
}

async function loadStorms(time) {
  const request = ++state.stormRequest;
  try {
    const data = await getJson(`/api/storms?time=${encodeURIComponent(time)}`);
    if (request !== state.stormRequest) return;
    state.map.getSource('storms').setData(data);
    state.stormCount = data.features.filter((f) => f.properties.layer === 'center').length;
    renderStatus();
  } catch (err) {
    console.warn(err);
  }
}

function setLive(live) {
  state.live = live;
  $('live').classList.toggle('active', live);
}

function stopPlayback() {
  clearInterval(state.timer);
  state.timer = null;
  $('play').textContent = '▶';
}

function startPlayback() {
  if (state.frames.length < 2) return;
  setLive(false);
  state.timer = setInterval(() => showFrame((state.index + 1) % state.frames.length), Number($('speed').value));
  $('play').textContent = '⏸';
}

function goLive() {
  stopPlayback();
  setLive(true);
  if (state.frames.length) showFrame(state.frames.length - 1);
}

async function refreshStatus() {
  try {
    state.status = await getJson('/api/status');
  } catch (err) {
    state.status = null;
  }
  renderStatus();
}

const ageMinutes = (iso) => (iso ? (Date.now() - new Date(iso).getTime()) / 60000 : Infinity);

function describeAge(minutes) {
  if (!Number.isFinite(minutes)) return 'none';
  if (minutes < 1) return '<1m';
  if (minutes < 120) return `${Math.round(minutes)}m`;
  return `${Math.round(minutes / 60)}h`;
}

function renderStatus() {
  const sources = state.status?.sources ?? {};
  const radarAge = ageMinutes(state.frames[state.frames.length - 1]);
  const nhcAge = ageMinutes(sources.nhc?.last_success);
  const badge = $('status');
  badge.textContent = `Radar ${describeAge(radarAge)} · NHC ${describeAge(nhcAge)}`;
  if (state.stormCount === 0) badge.textContent += ' · No active tropical cyclones';
  badge.classList.toggle('stale', radarAge > RADAR_STALE_MIN || nhcAge > NHC_STALE_MIN);
  badge.title = [sources.mrms?.last_error, sources.nhc?.last_error].filter(Boolean).join('\n');
}

function compass(degrees) {
  const points = ['N', 'NNE', 'NE', 'ENE', 'E', 'ESE', 'SE', 'SSE', 'S', 'SSW', 'SW', 'WSW', 'W', 'WNW', 'NW', 'NNW'];
  return points[Math.round((degrees % 360) / 22.5) % 16];
}

async function openPanel(stormId) {
  const time = currentTime();
  const query = time ? `?time=${encodeURIComponent(time)}` : '';
  let data;
  try {
    data = await getJson(`/api/storms/${encodeURIComponent(stormId)}${query}`);
  } catch (err) {
    console.warn(err);
    return;
  }
  const s = data.storm;
  const body = $('panel-body');
  body.replaceChildren();

  const title = document.createElement('h2');
  title.textContent = s.name;
  const subtitle = document.createElement('p');
  subtitle.className = 'subtitle';
  const strength = /^\d$/.test(data.category) ? `Category ${data.category}` : data.category;
  subtitle.textContent = `${CLASS_NAMES[s.classification] ?? s.classification} · ${strength}`;

  const rows = [
    ['Max wind', `${s.intensity_kt} kt (${Math.round(s.intensity_kt * 1.15078)} mph)`],
    ['Pressure', `${s.pressure_mb} mb`],
    ['Movement', `${compass(s.movement_dir)} at ${s.movement_speed_mph} mph`],
    ['Position', `${s.lat.toFixed(1)}°, ${s.lon.toFixed(1)}°`],
    ['Advisory', `#${s.advisory_num || '?'} · ${formatTime(s.issuance)}`],
  ];
  if (data.geometry_source === 'none') rows.push(['Forecast cone', 'unavailable']);
  const list = document.createElement('dl');
  for (const [label, value] of rows) {
    const dt = document.createElement('dt');
    dt.textContent = label;
    const dd = document.createElement('dd');
    dd.textContent = value;
    list.append(dt, dd);
  }
  body.append(title, subtitle, list);

  if (s.public_advisory_url) {
    const link = document.createElement('a');
    link.href = s.public_advisory_url;
    link.target = '_blank';
    link.rel = 'noopener';
    link.textContent = 'Read the public advisory ↗';
    body.append(link);
  }
  $('panel').hidden = false;
}

async function init() {
  const config = await getJson('/api/config');
  const map = new maplibregl.Map({ container: 'map', style: config.basemap_style, center: [-85, 27], zoom: 4 });
  state.map = map;
  map.addControl(new maplibregl.NavigationControl(), 'top-right');

  map.on('load', async () => {
    addLayers(map);
    for (const layer of ['storm-center', 'storm-cone']) {
      map.on('click', layer, (e) => openPanel(e.features[0].properties.storm_id));
      map.on('mouseenter', layer, () => { map.getCanvas().style.cursor = 'pointer'; });
      map.on('mouseleave', layer, () => { map.getCanvas().style.cursor = ''; });
    }
    await Promise.all([refreshFrames(), refreshStatus()]);
    setInterval(() => refreshFrames().catch(console.warn), 60_000);
    setInterval(refreshStatus, 30_000);
  });

  $('play').addEventListener('click', () => (state.timer ? stopPlayback() : startPlayback()));
  $('live').addEventListener('click', goLive);
  $('slider').addEventListener('input', (e) => {
    stopPlayback();
    setLive(false);
    showFrame(Number(e.target.value));
  });
  $('speed').addEventListener('change', () => {
    if (state.timer) { stopPlayback(); startPlayback(); }
  });
  $('opacity').addEventListener('input', (e) => {
    if (map.getLayer('radar')) map.setPaintProperty('radar', 'raster-opacity', Number(e.target.value));
  });
  $('panel-close').addEventListener('click', () => { $('panel').hidden = true; });
}

init().catch((err) => {
  $('status').textContent = `Failed to start: ${err.message}`;
  $('status').classList.add('stale');
});
```

- [ ] **Step 4: Syntax-check the script**

Run: `node --check web/app.js`
Expected: no output (exit code 0).

- [ ] **Step 5: Verify in a browser against live data**

Run: `cargo run --release`, then open `http://127.0.0.1:8080`. Check each item:
- A dark basemap appears. Radar colours appear over any area with precipitation, under the place labels.
- The status badge reads `Radar Nm · NHC Nm` with small numbers, and is not amber. When no storms are active at the selected time, it adds ` · No active tropical cyclones`.
- If any storm is active:
  - A coloured circle with the storm name appears.
  - Its cone, forecast track and wind radii are drawn.
  - Clicking the circle or the cone opens the side panel with wind, pressure, movement and an advisory link.
- Dragging the slider changes the time label and the radar. **Live** turns off. Clicking **Live** jumps back to the newest frame.
- **▶** loops through the frames. The speed menu changes the pace. The radar opacity slider fades the radar.
- No errors appear in the browser console. One exception is acceptable: a missing-glyph warning from the basemap style.

Fix anything that fails before committing.

- [ ] **Step 6: Commit**

```bash
git add web/
git commit -m "feat: MapLibre frontend with radar, storms, replay and status" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

## Spec Coverage Check

| Spec requirement | Task |
|---|---|
| MRMS live poll (60 s) | 7, 13 |
| Fallback and backfill from S3 | 7, 13 |
| Temp-then-rename; validate before archiving | 5 |
| NHC CurrentStorms poll (5 min) | 8, 11, 13 |
| Map-service GeoJSON with layer IDs looked up by name | 8, 11 |
| Advisory-zip fallback; centre-only when both fail | 10, 11 |
| Per-advisory snapshots for replay | 9 |
| Retention 72 h; hourly prune | 5, 9, 13 |
| LRU of decoded frames | 5 |
| Tile cache | 12 |
| Nearest-neighbour tiles with the NWS palette | 2, 3, 4 |
| `-999`/`-99` transparent | 2, 3 |
| API: frames, tiles (`latest` or time), storms, storm detail, status | 12 |
| Frontend: basemap, radar opacity, cones, tracks, wind radii, click panel | 14 |
| Frontend: slider, play/loop/speed, Live, status badge thresholds | 14 |
| Config file plus CLI overrides | 1, 13 |
| Retry with capped backoff; pollers never crash | 6, 7, 11 |
| "No active tropical cyclones" | 12 (empty FeatureCollection), 14 (badge text) |
| Tests: unit, decode fixture, integration, fake fetcher | 1–12 |
