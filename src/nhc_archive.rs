//! On-disk NHC advisory snapshots: `<dir>/<storm_id>/<issuance>.json`.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::Context;
use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::sources::nhc::StormInfo;

pub const GEOMETRY_NONE: &str = "none";
pub const GEOMETRY_ZIP: &str = "zip";
/// Only the map service carries wind radii and past track; anything else is provisional.
pub const GEOMETRY_MAPSERVER: &str = "mapserver";

/// Higher is more complete.
pub fn geometry_rank(source: &str) -> u8 {
    match source {
        GEOMETRY_MAPSERVER => 2,
        GEOMETRY_ZIP => 1,
        _ => 0,
    }
}
/// A storm with no advisory for this long is treated as gone.
pub const STALE_AFTER_HOURS: i64 = 12;
/// NHC posts advisories shortly before their nominal issuance time.
pub const EARLY_PUBLISH_MINUTES: i64 = 30;
const TIME_FORMAT: &str = "%Y%m%d-%H%M%S";
/// Storm id -> when it first went missing from CurrentStorms.json.
const ENDED_FILE: &str = "ended.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StormSnapshot {
    pub storm: StormInfo,
    pub features: Vec<Value>,
    pub geometry_source: String,
}

pub struct NhcArchive {
    dir: PathBuf,
    ended: Mutex<HashMap<String, DateTime<Utc>>>,
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
        let ended = match fs::read(dir.join(ENDED_FILE)) {
            Ok(bytes) => serde_json::from_slice(&bytes).context("parsing ended.json")?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
            Err(e) => return Err(e).context("reading ended.json"),
        };
        Ok(NhcArchive { dir, ended: Mutex::new(ended) })
    }

    /// Records which storms the latest feed listed. Archived storms missing from it are
    /// marked ended at `now`; listed storms are un-ended.
    pub fn record_active(&self, active: &[String], now: DateTime<Utc>) -> anyhow::Result<()> {
        let mut ended = self.ended.lock().unwrap();
        let before = ended.clone();
        ended.retain(|id, _| !active.contains(id));
        for id in self.storm_ids()? {
            if !active.contains(&id) {
                ended.entry(id).or_insert(now);
            }
        }
        if *ended != before {
            self.save_ended(&ended)?;
        }
        Ok(())
    }

    /// When the storm dropped out of the feed, if it has.
    pub fn ended_at(&self, storm_id: &str) -> Option<DateTime<Utc>> {
        self.ended.lock().unwrap().get(storm_id).copied()
    }

    fn save_ended(&self, ended: &HashMap<String, DateTime<Utc>>) -> anyhow::Result<()> {
        let path = self.dir.join(ENDED_FILE);
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec(ended)?).with_context(|| format!("writing {}", tmp.display()))?;
        fs::rename(&tmp, &path).with_context(|| format!("renaming to {}", path.display()))?;
        Ok(())
    }

    fn storm_ids(&self) -> anyhow::Result<Vec<String>> {
        let mut ids = Vec::new();
        for entry in fs::read_dir(&self.dir).with_context(|| format!("listing {}", self.dir.display()))? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                if let Some(id) = entry.file_name().to_str().filter(|id| valid_storm_id(id)) {
                    ids.push(id.to_string());
                }
            }
        }
        Ok(ids)
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

    /// Geometry source of the stored snapshot for this advisory, if any.
    pub fn geometry_source(&self, storm_id: &str, issuance: DateTime<Utc>) -> Option<String> {
        if !valid_storm_id(storm_id) {
            return None;
        }
        read_snapshot(&self.snapshot_path(storm_id, issuance)).ok().map(|s| s.geometry_source)
    }

    /// True once the advisory has complete (map-service) geometry; nothing left to fetch.
    pub fn has_geometry(&self, storm_id: &str, issuance: DateTime<Utc>) -> bool {
        self.geometry_source(storm_id, issuance).as_deref() == Some(GEOMETRY_MAPSERVER)
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
        if !valid_storm_id(storm_id) || self.ended_at(storm_id).is_some_and(|end| end <= t) {
            return Ok(None);
        }
        let visible_until = t + Duration::minutes(EARLY_PUBLISH_MINUTES);
        let current = self.issuances(storm_id)?.into_iter().rev().find(|&i| i <= visible_until);
        match current {
            Some(issuance) if t - issuance <= Duration::hours(STALE_AFTER_HOURS) => {
                Ok(Some(read_snapshot(&self.snapshot_path(storm_id, issuance))?))
            }
            _ => Ok(None),
        }
    }

    pub fn at_time(&self, t: DateTime<Utc>) -> anyhow::Result<Vec<StormSnapshot>> {
        let mut snapshots = Vec::new();
        for id in self.storm_ids()? {
            if let Some(snapshot) = self.get(&id, t)? {
                snapshots.push(snapshot);
            }
        }
        snapshots.sort_by(|a, b| a.storm.id.cmp(&b.storm.id));
        Ok(snapshots)
    }

    /// Removes snapshots issued before `cutoff`, except the one still current at `cutoff`
    /// (so replay from the start of the window shows the storm), plus leftover temp files,
    /// emptied storm dirs and their ended markers.
    pub fn prune(&self, cutoff: DateTime<Utc>) -> anyhow::Result<usize> {
        let mut removed = 0;
        for storm_dir in fs::read_dir(&self.dir)? {
            let storm_dir = storm_dir?.path();
            if !storm_dir.is_dir() {
                continue;
            }
            let mut old = Vec::new();
            for file in fs::read_dir(&storm_dir)? {
                let path = file?.path();
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                if name.ends_with(".tmp") {
                    fs::remove_file(&path)?;
                } else if let Some(t) = parse_snapshot_file_name(name).filter(|&t| t < cutoff) {
                    old.push((t, path));
                }
            }
            old.sort();
            if old.last().is_some_and(|(t, _)| cutoff - *t <= Duration::hours(STALE_AFTER_HOURS)) {
                old.pop();
            }
            for (_, path) in old {
                fs::remove_file(&path)?;
                removed += 1;
            }
            if fs::read_dir(&storm_dir)?.next().is_none() {
                fs::remove_dir(&storm_dir)?;
            }
        }
        let ids = self.storm_ids()?;
        let mut ended = self.ended.lock().unwrap();
        if ended.keys().any(|id| !ids.contains(id)) {
            ended.retain(|id, _| ids.contains(id));
            self.save_ended(&ended)?;
        }
        Ok(removed)
    }
}
