//! Starting the whole app: archives, pollers, backfill, pruning and the HTTP server.
//! Shared by the `rust-radar` server binary and the desktop app.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use chrono::Utc;
use tracing::{info, warn};

use crate::config::Config;
use crate::fetch::Fetcher;
use crate::mrms_archive::MrmsArchive;
use crate::nhc_archive::NhcArchive;
use crate::server::{router, AppState};
use crate::sources;
use crate::status::Status;

pub struct Running {
    /// The port actually bound (the OS picks one when the config says 0).
    pub port: u16,
    pub server: tokio::task::JoinHandle<std::io::Result<()>>,
}

/// Opens the archives, starts collection and serves the map. Must run inside a tokio runtime.
pub async fn start(config: &Config, fetcher: Arc<dyn Fetcher>) -> anyhow::Result<Running> {
    let mrms = Arc::new(MrmsArchive::open(config.archive_dir.join("mrms"), config.decoded_cache_frames)?);
    let nhc = Arc::new(NhcArchive::open(config.archive_dir.join("nhc"))?);
    let status = Status::default();

    tokio::spawn(sources::mrms::run(
        Arc::clone(&fetcher),
        Arc::clone(&mrms),
        status.clone(),
        Duration::from_secs(config.mrms_poll_secs),
        chrono::Duration::hours(config.backfill_hours as i64),
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
    let port = listener.local_addr().context("reading the bound address")?.port();
    info!("rust-radar listening on http://127.0.0.1:{port}");
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    Ok(Running { port, server })
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
