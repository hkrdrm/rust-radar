//! Runtime configuration: `rust-radar.toml`, overridden by CLI flags.

use std::path::PathBuf;

use anyhow::{ensure, Context};
use clap::Parser;
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub port: u16,
    pub archive_dir: PathBuf,
    pub web_dir: PathBuf,
    pub retention_hours: u64,
    pub backfill_hours: u64,
    pub mrms_poll_secs: u64,
    pub nhc_poll_secs: u64,
    pub decoded_cache_frames: usize,
    pub basemap_style: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            port: 8080,
            archive_dir: PathBuf::from("./archive"),
            web_dir: PathBuf::from("./web"),
            retention_hours: 72,
            backfill_hours: 6,
            mrms_poll_secs: 60,
            nhc_poll_secs: 300,
            decoded_cache_frames: 10,
            basemap_style: "https://tiles.openfreemap.org/styles/dark".to_string(),
        }
    }
}

#[derive(Debug, Parser)]
#[command(name = "rust-radar", about = "Local hurricane-tracking radar map")]
pub struct Cli {
    /// Path to the TOML config file (missing file = defaults)
    #[arg(long, default_value = "rust-radar.toml")]
    pub config: PathBuf,
    #[arg(long)]
    pub port: Option<u16>,
    #[arg(long)]
    pub archive_dir: Option<PathBuf>,
    #[arg(long)]
    pub web_dir: Option<PathBuf>,
    #[arg(long)]
    pub retention_hours: Option<u64>,
    #[arg(long)]
    pub backfill_hours: Option<u64>,
    #[arg(long)]
    pub mrms_poll_secs: Option<u64>,
    #[arg(long)]
    pub nhc_poll_secs: Option<u64>,
    #[arg(long)]
    pub decoded_cache_frames: Option<usize>,
    #[arg(long)]
    pub basemap_style: Option<String>,
}

impl Config {
    pub fn load(cli: &Cli) -> anyhow::Result<Config> {
        let mut config = match std::fs::read_to_string(&cli.config) {
            Ok(text) => toml::from_str(&text)
                .with_context(|| format!("parsing {}", cli.config.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Config::default(),
            Err(e) => {
                return Err(e).with_context(|| format!("reading {}", cli.config.display()))
            }
        };
        if let Some(v) = cli.port { config.port = v; }
        if let Some(v) = &cli.archive_dir { config.archive_dir = v.clone(); }
        if let Some(v) = &cli.web_dir { config.web_dir = v.clone(); }
        if let Some(v) = cli.retention_hours { config.retention_hours = v; }
        if let Some(v) = cli.backfill_hours { config.backfill_hours = v; }
        if let Some(v) = cli.mrms_poll_secs { config.mrms_poll_secs = v; }
        if let Some(v) = cli.nhc_poll_secs { config.nhc_poll_secs = v; }
        if let Some(v) = cli.decoded_cache_frames { config.decoded_cache_frames = v; }
        if let Some(v) = &cli.basemap_style { config.basemap_style = v.clone(); }
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        ensure!(self.mrms_poll_secs > 0, "mrms_poll_secs must be > 0");
        ensure!(self.nhc_poll_secs > 0, "nhc_poll_secs must be > 0");
        ensure!(self.decoded_cache_frames > 0, "decoded_cache_frames must be > 0");
        ensure!(self.retention_hours > 0, "retention_hours must be > 0");
        ensure!(
            self.backfill_hours <= self.retention_hours,
            "backfill_hours ({}) must not exceed retention_hours ({})",
            self.backfill_hours,
            self.retention_hours
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn cli(args: &[&str]) -> Cli {
        Cli::parse_from(std::iter::once("rust-radar").chain(args.iter().copied()))
    }

    #[test]
    fn defaults_when_file_missing() {
        let c = Config::load(&cli(&["--config", "/nonexistent/rust-radar.toml"])).unwrap();
        assert_eq!(c, Config::default());
        assert_eq!(c.port, 8080);
        assert_eq!(c.archive_dir, std::path::PathBuf::from("./archive"));
        assert_eq!(c.retention_hours, 72);
        assert_eq!(c.backfill_hours, 6);
        assert_eq!(c.mrms_poll_secs, 60);
        assert_eq!(c.nhc_poll_secs, 300);
        assert_eq!(c.decoded_cache_frames, 10);
        assert_eq!(c.basemap_style, "https://tiles.openfreemap.org/styles/dark");
    }

    #[test]
    fn file_values_then_cli_overrides() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.toml");
        std::fs::write(&path, "port = 9000\nretention_hours = 24\n").unwrap();
        let c = Config::load(&cli(&[
            "--config",
            path.to_str().unwrap(),
            "--retention-hours",
            "48",
        ]))
        .unwrap();
        assert_eq!(c.port, 9000);
        assert_eq!(c.retention_hours, 48);
        assert_eq!(c.nhc_poll_secs, 300);
    }

    #[test]
    fn rejects_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.toml");
        std::fs::write(&path, "prot = 9000\n").unwrap();
        assert!(Config::load(&cli(&["--config", path.to_str().unwrap()])).is_err());
    }

    #[test]
    fn production_config_is_lean_and_valid() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/deploy/rust-radar.prod.toml");
        assert!(std::path::Path::new(path).exists(), "{path} is missing");
        let c = Config::load(&cli(&["--config", path])).unwrap();
        assert_eq!(c.port, 8090);
        assert_eq!(c.archive_dir, std::path::PathBuf::from("/var/lib/rust-radar"));
        assert_eq!(c.web_dir, std::path::PathBuf::from("/opt/rust-radar/web"));
        assert_eq!(c.decoded_cache_frames, 2);
        assert_eq!(c.backfill_hours, 2);
        assert_eq!(c.retention_hours, 72);
    }

    #[test]
    fn rejects_invalid_values() {
        let missing = "/nonexistent/rust-radar.toml";
        assert!(Config::load(&cli(&["--config", missing, "--decoded-cache-frames", "0"])).is_err());
        assert!(Config::load(&cli(&["--config", missing, "--mrms-poll-secs", "0"])).is_err());
        assert!(Config::load(&cli(&[
            "--config", missing, "--backfill-hours", "100", "--retention-hours", "24",
        ]))
        .is_err());
    }
}
