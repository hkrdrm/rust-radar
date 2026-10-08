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
