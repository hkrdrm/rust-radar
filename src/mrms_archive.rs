//! Rolling on-disk store of MRMS frames with an LRU of decoded grids.

use std::fs;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Context;
use chrono::{DateTime, NaiveDateTime, Utc};
use lru::LruCache;

use crate::grid::Grid;

const SUFFIX: &str = ".grib2.gz";
const TIME_FORMAT: &str = "%Y%m%d-%H%M%S";

pub fn frame_file_name(t: DateTime<Utc>) -> String {
    format!("{}{SUFFIX}", t.format(TIME_FORMAT))
}

fn parse_frame_file_name(name: &str) -> Option<DateTime<Utc>> {
    let stem = name.strip_suffix(SUFFIX)?;
    NaiveDateTime::parse_from_str(stem, TIME_FORMAT).ok().map(|n| n.and_utc())
}

pub struct MrmsArchive {
    dir: PathBuf,
    cache: Mutex<LruCache<DateTime<Utc>, Arc<Grid>>>,
}

impl MrmsArchive {
    pub fn open(dir: impl Into<PathBuf>, cache_frames: usize) -> anyhow::Result<MrmsArchive> {
        let dir = dir.into();
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        for entry in fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.extension().is_some_and(|e| e == "tmp") {
                fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
            }
        }
        let capacity = NonZeroUsize::new(cache_frames.max(1)).expect("max(1) is non-zero");
        Ok(MrmsArchive { dir, cache: Mutex::new(LruCache::new(capacity)) })
    }

    pub fn frames(&self) -> anyhow::Result<Vec<DateTime<Utc>>> {
        let mut frames = Vec::new();
        for entry in fs::read_dir(&self.dir).with_context(|| format!("listing {}", self.dir.display()))? {
            if let Some(t) = entry?.file_name().to_str().and_then(parse_frame_file_name) {
                frames.push(t);
            }
        }
        frames.sort();
        Ok(frames)
    }

    pub fn latest(&self) -> anyhow::Result<Option<DateTime<Utc>>> {
        Ok(self.frames()?.pop())
    }

    pub fn contains(&self, t: DateTime<Utc>) -> bool {
        self.dir.join(frame_file_name(t)).exists()
    }

    /// Validates by decoding, then writes atomically. Nothing is written if decoding fails.
    pub fn store(&self, t: DateTime<Utc>, gz: &[u8]) -> anyhow::Result<Arc<Grid>> {
        let grid = Arc::new(Grid::from_grib2_gz(gz).with_context(|| format!("validating frame {t}"))?);
        let path = self.dir.join(frame_file_name(t));
        let tmp = self.dir.join(format!("{}.tmp", frame_file_name(t)));
        fs::write(&tmp, gz).with_context(|| format!("writing {}", tmp.display()))?;
        fs::rename(&tmp, &path).with_context(|| format!("renaming to {}", path.display()))?;
        self.cache.lock().unwrap().put(t, Arc::clone(&grid));
        Ok(grid)
    }

    pub fn load(&self, t: DateTime<Utc>) -> anyhow::Result<Arc<Grid>> {
        if let Some(grid) = self.cache.lock().unwrap().get(&t) {
            return Ok(Arc::clone(grid));
        }
        let path = self.dir.join(frame_file_name(t));
        let bytes = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        // Decode outside the lock; two concurrent misses just decode twice.
        let grid = Arc::new(Grid::from_grib2_gz(&bytes).with_context(|| format!("decoding {}", path.display()))?);
        self.cache.lock().unwrap().put(t, Arc::clone(&grid));
        Ok(grid)
    }

    pub fn prune(&self, cutoff: DateTime<Utc>) -> anyhow::Result<usize> {
        let mut removed = 0;
        for t in self.frames()?.into_iter().filter(|&t| t < cutoff) {
            let path = self.dir.join(frame_file_name(t));
            fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
            self.cache.lock().unwrap().pop(&t);
            removed += 1;
        }
        Ok(removed)
    }
}
