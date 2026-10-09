//! Desktop settings: built-in defaults, optionally overridden by ~/.config/rust-radar/rust-radar.toml.

use std::path::Path;

use anyhow::Context;
use rust_radar::config::Config;

pub fn load(config_file: &Path, data_dir: &Path, web_dir: &Path) -> anyhow::Result<Config> {
    let mut table = defaults(data_dir);
    match std::fs::read_to_string(config_file) {
        Ok(text) => {
            let file: toml::Table =
                text.parse().with_context(|| format!("parsing {}", config_file.display()))?;
            table.extend(file);
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).with_context(|| format!("reading {}", config_file.display())),
    }
    let mut config: Config = toml::Value::Table(table)
        .try_into()
        .with_context(|| format!("in {}", config_file.display()))?;
    config.port = 0;
    config.web_dir = web_dir.to_path_buf();
    config.validate().with_context(|| format!("in {}", config_file.display()))?;
    Ok(config)
}

/// Desktop defaults that differ from the server's; everything else comes from `Config::default()`.
fn defaults(data_dir: &Path) -> toml::Table {
    let mut t = toml::Table::new();
    t.insert("archive_dir".into(), data_dir.to_string_lossy().into_owned().into());
    t.insert("retention_hours".into(), 72.into());
    t.insert("backfill_hours".into(), 2.into());
    t.insert("decoded_cache_frames".into(), 10.into());
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct Dirs { _tmp: tempfile::TempDir, file: PathBuf, data: PathBuf, web: PathBuf }

    fn dirs(contents: Option<&str>) -> Dirs {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("rust-radar.toml");
        if let Some(c) = contents { std::fs::write(&file, c).unwrap(); }
        Dirs { data: tmp.path().join("data"), web: tmp.path().join("web"), file, _tmp: tmp }
    }

    #[test]
    fn defaults_without_a_file() {
        let d = dirs(None);
        let c = load(&d.file, &d.data, &d.web).unwrap();
        assert_eq!(c.port, 0);
        assert_eq!(c.archive_dir, d.data);
        assert_eq!(c.web_dir, d.web);
        assert_eq!(c.retention_hours, 72);
        assert_eq!(c.backfill_hours, 2);
        assert_eq!(c.decoded_cache_frames, 10);
        assert_eq!(c.mrms_poll_secs, Config::default().mrms_poll_secs);
        assert_eq!(c.basemap_style, Config::default().basemap_style);
    }

    #[test]
    fn file_values_override_defaults() {
        let d = dirs(Some("backfill_hours = 6\narchive_dir = \"/srv/radar\"\n"));
        let c = load(&d.file, &d.data, &d.web).unwrap();
        assert_eq!(c.backfill_hours, 6);
        assert_eq!(c.archive_dir, PathBuf::from("/srv/radar"));
        assert_eq!(c.decoded_cache_frames, 10);
    }

    #[test]
    fn port_and_web_dir_are_always_the_apps() {
        let d = dirs(Some("port = 9000\nweb_dir = \"/elsewhere\"\n"));
        let c = load(&d.file, &d.data, &d.web).unwrap();
        assert_eq!(c.port, 0);
        assert_eq!(c.web_dir, d.web);
    }

    #[test]
    fn unknown_key_is_an_error_naming_the_file() {
        let d = dirs(Some("backfil_hours = 6\n"));
        let err = format!("{:#}", load(&d.file, &d.data, &d.web).unwrap_err());
        assert!(err.contains("rust-radar.toml") && err.contains("backfil_hours"), "{err}");
    }

    #[test]
    fn malformed_toml_is_an_error_naming_the_file() {
        let d = dirs(Some("backfill_hours = \n"));
        let err = format!("{:#}", load(&d.file, &d.data, &d.web).unwrap_err());
        assert!(err.contains("rust-radar.toml"), "{err}");
    }

    #[test]
    fn invalid_values_are_rejected() {
        let d = dirs(Some("backfill_hours = 100\nretention_hours = 24\n"));
        assert!(load(&d.file, &d.data, &d.web).is_err());
    }
}
