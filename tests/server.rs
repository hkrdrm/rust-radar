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
