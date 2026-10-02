mod config;
mod ngrok_tunnel;
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
                  ticker as a web page. Connect OBS Browser Source to /scroll."
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

    /// Also share the ticker over the internet via an ngrok tunnel (overrides config).
    #[arg(long)]
    ngrok: bool,

    /// ngrok authtoken (overrides config / NGROK_AUTHTOKEN env var).
    #[arg(long)]
    ngrok_authtoken: Option<String>,
}

/// Loads KEY=VALUE pairs from a `.env` file in the current directory into the
/// process environment, without overriding variables already set (e.g. by the
/// shell). Missing file is not an error. No quoting/escaping support — kept
/// intentionally simple since it only needs to carry secrets like
/// NGROK_AUTHTOKEN that config files shouldn't hold in plain text.
fn load_dotenv() {
    let Ok(content) = std::fs::read_to_string(".env") else {
        return;
    };
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim().trim_matches('"');
            if std::env::var_os(key).is_none() {
                std::env::set_var(key, value);
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    load_dotenv();

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
    if args.ngrok {
        cfg.ngrok.enabled = true;
    }
    if let Some(authtoken) = args.ngrok_authtoken {
        cfg.ngrok.authtoken = authtoken;
    }

    server::run(cfg).await
}
