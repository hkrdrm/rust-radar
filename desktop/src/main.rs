// Desktop rust-radar: runs the radar server in-process and shows it in a window.

mod log_file;
mod port;
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
    let config_file =
        paths.config_dir().context("finding the config directory")?.join("rust-radar").join("rust-radar.toml");
    std::fs::create_dir_all(&data_dir).with_context(|| format!("creating {}", data_dir.display()))?;
    log_file::init(&data_dir.join("rust-radar.log"))?;
    let mut config = settings::load(&config_file, &data_dir, &web_dir(app)?)?;
    config.port = port::pick_port(port::PREFERRED_PORT);
    tracing::info!(
        "desktop: archive {}, settings {}, page files {}",
        config.archive_dir.display(),
        config_file.display(),
        config.web_dir.display()
    );
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
    anyhow::ensure!(
        repo.join("index.html").is_file(),
        "web files not found in {} or {}",
        bundled.display(),
        repo.display()
    );
    Ok(repo)
}
