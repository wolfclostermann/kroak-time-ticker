mod config;
mod ngrok_tunnel;
mod server;
mod state;
mod tui;

use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tokio::sync::Notify;

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

    /// Run without the interactive config TUI — just the server and log
    /// output, as before. Use this for background/service use (systemd,
    /// launchd, etc.) where there's no interactive terminal.
    #[arg(long)]
    headless: bool,
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

    let args = Args::parse();

    let filter = || {
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "kroak_time_ticker=debug".parse().unwrap())
    };

    if args.headless {
        tracing_subscriber::fmt().with_env_filter(filter()).init();
    } else {
        // The TUI owns the terminal (alternate screen + raw mode) — send logs
        // to a file instead of stdout so a stray log line can't corrupt it.
        let log_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("kroak-time-ticker.log")
            .context("Failed to open kroak-time-ticker.log")?;
        tracing_subscriber::fmt()
            .with_env_filter(filter())
            .with_writer(std::sync::Mutex::new(log_file))
            .with_ansi(false)
            .init();
    }

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

    let shared_cfg = Arc::new(RwLock::new(cfg));
    let shutdown = Arc::new(Notify::new());

    if args.headless {
        let mut server_task = tokio::spawn(server::run(shared_cfg.clone(), shutdown.clone(), None));
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                shutdown.notify_one();
            }
            res = &mut server_task => {
                return res?;
            }
        }
        server_task.await??;
    } else {
        let (info_tx, info_rx) = std::sync::mpsc::channel();
        let server_task = tokio::spawn(server::run(shared_cfg.clone(), shutdown.clone(), Some(info_tx)));

        let tui_cfg = shared_cfg.clone();
        let tui_shutdown = shutdown.clone();
        let tui_path = args.config.clone();
        tokio::task::spawn_blocking(move || tui::run(tui_cfg, tui_shutdown, tui_path, info_rx))
            .await
            .context("TUI thread panicked")??;

        server_task.await??;
    }

    Ok(())
}
