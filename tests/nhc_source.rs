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

#[tokio::test]
async fn zip_snapshot_is_upgraded_once_map_service_catches_up() {
    let (_dir, archive) = archive();
    let lagging = fake_with_mapserver("7");
    lagging.ok(ZIP_URL, common::nhc_zip_fixture());
    poll_once(&lagging, &archive).await.unwrap();
    assert_eq!(stored(&archive).geometry_source, "zip");

    let caught_up = fake_with_mapserver("8");
    poll_once(&caught_up, &archive).await.unwrap();
    let snapshot = stored(&archive);
    assert_eq!(snapshot.geometry_source, "mapserver");
    assert!(snapshot.features.iter().any(|f| f["properties"]["layer"] == "wind_radii"));
}

#[tokio::test]
async fn later_failure_never_downgrades_stored_geometry() {
    let (_dir, archive) = archive();
    let lagging = fake_with_mapserver("7");
    lagging.ok(ZIP_URL, common::nhc_zip_fixture());
    poll_once(&lagging, &archive).await.unwrap();

    let broken = FakeFetcher::new();
    broken.ok(CURRENT_STORMS_URL, STORMS);
    broken.fail(&layer_index_url(), "503");
    broken.fail(ZIP_URL, "404");
    poll_once(&broken, &archive).await.unwrap();
    let snapshot = stored(&archive);
    assert_eq!(snapshot.geometry_source, "zip");
    assert!(!snapshot.features.is_empty());
}
