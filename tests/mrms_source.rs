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

const CATCHUP: chrono::Duration = chrono::Duration::hours(6);

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
    assert_eq!(poll_once(&fake, &archive, CATCHUP).await.unwrap(), vec![t(17, 18, 34)]);
    assert_eq!(archive.frames().unwrap(), vec![t(17, 18, 34)]);
    assert!(poll_once(&fake, &archive, CATCHUP).await.unwrap().is_empty());
    assert_eq!(fake.calls_to(&live_url(t(17, 18, 34))), 1);
}

#[tokio::test]
async fn poll_rejects_corrupt_frame() {
    let (_dir, archive) = archive();
    let fake = FakeFetcher::new();
    fake.ok(LIVE_DIR_URL, HTML);
    fake.ok(&live_url(t(17, 18, 34)), b"garbage".to_vec());
    assert!(poll_once(&fake, &archive, CATCHUP).await.is_err());
    assert!(archive.frames().unwrap().is_empty());
}

#[tokio::test]
async fn poll_reports_listing_failure_and_empty_listing() {
    let (_dir, archive) = archive();
    let fake = FakeFetcher::new();
    fake.fail(LIVE_DIR_URL, "503");
    assert!(poll_once(&fake, &archive, CATCHUP).await.is_err());
    fake.ok(LIVE_DIR_URL, "<html>nothing here</html>");
    assert!(poll_once(&fake, &archive, CATCHUP).await.is_err());
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

#[tokio::test]
async fn poll_fills_gaps_left_by_an_outage() {
    let (_dir, archive) = archive();
    let bytes = common::mrms_fixture();
    archive.store(t(17, 14, 38), &bytes).unwrap();
    let fake = FakeFetcher::new();
    let listing = format!(
        "{}{}",
        remote_file_name(t(11, 0, 0)), // older than newest - catchup: ignored
        HTML
    );
    fake.ok(LIVE_DIR_URL, listing);
    fake.ok(&live_url(t(17, 16, 35)), bytes.clone());
    fake.ok(&live_url(t(17, 18, 34)), bytes.clone());
    assert_eq!(
        poll_once(&fake, &archive, CATCHUP).await.unwrap(),
        vec![t(17, 16, 35), t(17, 18, 34)],
        "missed frames are filled oldest first"
    );
    assert_eq!(fake.calls_to(&live_url(t(11, 0, 0))), 0);
}

#[tokio::test]
async fn backfill_continues_past_a_failed_day_listing() {
    let (_dir, archive) = archive();
    let fake = FakeFetcher::new();
    let yesterday = NaiveDate::from_ymd_opt(2026, 10, 7).unwrap();
    let today = NaiveDate::from_ymd_opt(2026, 10, 8).unwrap();
    fake.fail(&s3_list_url(yesterday), "503");
    fake.ok(&s3_list_url(today), s3_listing(&[t(0, 30, 0)]));
    fake.ok(&s3_url(t(0, 30, 0)), common::mrms_fixture());
    assert_eq!(backfill(&fake, &archive, t(1, 0, 0), 2).await.unwrap(), 1);
}
