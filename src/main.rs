use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use chrono::Utc;
use clap::Parser;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use rust_radar::config::{Cli, Config};
use rust_radar::fetch::{Fetcher, HttpFetcher};
use rust_radar::mrms_archive::MrmsArchive;
use rust_radar::nhc_archive::NhcArchive;
use rust_radar::server::{router, AppState};
use rust_radar::sources;
use rust_radar::status::Status;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();
    let config = Config::load(&Cli::parse())?;

    let mrms = Arc::new(MrmsArchive::open(config.archive_dir.join("mrms"), config.decoded_cache_frames)?);
    let nhc = Arc::new(NhcArchive::open(config.archive_dir.join("nhc"))?);
    let status = Status::default();
    let fetcher: Arc<dyn Fetcher> = Arc::new(HttpFetcher::new()?);

    tokio::spawn(sources::mrms::run(
        Arc::clone(&fetcher),
        Arc::clone(&mrms),
        status.clone(),
        Duration::from_secs(config.mrms_poll_secs),
    ));
    tokio::spawn(sources::nhc::run(
        Arc::clone(&fetcher),
        Arc::clone(&nhc),
        status.clone(),
        Duration::from_secs(config.nhc_poll_secs),
    ));
    {
        let (fetcher, mrms, hours) = (Arc::clone(&fetcher), Arc::clone(&mrms), config.backfill_hours);
        tokio::spawn(async move {
            match sources::mrms::backfill(fetcher.as_ref(), &mrms, Utc::now(), hours).await {
                Ok(n) => info!("backfilled {n} radar frames"),
                Err(e) => warn!("radar backfill failed: {e:#}"),
            }
        });
    }
    tokio::spawn(prune_loop(Arc::clone(&mrms), Arc::clone(&nhc), config.retention_hours));

    let state = Arc::new(AppState::new(mrms, nhc, status, config.basemap_style.clone()));
    let app = router(state, &config.web_dir);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", config.port))
        .await
        .with_context(|| format!("binding 127.0.0.1:{}", config.port))?;
    info!("rust-radar listening on http://127.0.0.1:{}", config.port);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn prune_loop(mrms: Arc<MrmsArchive>, nhc: Arc<NhcArchive>, retention_hours: u64) {
    let mut ticker = tokio::time::interval(Duration::from_secs(3600));
    loop {
        ticker.tick().await;
        let cutoff = Utc::now() - chrono::Duration::hours(retention_hours as i64);
        let (mrms, nhc) = (Arc::clone(&mrms), Arc::clone(&nhc));
        let result = tokio::task::spawn_blocking(move || -> anyhow::Result<(usize, usize)> {
            Ok((mrms.prune(cutoff)?, nhc.prune(cutoff)?))
        })
        .await;
        match result {
            Ok(Ok((frames, advisories))) if frames + advisories > 0 => {
                info!("pruned {frames} radar frames and {advisories} advisories")
            }
            Ok(Ok(_)) => {}
            Ok(Err(e)) => warn!("prune failed: {e:#}"),
            Err(e) => warn!("prune task panicked: {e}"),
        }
    }
}
