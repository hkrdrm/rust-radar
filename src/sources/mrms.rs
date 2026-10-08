//! MRMS composite reflectivity: live polling and S3 backfill.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context};
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use tracing::{info, warn};

use crate::fetch::Fetcher;
use crate::mrms_archive::MrmsArchive;
use crate::status::{backoff_delay, supervise, Status};

pub const SOURCE: &str = "mrms";
pub const PRODUCT: &str = "MergedReflectivityQCComposite_00.50";
pub const LIVE_DIR_URL: &str = "https://mrms.ncep.noaa.gov/2D/MergedReflectivityQCComposite/";
pub const S3_BASE_URL: &str = "https://noaa-mrms-pds.s3.amazonaws.com";
const FILE_PREFIX: &str = "MRMS_MergedReflectivityQCComposite_00.50_";
const FILE_SUFFIX: &str = ".grib2.gz";
const TIME_FORMAT: &str = "%Y%m%d-%H%M%S";

pub fn remote_file_name(t: DateTime<Utc>) -> String {
    format!("{FILE_PREFIX}{}{FILE_SUFFIX}", t.format(TIME_FORMAT))
}

pub fn live_url(t: DateTime<Utc>) -> String {
    format!("{LIVE_DIR_URL}{}", remote_file_name(t))
}

/// One S3 page holds 1000 keys; MRMS publishes ~720 files a day, so no pagination is needed.
pub fn s3_list_url(day: NaiveDate) -> String {
    format!("{S3_BASE_URL}/?list-type=2&prefix=CONUS/{PRODUCT}/{}/", day.format("%Y%m%d"))
}

pub fn s3_url(t: DateTime<Utc>) -> String {
    format!("{S3_BASE_URL}/CONUS/{PRODUCT}/{}/{}", t.format("%Y%m%d"), remote_file_name(t))
}

/// Every frame time named in an HTML directory listing or S3 XML listing, sorted and de-duplicated.
pub fn parse_listing(text: &str) -> Vec<DateTime<Utc>> {
    let mut times = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find(FILE_PREFIX) {
        rest = &rest[i + FILE_PREFIX.len()..];
        let has_suffix = rest.get(15..).is_some_and(|s| s.starts_with(FILE_SUFFIX));
        if let (Some(stamp), true) = (rest.get(..15), has_suffix) {
            if let Ok(naive) = NaiveDateTime::parse_from_str(stamp, TIME_FORMAT) {
                times.push(naive.and_utc());
            }
        }
    }
    times.sort();
    times.dedup();
    times
}

/// UTC calendar days touched by [start, end].
pub fn days_in_window(start: DateTime<Utc>, end: DateTime<Utc>) -> Vec<NaiveDate> {
    let mut days = Vec::new();
    let mut day = start.date_naive();
    while day <= end.date_naive() {
        days.push(day);
        day = day.succ_opt().expect("date overflow");
    }
    days
}

/// Downloads every live frame from the last `catchup` (relative to the newest) that is not
/// archived yet, oldest first, so gaps from outages close on the next good poll.
/// Fails only when the newest frame itself cannot be stored; older gaps are logged.
pub async fn poll_once(
    fetcher: &dyn Fetcher,
    archive: &Arc<MrmsArchive>,
    catchup: chrono::Duration,
) -> anyhow::Result<Vec<DateTime<Utc>>> {
    let listing = fetcher.get(LIVE_DIR_URL).await.context("fetching MRMS listing")?;
    let times = parse_listing(&String::from_utf8_lossy(&listing));
    let Some(&newest) = times.last() else {
        bail!("MRMS listing contains no {PRODUCT} files");
    };
    let mut stored = Vec::new();
    for t in times.into_iter().filter(|&t| t >= newest - catchup && !archive.contains(t)) {
        let result = match fetcher.get(&live_url(t)).await {
            Ok(bytes) => store(archive, t, bytes).await,
            Err(e) => Err(e),
        };
        match result {
            Ok(()) => stored.push(t),
            Err(e) if t == newest => return Err(e.context(format!("downloading newest MRMS frame {t}"))),
            Err(e) => warn!("could not fill MRMS frame {t}: {e:#}"),
        }
    }
    Ok(stored)
}

/// Fills gaps in the last `hours` from S3. Individual frame failures are logged and skipped.
pub async fn backfill(fetcher: &dyn Fetcher, archive: &Arc<MrmsArchive>, now: DateTime<Utc>, hours: u64) -> anyhow::Result<usize> {
    let start = now - chrono::Duration::hours(hours as i64);
    let mut stored = 0;
    for day in days_in_window(start, now) {
        let listing = match fetcher.get(&s3_list_url(day)).await {
            Ok(listing) => listing,
            Err(e) => {
                warn!("backfill skipped {day}: listing MRMS on S3 failed: {e:#}");
                continue;
            }
        };
        for t in parse_listing(&String::from_utf8_lossy(&listing)) {
            if t < start || t > now || archive.contains(t) {
                continue;
            }
            let result = match fetcher.get(&s3_url(t)).await {
                Ok(bytes) => store(archive, t, bytes).await,
                Err(e) => Err(e),
            };
            match result {
                Ok(()) => stored += 1,
                Err(e) => warn!("backfill skipped MRMS frame {t}: {e:#}"),
            }
        }
    }
    Ok(stored)
}

async fn store(archive: &Arc<MrmsArchive>, t: DateTime<Utc>, bytes: Vec<u8>) -> anyhow::Result<()> {
    let archive = Arc::clone(archive);
    tokio::task::spawn_blocking(move || archive.store(t, &bytes).map(|_| ()))
        .await
        .context("MRMS store task panicked")?
}

pub async fn run(
    fetcher: Arc<dyn Fetcher>,
    archive: Arc<MrmsArchive>,
    status: Status,
    interval: Duration,
    catchup: chrono::Duration,
) {
    loop {
        let (fetcher, archive) = (Arc::clone(&fetcher), Arc::clone(&archive));
        let failures = supervise(&status, SOURCE, async move {
            for t in poll_once(fetcher.as_ref(), &archive, catchup).await? {
                info!("stored MRMS frame {t}");
            }
            Ok(())
        })
        .await;
        tokio::time::sleep(backoff_delay(interval, failures)).await;
    }
}
