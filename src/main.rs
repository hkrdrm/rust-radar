use clap::Parser;
use rust_radar::config::{Cli, Config};

fn main() -> anyhow::Result<()> {
    let config = Config::load(&Cli::parse())?;
    println!("{config:#?}");
    Ok(())
}
