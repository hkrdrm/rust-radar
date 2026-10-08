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
