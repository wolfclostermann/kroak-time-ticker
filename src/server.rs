use crate::config::Config;
use crate::ngrok_tunnel;
use crate::state::{fetch_state, KaraokeState};
use anyhow::Result;
use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Json},
    routing::get,
    Router,
};
use std::{
    net::SocketAddr,
    sync::{Arc, RwLock},
};
use tokio::sync::Notify;
use tokio::time::Duration;
use tower_http::cors::CorsLayer;

pub type SharedConfig = Arc<RwLock<Config>>;
type SharedState = Arc<RwLock<Option<KaraokeState>>>;

/// URLs resolved once at startup (local IP lookup, ngrok tunnel). Either
/// printed directly (headless) or handed to the TUI to show in its status
/// pane, since the TUI owns the terminal and raw `println!`/log output
/// would corrupt its display.
#[derive(Debug, Clone)]
pub struct StartupInfo {
    pub upstream_url: String,
    pub local_dashboard_url: String,
    pub local_scroll_url: String,
    pub local_api_url: String,
    pub lan_dashboard_url: String,
    pub lan_scroll_url: String,
    pub ngrok_dashboard_url: Option<String>,
    pub ngrok_scroll_url: Option<String>,
    /// Set when `[ngrok]` is enabled but the tunnel failed to connect/start —
    /// distinct from the `ngrok_*_url` fields just being `None` because
    /// ngrok is disabled.
    pub ngrok_error: Option<String>,
}

const LIST_HTML:   &str = include_str!("static/list.html");
const SCROLL_HTML: &str = include_str!("static/scroll.html");

fn render_scroll_html(scroll_cfg: &crate::config::ScrollConfig, ticker_cfg: &crate::config::TickerConfig) -> String {
    SCROLL_HTML
        .replace("__SCROLL_CFG__", &serde_json::to_string(scroll_cfg).expect("ScrollConfig is always serializable"))
        .replace("__TICKER_CFG__", &serde_json::to_string(ticker_cfg).expect("TickerConfig is always serializable"))
}

fn render_list_html(cfg: &crate::config::TickerConfig) -> String {
    LIST_HTML.replace(
        "__TICKER_CFG__",
        &serde_json::to_string(cfg).expect("TickerConfig is always serializable"),
    )
}

