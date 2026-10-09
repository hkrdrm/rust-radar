# rust-radar desktop app — design

Date: 2026-10-09. Status: approved in conversation; awaiting review of this written spec.

## Goal

A standalone Linux desktop app that runs rust-radar entirely on the user's machine — its own
radar/NHC collection, archive and map window — with no droplet involved.

**What the user decided:**
- Linux only; one user (the author). Windows/macOS are out of scope but must not be designed out.
- Closing the window quits the app; collection only runs while it is open.
- Built with Tauri (approach A). Fallback, if WebKitGTK renders the map poorly: a `--desktop`
  mode that opens Chrome in app mode (approach B).

**Assumptions (not stated by the user):** the basemap still comes from OpenFreeMap and the satellite
from NASA GIBS, so the app needs internet just as the website does.

## Success criteria

1. Launching the installed app opens a window showing the same map, radar, storms, satellite and
   playback (including 100×) as the website, without a password.
2. Radar frames are collected while the window is open and survive restarts (72 h retention).
3. Closing the window stops all collection and exits the process.
4. The droplet build and `deploy/deploy.sh` are unchanged in behaviour.

## Architecture

### Shared startup: `src/app.rs` (library)

Everything `src/main.rs` does today moves into one async function:

```rust
pub struct Running { pub port: u16, /* join handle for the HTTP server */ }
pub async fn start(config: &Config, fetcher: Arc<dyn Fetcher>) -> anyhow::Result<Running>;
```

It opens both archives, spawns the MRMS and NHC pollers, the backfill and the prune loop, binds
`127.0.0.1:config.port` and serves the router. `port = 0` lets the OS pick a free port; `Running.port`
reports the real one. Taking the fetcher as a parameter lets tests pass the existing test fetcher.

`src/main.rs` becomes: init logging, load config, `start`, await the server. Behaviour, CLI flags and the
musl build are unchanged.

### Desktop crate: `desktop/`

A Tauri 2 binary crate (`rust-radar-desktop`) depending on the `rust-radar` library by path.
The repository becomes a Cargo workspace whose root package stays the default member, so `cargo build`
at the root (and therefore `deploy.sh`) still builds only the server and never compiles Tauri.

On launch it:
1. Takes a single-instance lock (Tauri's single-instance plugin). A second launch focuses the
   existing window and exits.
2. Builds the desktop config (below), using the bundled `web/` resource directory as `web_dir`.
3. Calls `app::start` with `port = 0`.
4. Opens one window titled "rust-radar" at `http://127.0.0.1:<port>/`.
5. Exits the process when the window closes.

### Desktop configuration

Defaults: `port = 0`, `archive_dir = ~/.local/share/rust-radar`, `retention_hours = 72`,
`backfill_hours = 2`, `decoded_cache_frames = 10`, poll intervals and basemap as today.
An optional `~/.config/rust-radar/rust-radar.toml` (same keys, `deny_unknown_fields`) overrides them;
a missing file is fine. `port` and `web_dir` are always set by the app (a free port and the bundled
`web/` directory), whatever the file says.

Paths come from Tauri's path resolver (`data_dir()`, `config_dir()`, each joined with `rust-radar`) so
other platforms would get their native locations if ever added.

### Logging

`tracing` output goes to `~/.local/share/rust-radar/rust-radar.log` (append; `RUST_LOG` respected,
default `info`). The file is truncated when it exceeds 10 MB at startup.

## Error handling

- `app::start` failing (archive directory not writable, disk full, bind error): show a native error
  dialog with the error chain, then exit non-zero. Never show a blank window.
- Network outages: unchanged from the server — pollers retry and the status bar shows radar age.
- A server task dying after startup is logged; the window stays open showing the last data.

## Packaging

`desktop/build.sh` runs `cargo tauri build` and produces a `.deb` and an AppImage under
`target/release/bundle/`. Requires the one-time `cargo install tauri-cli --version '^2' --locked`
(WebKitGTK 4.1 dev packages are already installed). README gains a "Desktop app" section:
prerequisites, build, install, where data and logs live.

Icon: a simple generated radar-sweep PNG set (Tauri requires one); no brand assets.

## Testing

- **Step zero — WebKitGTK check (throwaway):** a bare Tauri window pointed at a locally running server.
  Check the map renders, the satellite layer shows, and 100× playback runs without visible stalls.
  If it is unacceptable, stop and return to the user with approach B.
- `app::start` integration test: start with `port = 0`, a temp archive dir and the test fetcher;
  assert `GET /api/status` returns 200 on the reported port.
- Desktop config tests: defaults with no file; file values override defaults; unknown keys rejected.
- Existing suites stay green: `cargo test` (root) and `node --test 'web/*.test.js'`.
- The droplet build is re-checked: `cargo build --release --locked --target x86_64-unknown-linux-musl`
  at the root still succeeds without building Tauri.
- Manual: launch the built AppImage, take screenshots, close the window and confirm the process is gone.
  How it feels in daily use is for the user to judge.

## Out of scope

Windows/macOS builds and code signing, tray/background collection, auto-update, offline basemap.
