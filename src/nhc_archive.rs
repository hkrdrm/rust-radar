//! On-disk NHC advisory snapshots: `<dir>/<storm_id>/<issuance>.json`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::sources::nhc::StormInfo;

pub const GEOMETRY_NONE: &str = "none";
/// A storm with no advisory for this long is treated as gone.
pub const STALE_AFTER_HOURS: i64 = 12;
/// NHC posts advisories shortly before their nominal issuance time.
pub const EARLY_PUBLISH_MINUTES: i64 = 30;
const TIME_FORMAT: &str = "%Y%m%d-%H%M%S";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StormSnapshot {
    pub storm: StormInfo,
    pub features: Vec<Value>,
    pub geometry_source: String,
}

pub struct NhcArchive {
    dir: PathBuf,
}

fn valid_storm_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric())
}

fn snapshot_file_name(issuance: DateTime<Utc>) -> String {
    format!("{}.json", issuance.format(TIME_FORMAT))
}

fn parse_snapshot_file_name(name: &str) -> Option<DateTime<Utc>> {
    let stem = name.strip_suffix(".json")?;
    NaiveDateTime::parse_from_str(stem, TIME_FORMAT).ok().map(|n| n.and_utc())
}

fn read_snapshot(path: &Path) -> anyhow::Result<StormSnapshot> {
    let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))
}

impl NhcArchive {
    pub fn open(dir: impl Into<PathBuf>) -> anyhow::Result<NhcArchive> {
        let dir = dir.into();
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        Ok(NhcArchive { dir })
    }

    fn snapshot_path(&self, storm_id: &str, issuance: DateTime<Utc>) -> PathBuf {
        self.dir.join(storm_id).join(snapshot_file_name(issuance))
    }

    pub fn store(&self, snapshot: &StormSnapshot) -> anyhow::Result<()> {
        let storm_id = &snapshot.storm.id;
        anyhow::ensure!(valid_storm_id(storm_id), "invalid storm id {storm_id:?}");
        let path = self.snapshot_path(storm_id, snapshot.storm.issuance);
        fs::create_dir_all(path.parent().expect("snapshot path has a parent"))?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec(snapshot)?).with_context(|| format!("writing {}", tmp.display()))?;
        fs::rename(&tmp, &path).with_context(|| format!("renaming to {}", path.display()))?;
        Ok(())
    }

    pub fn has_geometry(&self, storm_id: &str, issuance: DateTime<Utc>) -> bool {
        valid_storm_id(storm_id)
            && read_snapshot(&self.snapshot_path(storm_id, issuance))
                .is_ok_and(|s| s.geometry_source != GEOMETRY_NONE)
    }

    fn issuances(&self, storm_id: &str) -> anyhow::Result<Vec<DateTime<Utc>>> {
        let dir = self.dir.join(storm_id);
        if !dir.is_dir() {
            return Ok(Vec::new());
        }
        let mut times = Vec::new();
        for entry in fs::read_dir(&dir).with_context(|| format!("listing {}", dir.display()))? {
            if let Some(t) = entry?.file_name().to_str().and_then(parse_snapshot_file_name) {
                times.push(t);
            }
        }
        times.sort();
        Ok(times)
    }

    pub fn get(&self, storm_id: &str, t: DateTime<Utc>) -> anyhow::Result<Option<StormSnapshot>> {
        if !valid_storm_id(storm_id) {
            return Ok(None);
        }
        let visible_until = t + Duration::minutes(EARLY_PUBLISH_MINUTES);
        let current = self.issuances(storm_id)?.into_iter().filter(|&i| i <= visible_until).last();
        match current {
            Some(issuance) if t - issuance <= Duration::hours(STALE_AFTER_HOURS) => {
                Ok(Some(read_snapshot(&self.snapshot_path(storm_id, issuance))?))
            }
            _ => Ok(None),
        }
    }

    pub fn at_time(&self, t: DateTime<Utc>) -> anyhow::Result<Vec<StormSnapshot>> {
        let mut snapshots = Vec::new();
        for entry in fs::read_dir(&self.dir).with_context(|| format!("listing {}", self.dir.display()))? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            if let Some(id) = entry.file_name().to_str() {
                if let Some(snapshot) = self.get(id, t)? {
                    snapshots.push(snapshot);
                }
            }
        }
        snapshots.sort_by(|a, b| a.storm.id.cmp(&b.storm.id));
        Ok(snapshots)
    }

    /// Removes snapshots issued before `cutoff`, leftover temp files, and emptied storm dirs.
    pub fn prune(&self, cutoff: DateTime<Utc>) -> anyhow::Result<usize> {
        let mut removed = 0;
        for storm_dir in fs::read_dir(&self.dir)? {
            let storm_dir = storm_dir?.path();
            if !storm_dir.is_dir() {
                continue;
            }
            for file in fs::read_dir(&storm_dir)? {
                let path = file?.path();
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                if name.ends_with(".tmp") {
                    fs::remove_file(&path)?;
                } else if parse_snapshot_file_name(name).is_some_and(|t| t < cutoff) {
                    fs::remove_file(&path)?;
                    removed += 1;
                }
            }
            if fs::read_dir(&storm_dir)?.next().is_none() {
                fs::remove_dir(&storm_dir)?;
            }
        }
        Ok(removed)
    }
}
