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
