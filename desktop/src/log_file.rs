//! The desktop app has no terminal, so logs go to a file in the data directory.

use std::fs::{File, OpenOptions};
use std::path::Path;
use std::sync::Mutex;

use anyhow::Context;
use tracing_subscriber::EnvFilter;

pub const MAX_LOG_BYTES: u64 = 10 * 1024 * 1024;

/// Opens the log for appending, emptying it first if it has grown past `max_bytes`.
pub fn open_log(path: &Path, max_bytes: u64) -> std::io::Result<File> {
    let too_big = std::fs::metadata(path).is_ok_and(|m| m.len() > max_bytes);
    if too_big {
        File::create(path)?; // truncate
    }
    OpenOptions::new().create(true).append(true).open(path)
}

/// Sends `tracing` output (RUST_LOG, default info) to `path`.
pub fn init(path: &Path) -> anyhow::Result<()> {
    let file = open_log(path, MAX_LOG_BYTES).with_context(|| format!("opening {}", path.display()))?;
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with_ansi(false)
        .with_writer(Mutex::new(file))
        .try_init()
        .map_err(|e| anyhow::anyhow!("starting logging: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn open_log_truncates_only_when_over_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rust-radar.log");
        std::fs::write(&path, vec![b'x'; 100]).unwrap();
        let mut f = open_log(&path, 1000).unwrap();
        f.write_all(b"more").unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 104, "small log kept and appended");
        std::fs::write(&path, vec![b'x'; 1001]).unwrap();
        open_log(&path, 1000).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0, "big log emptied");
    }

    #[test]
    fn open_log_creates_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rust-radar.log");
        open_log(&path, 1000).unwrap();
        assert!(path.exists());
    }
}
