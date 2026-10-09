# rust-radar Desktop App Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Linux desktop app (Tauri 2) that runs the rust-radar server in-process on a free local port and shows it in its own window, with no droplet.

**Architecture:** `main.rs`'s startup moves into a library function `rust_radar::app::start`. A new workspace member `desktop/` (Tauri 2) loads desktop settings, starts that function on port 0 inside Tauri's tokio runtime, and opens a window at `http://127.0.0.1:<port>/`. Closing the window exits the process (Tauri's default when the last window closes).

**Tech Stack:** Rust, axum 0.8, tokio, Tauri 2 (WebKitGTK 4.1 on Linux), tauri-plugin-single-instance 2, tauri-plugin-dialog 2, tracing-subscriber, toml 0.8.

**Spec:** `docs/superpowers/specs/2026-10-09-desktop-app-design.md`

## Global Constraints

- Linux only. Do not add Windows/macOS-specific code, but do not hard-code `/home/...` paths either: base directories come from Tauri's path resolver (`data_dir()`, `config_dir()`), then `.join("rust-radar")`.
- Data and log: `~/.local/share/rust-radar/` and `~/.local/share/rust-radar/rust-radar.log`. Optional settings file: `~/.config/rust-radar/rust-radar.toml`.
- Desktop defaults: `port = 0`, `retention_hours = 72`, `backfill_hours = 2`, `decoded_cache_frames = 10`; everything else as `Config::default()`. `port` and `web_dir` are always set by the app, whatever the file says.
- Closing the window quits the app; no tray, no background collection.
- The droplet build must not change: `cargo build --release --locked --target x86_64-unknown-linux-musl` at the repo root builds only the server and never compiles Tauri. `src/main.rs` keeps its CLI and behaviour.
- `web/` is shared, not copied into the repo twice; the bundle includes it as a resource.
- Commits end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Never `pkill -f`/`pgrep -f` (it matches the calling shell). Track processes by captured `$!` PIDs.

## Review Focus

1. Archive directory not writable (or a file sits where it should be) → `start` returns an error naming the path, and the desktop shows a dialog instead of a blank window. Pinned in Task 2 (`start_fails_when_archive_dir_is_a_file`) and exercised manually in Task 4.
2. A typo in `~/.config/rust-radar/rust-radar.toml` → a clear error naming the file, not a silent fallback. Pinned in Task 3 (`unknown_key_is_an_error_naming_the_file`, `malformed_toml_is_an_error_naming_the_file`).
3. Settings file sets `port` or `web_dir` → ignored, so the app still starts on a free port with its bundled page. Pinned in Task 3 (`port_and_web_dir_are_always_the_apps`).
4. A log file grown past 10 MB → emptied at the next start; a small one is kept. Pinned in Task 4 (`open_log_truncates_only_when_over_limit`).
5. Launching a second copy → the first window is focused and the second exits, so two pollers never share one archive. Manual check in Task 5.

---

### Task 1: WebKitGTK feasibility check (throwaway spike — gate)

Nothing from this task is committed. Its output is a pass/fail answer.

**Files (scratchpad only, `$SP` = the session scratchpad directory):**
- Create: `$SP/webkit-spike/Cargo.toml`, `build.rs`, `tauri.conf.json`, `icons/icon.png`, `src/main.rs`
- Create: `$SP/webkit-spike/report.py`, `$SP/spike.toml`

- [ ] **Step 1: Install the Tauri CLI (one time; also used in Task 5)**

Run: `cargo install tauri-cli --version '^2' --locked`
Expected: `cargo tauri --version` prints `tauri-cli 2.x`.

- [ ] **Step 2: Start a local rust-radar server with some archive**

```bash
cat > $SP/spike.toml <<EOF
port = 8099
archive_dir = "$SP/archive"
web_dir = "/home/zerosum/workspace/rust-radar/web"
backfill_hours = 1
decoded_cache_frames = 10
EOF
cargo build --release
./target/release/rust-radar --config $SP/spike.toml > $SP/spike-server.log 2>&1 & SPID=$!
```
Expected: `curl -s 127.0.0.1:8099/api/frames | head -c 200` returns JSON with frame times.

- [ ] **Step 3: A results listener**

`$SP/webkit-spike/report.py`:
```python
import http.server, sys
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        print(self.path, flush=True)
        self.send_response(204); self.send_header('Access-Control-Allow-Origin', '*'); self.end_headers()
    def log_message(self, *a): pass
http.server.HTTPServer(('127.0.0.1', 9911), H).serve_forever()
```
Run: `python3 -I $SP/webkit-spike/report.py > $SP/spike-report.log & RPID=$!`

- [ ] **Step 4: Bare Tauri window pointed at the server, with a measuring script**

`Cargo.toml`:
```toml
[package]
name = "webkit-spike"
version = "0.0.0"
edition = "2021"
[build-dependencies]
tauri-build = "2"
[dependencies]
tauri = "2"
```
`build.rs`: `fn main() { tauri_build::build() }`

`tauri.conf.json`:
```json
{ "productName": "webkit-spike", "version": "0.0.0", "identifier": "test.webkit-spike",
  "app": { "windows": [], "security": { "csp": null } },
  "bundle": { "icon": ["icons/icon.png"] } }
```
`icons/icon.png`: `python3 -c "from PIL import Image; Image.new('RGBA',(512,512),(30,60,90,255)).save('$SP/webkit-spike/icons/icon.png')"`

`src/main.rs`:
```rust
use tauri::{WebviewUrl, WebviewWindowBuilder};
const MEASURE: &str = r#"
window.addEventListener('load', () => setTimeout(async () => {
  const r = (q) => fetch('http://127.0.0.1:9911/' + q).catch(() => {});
  const gl = document.createElement('canvas').getContext('webgl');
  const dbg = gl && gl.getExtension('WEBGL_debug_renderer_info');
  r('renderer=' + encodeURIComponent(dbg ? gl.getParameter(dbg.UNMASKED_RENDERER_WEBGL) : 'none'));
  for (const sat of ['off', 'infrared']) {
    $('satellite').value = sat; $('satellite').dispatchEvent(new Event('change'));
    await new Promise((s) => setTimeout(s, 8000));
    $('speed').value = '100'; showFrame(0); $('play').click();
    let steps = 0, last = state.index; const t0 = Date.now();
    while (Date.now() - t0 < 10000) { if (state.index !== last) { steps++; last = state.index; } await new Promise((s) => setTimeout(s, 50)); }
    $('play').click();
    r(`sat=${sat}&steps=${steps}`);
  }
  r('done');
}, 6000));
"#;
fn main() {
    tauri::Builder::default()
        .setup(|app| {
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External("http://127.0.0.1:8099/".parse().unwrap()))
                .title("webkit-spike").inner_size(1280.0, 800.0)
                .initialization_script(MEASURE)
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("run");
}
```
(Steps = frame changes seen in 10 s. The comparison that matters is with the headless Chrome result from the playback-speed work: ~32 frame changes in 10 s with satellite off.)

- [ ] **Step 5: Run it, measure, screenshot**

```bash
cd $SP/webkit-spike && cargo build 2>&1 | tail -2
./target/debug/webkit-spike & WPID=$!
# wait for 'done' in the report (up to 90 s), then screenshot the X11 desktop
for i in $(seq 90); do grep -q done $SP/spike-report.log && break; sleep 1; done
import -window root $SP/spike-screen.png
cat $SP/spike-report.log
kill $WPID $RPID $SPID
```
Expected: the report shows a renderer line and two `sat=` lines. Read `$SP/spike-screen.png`.

- [ ] **Step 6: Decide (gate)**

PASS if all hold: the renderer is a hardware GPU (contains `AMD`/`Radeon`, not `llvmpipe`/`SwiftShader`/`none`); the screenshot shows the basemap, radar and controls drawn correctly; `sat=off` steps ≥ 20. Otherwise STOP: report the numbers and screenshot to the user and propose approach B (Chrome app window) — do not continue to Task 2.

On PASS, tell the user the numbers and that the window feel is theirs to judge after Task 5; continue.

---

### Task 2: `rust_radar::app::start` — shared startup

**Files:**
- Create: `src/app.rs`
- Modify: `src/lib.rs` (add `pub mod app;`)
- Modify: `src/main.rs` (whole file — becomes a thin wrapper)
- Test: `tests/app_start.rs`

**Interfaces:**
- Consumes: `Config` (`src/config.rs`), `Fetcher`/`FakeFetcher` (`src/fetch.rs`), `MrmsArchive::open`, `NhcArchive::open`, `sources::mrms::{run, backfill}`, `sources::nhc::run`, `server::{router, AppState}`, `Status`.
- Produces:
  ```rust
  pub struct Running { pub port: u16, pub server: tokio::task::JoinHandle<std::io::Result<()>> }
  pub async fn start(config: &Config, fetcher: Arc<dyn Fetcher>) -> anyhow::Result<Running>
  ```
  Must be called inside a tokio runtime. `config.port == 0` → OS-chosen port, reported in `Running.port`.

- [ ] **Step 1: Write the failing tests**

`tests/app_start.rs`:
```rust
use std::path::PathBuf;
use std::sync::Arc;

use rust_radar::app;
use rust_radar::config::Config;
use rust_radar::fetch::FakeFetcher;

fn config(archive_dir: PathBuf) -> Config {
    Config {
        port: 0,
        archive_dir,
        web_dir: PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/web")),
        backfill_hours: 0,
        ..Config::default()
    }
}

#[tokio::test]
async fn starts_on_a_free_port_and_serves_api_and_page() {
    let dir = tempfile::tempdir().unwrap();
    let running = app::start(&config(dir.path().to_path_buf()), Arc::new(FakeFetcher::new())).await.unwrap();
    assert_ne!(running.port, 0);
    let base = format!("http://127.0.0.1:{}", running.port);
    let status = reqwest::get(format!("{base}/api/status")).await.unwrap();
    assert_eq!(status.status(), 200);
    let page = reqwest::get(format!("{base}/")).await.unwrap().text().await.unwrap();
    assert!(page.contains("<title>"), "index.html not served: {page:.200}");
    assert!(dir.path().join("mrms").is_dir() && dir.path().join("nhc").is_dir());
}

#[tokio::test]
async fn two_instances_get_different_ports() {
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let one = app::start(&config(a.path().to_path_buf()), Arc::new(FakeFetcher::new())).await.unwrap();
    let two = app::start(&config(b.path().to_path_buf()), Arc::new(FakeFetcher::new())).await.unwrap();
    assert_ne!(one.port, two.port);
}

#[tokio::test]
async fn start_fails_when_archive_dir_is_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("not-a-dir");
    std::fs::write(&file, b"x").unwrap();
    let err = app::start(&config(file.clone()), Arc::new(FakeFetcher::new())).await.err().expect("should fail");
    assert!(format!("{err:#}").contains("not-a-dir"), "error should name the path: {err:#}");
}
```
Check first that `index.html` has a `<title>`: `grep -c '<title>' web/index.html` → `1`. (`reqwest` is already a normal dependency, so tests can use it.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --test app_start 2>&1 | tail -5`
Expected: compile error `unresolved import rust_radar::app`.

- [ ] **Step 3: Implement `src/app.rs`**

Move the body of `main` (everything after logging init and config load) and `prune_loop` from `src/main.rs`:
```rust
//! Starting the whole app — archives, pollers, backfill, pruning and the HTTP server.
//! Shared by the `rust-radar` server binary and the desktop app.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use chrono::Utc;
use tracing::{info, warn};

use crate::config::Config;
use crate::fetch::Fetcher;
use crate::mrms_archive::MrmsArchive;
use crate::nhc_archive::NhcArchive;
use crate::server::{router, AppState};
use crate::sources;
use crate::status::Status;

pub struct Running {
    /// The port actually bound (the OS picks one when the config says 0).
    pub port: u16,
    pub server: tokio::task::JoinHandle<std::io::Result<()>>,
}

/// Opens the archives, starts collection and serves the map. Must run inside a tokio runtime.
pub async fn start(config: &Config, fetcher: Arc<dyn Fetcher>) -> anyhow::Result<Running> {
    let mrms = Arc::new(MrmsArchive::open(config.archive_dir.join("mrms"), config.decoded_cache_frames)?);
    let nhc = Arc::new(NhcArchive::open(config.archive_dir.join("nhc"))?);
    let status = Status::default();

    tokio::spawn(sources::mrms::run(
        Arc::clone(&fetcher),
        Arc::clone(&mrms),
        status.clone(),
        Duration::from_secs(config.mrms_poll_secs),
        chrono::Duration::hours(config.backfill_hours as i64),
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
    let port = listener.local_addr().context("reading the bound address")?.port();
    info!("rust-radar listening on http://127.0.0.1:{port}");
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    Ok(Running { port, server })
}

// prune_loop: moved verbatim from src/main.rs (same signature, same body).
```
Paste the existing `prune_loop` function body unchanged below. `MrmsArchive::open` already errors with `creating <path>` when the path is a file, which satisfies the third test.

Add `pub mod app;` to `src/lib.rs`.

`src/main.rs` becomes:
```rust
use std::sync::Arc;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use rust_radar::app;
use rust_radar::config::{Cli, Config};
use rust_radar::fetch::HttpFetcher;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();
    let config = Config::load(&Cli::parse())?;
    let running = app::start(&config, Arc::new(HttpFetcher::new()?)).await?;
    running.server.await??;
    Ok(())
}
```

- [ ] **Step 4: Run to verify pass, plus the full suite**

Run: `cargo test --test app_start 2>&1 | tail -3` → Expected: `3 passed`.
Run: `cargo test 2>&1 | grep -E '^test result' ` → Expected: every line `ok`, total 81 passed (78 + 3).
Run: `cargo build --release --locked --target x86_64-unknown-linux-musl 2>&1 | tail -1` with `CC_x86_64_unknown_linux_musl=gcc` → Expected: `Finished`.

- [ ] **Step 5: Smoke-test the server binary unchanged**

```bash
./target/release/rust-radar --config $SP/spike.toml > $SP/t2.log 2>&1 & SPID=$!
sleep 3; curl -s -o /dev/null -w '%{http_code}\n' 127.0.0.1:8099/api/status; kill $SPID
```
Expected: `200`, and `$SP/t2.log` contains `listening on http://127.0.0.1:8099`.

- [ ] **Step 6: Commit**

```bash
git add src/app.rs src/lib.rs src/main.rs tests/app_start.rs
git commit -m "refactor: move startup into rust_radar::app::start so the desktop app can reuse it

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Desktop crate scaffold and desktop settings

**Files:**
- Modify: `Cargo.toml` (root — add `[workspace]`)
- Create: `desktop/Cargo.toml`, `desktop/build.rs`, `desktop/tauri.conf.json`
- Create: `desktop/icons/32x32.png`, `128x128.png`, `128x128@2x.png`, `icon.png` (generated)
- Create: `desktop/src/main.rs` (minimal for now), `desktop/src/settings.rs`
- Modify: `.gitignore` (nothing to add — `/target` is the workspace target; verify)

**Interfaces:**
- Consumes: `rust_radar::config::Config` (fields `port`, `archive_dir`, `web_dir`, `retention_hours`, `backfill_hours`, `decoded_cache_frames`; `validate()`; `Deserialize` with `deny_unknown_fields`).
- Produces: `pub fn settings::load(config_file: &Path, data_dir: &Path, web_dir: &Path) -> anyhow::Result<Config>` — desktop defaults, overlaid by the file if present, then `port = 0` and `web_dir` forced, then validated.

- [ ] **Step 1: Workspace**

Append to root `Cargo.toml`:
```toml
[workspace]
members = ["desktop"]
# `cargo build` / `cargo test` at the root (and so deploy.sh) only touch the server.
default-members = ["."]
```

- [ ] **Step 2: Desktop crate files**

`desktop/Cargo.toml`:
```toml
[package]
name = "rust-radar-desktop"
version = "0.1.0"
edition = "2021"

[build-dependencies]
tauri-build = { version = "2", features = [] }

[dependencies]
anyhow = "1"
rust-radar = { path = ".." }
tauri = { version = "2", features = [] }
tauri-plugin-dialog = "2"
tauri-plugin-single-instance = "2"
toml = "0.8"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

[dev-dependencies]
tempfile = "3"
```
`desktop/build.rs`: `fn main() { tauri_build::build() }`

`desktop/tauri.conf.json`:
```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "rust-radar",
  "version": "0.1.0",
  "identifier": "ink.bitspace.rust-radar",
  "build": { "frontendDist": "../web" },
  "app": { "windows": [], "security": { "csp": null } },
  "bundle": {
    "active": true,
    "targets": ["deb", "appimage"],
    "category": "Weather",
    "shortDescription": "Live US radar and hurricane tracker",
    "icon": ["icons/32x32.png", "icons/128x128.png", "icons/128x128@2x.png", "icons/icon.png"],
    "resources": { "../web/": "web/" }
  }
}
```
Icons (radar sweep: dark disc, green rings, a bright wedge), generated once, outside the repo's code:
```bash
python3 -I - <<'EOF'
from PIL import Image, ImageDraw
def icon(n):
    im = Image.new('RGBA', (n, n), (0, 0, 0, 0)); d = ImageDraw.Draw(im)
    d.ellipse([0, 0, n - 1, n - 1], fill=(14, 26, 38, 255))
    for f in (0.32, 0.62, 0.92):
        r = n * f / 2; c = n / 2
        d.ellipse([c - r, c - r, c + r, c + r], outline=(46, 160, 90, 255), width=max(1, n // 48))
    d.pieslice([n * 0.04, n * 0.04, n * 0.96, n * 0.96], 290, 340, fill=(80, 230, 120, 200))
    return im
for name, n in [('32x32', 32), ('128x128', 128), ('128x128@2x', 256), ('icon', 512)]:
    icon(n).save(f'desktop/icons/{name}.png')
EOF
```

`desktop/src/main.rs` (minimal so the crate builds; Task 4 replaces it):
```rust
mod settings;

fn main() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("running the Tauri app");
}
```

- [ ] **Step 3: Write the failing settings tests**

`desktop/src/settings.rs` — tests first, with `load` as `todo!()`:
```rust
//! Desktop settings: built-in defaults, optionally overridden by ~/.config/rust-radar/rust-radar.toml.

use std::path::Path;

use rust_radar::config::Config;

pub fn load(config_file: &Path, data_dir: &Path, web_dir: &Path) -> anyhow::Result<Config> {
    let _ = (config_file, data_dir, web_dir);
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct Dirs { _tmp: tempfile::TempDir, file: PathBuf, data: PathBuf, web: PathBuf }

    fn dirs(contents: Option<&str>) -> Dirs {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("rust-radar.toml");
        if let Some(c) = contents { std::fs::write(&file, c).unwrap(); }
        Dirs { data: tmp.path().join("data"), web: tmp.path().join("web"), file, _tmp: tmp }
    }

    #[test]
    fn defaults_without_a_file() {
        let d = dirs(None);
        let c = load(&d.file, &d.data, &d.web).unwrap();
        assert_eq!(c.port, 0);
        assert_eq!(c.archive_dir, d.data);
        assert_eq!(c.web_dir, d.web);
        assert_eq!(c.retention_hours, 72);
        assert_eq!(c.backfill_hours, 2);
        assert_eq!(c.decoded_cache_frames, 10);
        assert_eq!(c.mrms_poll_secs, Config::default().mrms_poll_secs);
        assert_eq!(c.basemap_style, Config::default().basemap_style);
    }

    #[test]
    fn file_values_override_defaults() {
        let d = dirs(Some("backfill_hours = 6\narchive_dir = \"/srv/radar\"\n"));
        let c = load(&d.file, &d.data, &d.web).unwrap();
        assert_eq!(c.backfill_hours, 6);
        assert_eq!(c.archive_dir, PathBuf::from("/srv/radar"));
        assert_eq!(c.decoded_cache_frames, 10);
    }

    #[test]
    fn port_and_web_dir_are_always_the_apps() {
        let d = dirs(Some("port = 9000\nweb_dir = \"/elsewhere\"\n"));
        let c = load(&d.file, &d.data, &d.web).unwrap();
        assert_eq!(c.port, 0);
        assert_eq!(c.web_dir, d.web);
    }

    #[test]
    fn unknown_key_is_an_error_naming_the_file() {
        let d = dirs(Some("backfil_hours = 6\n"));
        let err = format!("{:#}", load(&d.file, &d.data, &d.web).unwrap_err());
        assert!(err.contains("rust-radar.toml") && err.contains("backfil_hours"), "{err}");
    }

    #[test]
    fn malformed_toml_is_an_error_naming_the_file() {
        let d = dirs(Some("backfill_hours = \n"));
        let err = format!("{:#}", load(&d.file, &d.data, &d.web).unwrap_err());
        assert!(err.contains("rust-radar.toml"), "{err}");
    }

    #[test]
    fn invalid_values_are_rejected() {
        let d = dirs(Some("backfill_hours = 100\nretention_hours = 24\n"));
        assert!(load(&d.file, &d.data, &d.web).is_err());
    }
}
```

- [ ] **Step 4: Run to verify failure**

Run: `cargo test -p rust-radar-desktop 2>&1 | tail -15` (first build compiles Tauri — several minutes; redirect to `$SP/t3.log` and read the tail).
Expected: the crate builds; 6 tests FAIL with `not yet implemented`.

- [ ] **Step 5: Implement `load`**

```rust
use anyhow::Context;

pub fn load(config_file: &Path, data_dir: &Path, web_dir: &Path) -> anyhow::Result<Config> {
    let mut table = defaults(data_dir);
    match std::fs::read_to_string(config_file) {
        Ok(text) => {
            let file: toml::Table =
                text.parse().with_context(|| format!("parsing {}", config_file.display()))?;
            table.extend(file);
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).with_context(|| format!("reading {}", config_file.display())),
    }
    let mut config: Config = toml::Value::Table(table)
        .try_into()
        .with_context(|| format!("in {}", config_file.display()))?;
    config.port = 0;
    config.web_dir = web_dir.to_path_buf();
    config.validate().with_context(|| format!("in {}", config_file.display()))?;
    Ok(config)
}

/// Desktop defaults that differ from the server's; everything else comes from `Config::default()`.
fn defaults(data_dir: &Path) -> toml::Table {
    let mut t = toml::Table::new();
    t.insert("archive_dir".into(), data_dir.to_string_lossy().into_owned().into());
    t.insert("retention_hours".into(), 72.into());
    t.insert("backfill_hours".into(), 2.into());
    t.insert("decoded_cache_frames".into(), 10.into());
    t
}
```
Remove the `let _ = ...; todo!()` stub.

- [ ] **Step 6: Run to verify pass; check the droplet build is untouched**

Run: `cargo test -p rust-radar-desktop 2>&1 | tail -3` → Expected: `6 passed`.
Run: `cargo test 2>&1 | grep -E '^test result' | grep -v ' 0 failed' ; echo exit=$?` → Expected: no lines printed (all ok).
Run: `cargo tree -p rust-radar -e normal | grep -c tauri` → Expected: `0` (the server does not depend on Tauri). Then `CC_x86_64_unknown_linux_musl=gcc cargo build --release --locked --target x86_64-unknown-linux-musl 2>&1 | tail -1` → Expected: `Finished`.
Run: `git status --short` → Expected: `Cargo.lock` modified (Tauri entries), new `desktop/` files; no `desktop/target/`.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock desktop/
git commit -m "feat(desktop): Tauri crate scaffold and desktop settings

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Desktop startup — logging, server, window, error dialog, single instance

**Files:**
- Create: `desktop/src/log_file.rs`
- Modify: `desktop/src/main.rs` (whole file)

**Interfaces:**
- Consumes: `settings::load` (Task 3), `rust_radar::app::{start, Running}` (Task 2), `rust_radar::fetch::HttpFetcher`.
- Produces: `log_file::open_log(path: &Path, max_bytes: u64) -> std::io::Result<std::fs::File>` and `log_file::init(path: &Path) -> anyhow::Result<()>`; the runnable desktop binary.

- [ ] **Step 1: Write the failing log test**

`desktop/src/log_file.rs`:
```rust
//! The desktop app has no terminal, so logs go to a file in the data directory.

use std::fs::File;
use std::path::Path;

pub const MAX_LOG_BYTES: u64 = 10 * 1024 * 1024;

/// Opens the log for appending, emptying it first if it has grown past `max_bytes`.
pub fn open_log(path: &Path, max_bytes: u64) -> std::io::Result<File> {
    let _ = (path, max_bytes);
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn open_log_truncates_only_when_over_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rust-radar.log");
        std::fs::write(&path, vec![b'x'; 100]).unwrap();
        let mut f = open_log(&path, 1000).unwrap();
        f.write_all(b"more").unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 104, "small log kept and appended");
        std::fs::write(&path, vec![b'x'; 1001]).unwrap();
        open_log(&path, 1000).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0, "big log emptied");
    }

    #[test]
    fn open_log_creates_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rust-radar.log");
        open_log(&path, 1000).unwrap();
        assert!(path.exists());
    }
}
```
Add `mod log_file;` to `desktop/src/main.rs`.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p rust-radar-desktop log_file 2>&1 | tail -5` → Expected: 2 FAIL with `not yet implemented`.

