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
    assert!(!archive.has_geometry("al092026", t(8, 15)), "zip lacks wind radii and past track");
    archive.store(&snapshot(t(8, 15), "008", "mapserver")).unwrap();
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

#[test]
fn advisory_published_before_its_nominal_time_is_visible() {
    // NHC posts advisories a few minutes before their nominal issuance time.
    let dir = tempfile::tempdir().unwrap();
    let archive = NhcArchive::open(dir.path()).unwrap();
    archive.store(&snapshot(t(8, 18), "005a", "mapserver")).unwrap();
    let just_before = t(8, 18) - Duration::minutes(7);
    assert_eq!(archive.get("al092026", just_before).unwrap().unwrap().storm.advisory_num, "005a");
    assert_eq!(archive.at_time(just_before).unwrap().len(), 1);
    assert!(archive.get("al092026", t(8, 16)).unwrap().is_none(), "not hours early");
}

#[test]
fn storm_dropped_from_the_feed_is_hidden_from_then_on() {
    let dir = tempfile::tempdir().unwrap();
    let archive = NhcArchive::open(dir.path()).unwrap();
    archive.store(&snapshot(t(8, 15), "008", "mapserver")).unwrap();

    archive.record_active(&[], t(8, 16)).unwrap();
    assert_eq!(archive.ended_at("al092026"), Some(t(8, 16)));
    assert!(archive.get("al092026", t(8, 17)).unwrap().is_none(), "ended storm still shown live");
    assert!(archive.at_time(t(8, 17)).unwrap().is_empty());
    assert!(archive.get("al092026", t(8, 15) + Duration::minutes(30)).unwrap().is_some(), "replay before the end keeps it");

    // Seen again (e.g. regenerated): visible again.
    archive.record_active(&["al092026".to_string()], t(8, 18)).unwrap();
    assert_eq!(archive.ended_at("al092026"), None);
    assert!(archive.get("al092026", t(8, 19)).unwrap().is_some());

    // Survives reopening.
    archive.record_active(&[], t(8, 20)).unwrap();
    assert_eq!(NhcArchive::open(dir.path()).unwrap().ended_at("al092026"), Some(t(8, 20)));
}

#[test]
fn prune_keeps_the_advisory_covering_the_cutoff() {
    let dir = tempfile::tempdir().unwrap();
    let archive = NhcArchive::open(dir.path()).unwrap();
    archive.store(&snapshot(t(5, 9), "001", "mapserver")).unwrap();
    archive.store(&snapshot(t(5, 21), "003", "mapserver")).unwrap();
    archive.store(&snapshot(t(8, 15), "008", "mapserver")).unwrap();
    assert_eq!(archive.prune(t(6, 0)).unwrap(), 1);
    assert_eq!(
        archive.get("al092026", t(6, 1)).unwrap().unwrap().storm.advisory_num,
        "003",
        "start of the replay window keeps its storm"
    );
}
