use std::sync::Arc;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use rust_radar::app;
use rust_radar::config::{Cli, Config};
use rust_radar::fetch::HttpFetcher;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();
    let config = Config::load(&Cli::parse())?;
    let running = app::start(&config, Arc::new(HttpFetcher::new()?)).await?;
    running.server.await??;
    Ok(())
}
