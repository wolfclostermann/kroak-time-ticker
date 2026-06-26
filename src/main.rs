mod config;
mod server;
mod state;

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "kroak-time-ticker",
    about = "kroak-time rotation ticker for OBS",
    long_about = "Polls a kroak-time /api/state endpoint and serves a live rotation \
                  ticker as a web page. Connect OBS Browser Source to /ticker or /scroll."
)]
struct Args {
    /// Path to the config file (created with defaults if it does not exist).
    #[arg(short, long, default_value = "kroak-time-ticker.toml")]
    config: PathBuf,

    /// Override the kroak-time upstream API URL.
    #[arg(long)]
    upstream_url: Option<String>,

    /// Override the HTTP server port.
    #[arg(short, long)]
    port: Option<u16>,

    /// How many singers to display in the ticker (overrides config).
    #[arg(long)]
    singer_count: Option<usize>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "kroak_time_ticker=debug".parse().unwrap()),
        )
        .init();

    let args = Args::parse();

    let mut cfg = config::Config::load_or_create(&args.config)?;

    // CLI flags override config file values.
    if let Some(url) = args.upstream_url {
        cfg.ticker.upstream_url = url;
    }
    if let Some(port) = args.port {
        cfg.server.port = port;
    }
    if let Some(count) = args.singer_count {
        cfg.ticker.singer_count = count;
    }

    server::run(cfg).await
}