- [ ] **Step 3: Implement**

```rust
use std::fs::OpenOptions;
use std::sync::Mutex;

use anyhow::Context;
use tracing_subscriber::EnvFilter;

pub fn open_log(path: &Path, max_bytes: u64) -> std::io::Result<File> {
    let too_big = std::fs::metadata(path).is_ok_and(|m| m.len() > max_bytes);
    if too_big {
        File::create(path)?; // truncate
    }
    OpenOptions::new().create(true).append(true).open(path)
}

/// Sends `tracing` output (RUST_LOG, default info) to `path`.
pub fn init(path: &Path) -> anyhow::Result<()> {
    let file = open_log(path, MAX_LOG_BYTES).with_context(|| format!("opening {}", path.display()))?;
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with_ansi(false)
        .with_writer(Mutex::new(file))
        .try_init()
        .map_err(|e| anyhow::anyhow!("starting logging: {e}"))
}
```
Remove the stub. Run: `cargo test -p rust-radar-desktop 2>&1 | tail -3` → Expected: `8 passed`.

- [ ] **Step 4: Desktop `main.rs`**

```rust
// Desktop rust-radar: runs the radar server in-process and shows it in a window.

mod log_file;
mod settings;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

fn main() {
    tauri::Builder::default()
        // Must be the first plugin: a second launch focuses this window and exits.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            match launch(app.handle()) {
                Ok(url) => {
                    WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
                        .title("rust-radar")
                        .inner_size(1280.0, 800.0)
                        .build()?;
                }
                Err(e) => {
                    tracing::error!("could not start: {e:#}");
                    let handle = app.handle().clone();
                    app.dialog()
                        .message(format!("rust-radar could not start:\n\n{e:#}"))
                        .title("rust-radar")
                        .kind(MessageDialogKind::Error)
                        .show(move |_| handle.exit(1));
                }
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("running the Tauri app");
}

/// Logging, settings and the in-process server; returns the URL for the window.
fn launch(app: &AppHandle) -> anyhow::Result<tauri::Url> {
    let paths = app.path();
    let data_dir = paths.data_dir().context("finding the data directory")?.join("rust-radar");
    let config_file = paths.config_dir().context("finding the config directory")?.join("rust-radar").join("rust-radar.toml");
    std::fs::create_dir_all(&data_dir).with_context(|| format!("creating {}", data_dir.display()))?;
    log_file::init(&data_dir.join("rust-radar.log"))?;
    let config = settings::load(&config_file, &data_dir, &web_dir(app)?)?;
    tracing::info!("desktop: archive {}, settings {}", config.archive_dir.display(), config_file.display());
    let fetcher = Arc::new(rust_radar::fetch::HttpFetcher::new()?);
    let running = tauri::async_runtime::block_on(rust_radar::app::start(&config, fetcher))?;
    Ok(format!("http://127.0.0.1:{}/", running.port).parse()?)
}

/// The bundled copy of web/; in a `cargo run` build, the repo's web/ directory.
fn web_dir(app: &AppHandle) -> anyhow::Result<PathBuf> {
    let bundled = app.path().resource_dir().context("finding the resource directory")?.join("web");
    if bundled.join("index.html").is_file() {
        return Ok(bundled);
    }
    let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../web"));
    anyhow::ensure!(repo.join("index.html").is_file(), "web files not found in {} or {}", bundled.display(), repo.display());
    Ok(repo)
}
```
Note: `running` is dropped at the end of `launch`; dropping a tokio `JoinHandle` detaches the task, it does not stop it. The server and pollers live on Tauri's tokio runtime until the process exits.