/// Runs the HTTP server against a shared, live config: pages and the poll
/// loop re-read `cfg` on every request/tick, so edits made through the TUI
/// (which writes into the same `Arc<RwLock<Config>>`) take effect on the
/// next page load or poll without a restart. `server.*` and `[ngrok]` are
/// the exception — the listener and tunnel are only ever bound once, from
/// the config snapshot at startup.
pub async fn run(
    cfg: SharedConfig,
    shutdown: Arc<Notify>,
    startup_info_tx: Option<std::sync::mpsc::Sender<StartupInfo>>,
) -> Result<()> {
    let shared: SharedState = Arc::new(RwLock::new(None));
    let startup = cfg.read().unwrap().clone();

    // Background task: poll the upstream kroak-time API, re-reading the
    // upstream URL and interval from the shared config every cycle.
    let poll_shared = shared.clone();
    let poll_cfg = cfg.clone();

    tokio::spawn(async move {
        loop {
            let (upstream_url, interval_ms) = {
                let cfg = poll_cfg.read().unwrap();
                (cfg.ticker.upstream_url.clone(), cfg.ticker.poll_interval_ms)
            };
            match fetch_state(&upstream_url).await {
                Ok(state) => {
                    *poll_shared.write().unwrap() = Some(state);
                }
                Err(e) => {
                    tracing::warn!("Upstream fetch error: {}", e);
                }
            }
            tokio::time::sleep(Duration::from_millis(interval_ms)).await;
        }
    });

    let list_cfg = cfg.clone();
    let scroll_cfg = cfg.clone();

    let app = Router::new()
        .route(
            "/",
            get(move || {
                let cfg = list_cfg.clone();
                async move {
                    let cfg = cfg.read().unwrap();
                    Html(render_list_html(&cfg.ticker))
                }
            }),
        )
        .route(
            "/scroll",
            get(move || {
                let cfg = scroll_cfg.clone();
                async move {
                    let cfg = cfg.read().unwrap();
                    Html(render_scroll_html(&cfg.scroll, &cfg.ticker))
                }
            }),
        )
        .route("/api/state", get(api_state_handler))
        .layer(CorsLayer::permissive())
        .with_state(shared);

    let addr: SocketAddr = format!("{}:{}", startup.server.bind_address, startup.server.port).parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;

    let forward_host = if startup.server.bind_address == "0.0.0.0" {
        "127.0.0.1"
    } else {
        &startup.server.bind_address
    };
    let ngrok_status = ngrok_tunnel::start(&startup.ngrok, forward_host, startup.server.port).await;
    let (ngrok_dashboard_url, ngrok_scroll_url, ngrok_error) = match &ngrok_status {
        ngrok_tunnel::NgrokStatus::Disabled => (None, None, None),
        ngrok_tunnel::NgrokStatus::Running(t) => (Some(format!("{}/", t.url)), Some(format!("{}/scroll", t.url)), None),
        ngrok_tunnel::NgrokStatus::Failed(e) => (None, None, Some(e.clone())),
    };

    let local_ip = get_local_ip().unwrap_or_else(|| "<your-machine-ip>".to_string());
    let port = startup.server.port;
    let info = StartupInfo {
        upstream_url: startup.ticker.upstream_url.clone(),
        local_dashboard_url: format!("http://localhost:{port}/"),
        local_scroll_url: format!("http://localhost:{port}/scroll"),
        local_api_url: format!("http://localhost:{port}/api/state"),
        lan_dashboard_url: format!("http://{local_ip}:{port}/"),
        lan_scroll_url: format!("http://{local_ip}:{port}/scroll"),
        ngrok_dashboard_url,
        ngrok_scroll_url,
        ngrok_error,
    };

    match startup_info_tx {
        Some(tx) => {
            let _ = tx.send(info);
        }
        None => print_startup_info(&info),
    }

    axum::serve(listener, app)
        .with_graceful_shutdown(async move { shutdown.notified().await })
        .await?;

    Ok(())
}

fn print_startup_info(info: &StartupInfo) {
    println!();
    println!("kroak-time-ticker is running");
    println!("────────────────────────────────────────────────");
    println!("  Upstream  :  {}", info.upstream_url);
    println!("  Dashboard :  {}", info.local_dashboard_url);
    println!("  OBS Scroll:  {}  (1920×1080)", info.local_scroll_url);
    println!("  JSON API  :  {}", info.local_api_url);
    println!();
    println!("  From other machines on your network:");
    println!("  Dashboard :  {}", info.lan_dashboard_url);
    println!("  OBS Scroll:  {}", info.lan_scroll_url);
    if let Some(ngrok_url) = &info.ngrok_dashboard_url {
        println!();
        println!("  From anywhere via ngrok:");
        println!("  Dashboard :  {ngrok_url}");
        println!("  OBS Scroll:  {}", info.ngrok_scroll_url.as_deref().unwrap_or(""));
    } else if let Some(err) = &info.ngrok_error {
        println!();
        println!("  ⚠ ngrok is enabled but could not connect: {err}");
    }
    println!("────────────────────────────────────────────────");
    println!("Press Ctrl+C to stop.");
    println!();
}

fn get_local_ip() -> Option<String> {
    use std::net::UdpSocket;
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("8.8.8.8:80").ok()?;
    sock.local_addr().ok().map(|a| a.ip().to_string())
}

async fn api_state_handler(State(state): State<SharedState>) -> impl IntoResponse {
    match state.read().unwrap().clone() {
        Some(s) => Json(s).into_response(),
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "status": "not_ready",
                "message": "Upstream not yet reachable"
            })),
        )
            .into_response(),
    }
}