- [ ] **Step 5: Build and run it (dev build)**

```bash
cargo build -p rust-radar-desktop 2>&1 | tail -2
./target/debug/rust-radar-desktop & DPID=$!
sleep 15; import -window root $SP/t4-window.png
tail -5 ~/.local/share/rust-radar/rust-radar.log
```
Expected: `Finished`; the log has `rust-radar listening on http://127.0.0.1:<port>` and `backfilled`/MRMS lines appear over the next minute; the screenshot shows the rust-radar window with the map. Read the screenshot.

- [ ] **Step 6: Closing the window ends the process**

Close the window through the window manager: `xdotool search --name '^rust-radar$' windowclose` if `xdotool` is installed (check `which xdotool`), otherwise `wmctrl -c rust-radar`; if neither exists, send `kill -TERM $DPID` and note in the ledger that window-close was not exercised here (the user checks it in Task 5).
Then: `sleep 2; kill -0 $DPID 2>/dev/null && echo STILL RUNNING || echo exited`
Expected: `exited`.

- [ ] **Step 7: Error dialog instead of a blank window**

```bash
mkdir -p ~/.config/rust-radar
[ -e ~/.config/rust-radar/rust-radar.toml ] && echo "EXISTS — stop, do not overwrite" || {
  printf 'backfil_hours = 6\n' > ~/.config/rust-radar/rust-radar.toml
  ./target/debug/rust-radar-desktop & DPID=$!
  sleep 5; import -window root $SP/t4-error.png
  kill $DPID; rm ~/.config/rust-radar/rust-radar.toml
}
```
Expected: the screenshot shows an error dialog mentioning `rust-radar.toml` and `backfil_hours`; no map window. Read the screenshot. Then the settings file is removed again.

- [ ] **Step 8: Run all tests and commit**

Run: `cargo test -p rust-radar-desktop 2>&1 | tail -3` → `8 passed`. Run: `cargo test 2>&1 | grep -E '^test result'` → all ok.
```bash
git add desktop/src/
git commit -m "feat(desktop): run the server in-process and show it in a window

Logs to ~/.local/share/rust-radar/rust-radar.log; a startup error shows a dialog;
a second launch focuses the first window.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Packaging and docs

**Files:**
- Create: `desktop/build.sh`
- Modify: `README.md` (new "Desktop app" section)
- Modify: `.gitignore` only if the bundle step creates files outside `target/` (check `git status`)

- [ ] **Step 1: Build script**

`desktop/build.sh`:
```bash
#!/usr/bin/env bash
# Builds the Linux desktop app: a .deb and an AppImage under target/release/bundle/.
# Needs: cargo install tauri-cli --version '^2' --locked, and the WebKitGTK 4.1 dev packages.
set -euo pipefail
cd "$(dirname "$0")"
cargo tauri build
echo
ls -1 ../target/release/bundle/deb/*.deb ../target/release/bundle/appimage/*.AppImage
```
`chmod +x desktop/build.sh`

- [ ] **Step 2: Build the bundles**

Run: `desktop/build.sh > $SP/t5.log 2>&1; tail -5 $SP/t5.log`
Expected: ends listing one `.deb` and one `.AppImage`. If AppImage bundling fails (it downloads `linuxdeploy` on first use), record the error, keep the `.deb`, and tell the user rather than working around it silently.

- [ ] **Step 3: Check the bundle contains the web files**

Run: `dpkg-deb -c target/release/bundle/deb/*.deb | grep -E 'web/(index.html|app.js|playback.js|satellite.js)'`
Expected: 4 lines, under `usr/lib/rust-radar/web/` (or the path Tauri chose — record it).

- [ ] **Step 4: Run the AppImage and check it uses the bundled page**

```bash
chmod +x target/release/bundle/appimage/*.AppImage
target/release/bundle/appimage/*.AppImage & APID=$!
sleep 15; import -window root $SP/t5-appimage.png
PORT=$(grep -o 'listening on http://127.0.0.1:[0-9]*' ~/.local/share/rust-radar/rust-radar.log | tail -1 | grep -o '[0-9]*$')
curl -s 127.0.0.1:$PORT/playback.js | head -2
```
Expected: the screenshot shows the map window; `playback.js` is served.

- [ ] **Step 5: Second launch focuses the first**

Run: `target/release/bundle/appimage/*.AppImage & BPID=$!; sleep 5; kill -0 $BPID 2>/dev/null && echo "second still running" || echo "second exited"; grep -c 'listening on' ~/.local/share/rust-radar/rust-radar.log`
Expected: `second exited`; the `listening on` count did not increase from that launch. Then close the first window as in Task 4 Step 6 and confirm `$APID` exited.

- [ ] **Step 6: README section**

Add after the Deploy section:
```markdown
## Desktop app (Linux)

Runs everything on your machine — no server or password. Radar is collected only while the window
is open; when you reopen it, it catches up on the last 2 hours.

One-time setup: `sudo apt install libwebkit2gtk-4.1-dev librsvg2-dev` (if not already installed) and
`cargo install tauri-cli --version '^2' --locked`.

Build: `desktop/build.sh` → `target/release/bundle/deb/*.deb` (install with `sudo apt install ./<file>.deb`)
and `target/release/bundle/appimage/*.AppImage` (run directly).

- Data: `~/.local/share/rust-radar/` (72 hours kept). Log: `~/.local/share/rust-radar/rust-radar.log`.
- Optional settings: `~/.config/rust-radar/rust-radar.toml`, same keys as `rust-radar.toml`
  (`port` and `web_dir` are ignored — the app picks those).
- For development: `cargo run -p rust-radar-desktop`.
```

- [ ] **Step 7: Final checks and commit**

Run: `cargo test 2>&1 | grep -E '^test result'` (all ok), `cargo test -p rust-radar-desktop 2>&1 | tail -3` (`8 passed`), `node --test 'web/*.test.js' 2>&1 | grep -E '^# (pass|fail)'` (`pass 16`, `fail 0`), and the musl build from Task 3 Step 6.
Run: `git status --short` → only `desktop/build.sh` and `README.md` (no bundle output outside `target/`).
```bash
git add desktop/build.sh README.md
git commit -m "feat(desktop): .deb and AppImage build script and README section

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```
