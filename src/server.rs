use crate::config::Config;
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
use tokio::time::{interval, Duration};
use tower_http::cors::CorsLayer;

type SharedState = Arc<RwLock<Option<KaraokeState>>>;

const TICKER_HTML: &str = include_str!("static/ticker.html");
const LIST_HTML:   &str = include_str!("static/list.html");
const SCROLL_HTML: &str = include_str!("static/scroll.html");

fn render_scroll_html(cfg: &crate::config::ScrollConfig) -> String {
    SCROLL_HTML.replace(
        "__SCROLL_CFG__",
        &serde_json::to_string(cfg).expect("ScrollConfig is always serializable"),
    )
}

pub async fn run(cfg: Config) -> Result<()> {
    let shared: SharedState = Arc::new(RwLock::new(None));

    // Background task: poll the upstream kroak-time API on a fixed interval.
    let poll_shared = shared.clone();
    let poll_interval_ms = cfg.ticker.poll_interval_ms;
    let upstream_url = cfg.ticker.upstream_url.clone();

    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_millis(poll_interval_ms));
        loop {
            ticker.tick().await;
            match fetch_state(&upstream_url).await {
                Ok(state) => {
                    *poll_shared.write().unwrap() = Some(state);
                }
                Err(e) => {
                    tracing::warn!("Upstream fetch error: {}", e);
                }
            }
        }
    });

    // Pre-render scroll HTML once with the config baked in.
    let scroll_html = Arc::new(render_scroll_html(&cfg.scroll));
    let scroll_html_route = scroll_html.clone();

    let app = Router::new()
        .route("/", get(list_handler))
        .route("/ticker", get(ticker_handler))
        .route("/scroll", get(move || {
            let html = scroll_html_route.clone();
            async move { Html((*html).clone()) }
        }))
        .route("/api/state", get(api_state_handler))
        .layer(CorsLayer::permissive())
        .with_state(shared);

    let addr: SocketAddr = format!("{}:{}", cfg.server.bind_address, cfg.server.port).parse()?;

    print_startup_info(cfg.server.port, &cfg.ticker.upstream_url);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

fn print_startup_info(port: u16, upstream_url: &str) {
    let local_ip = get_local_ip().unwrap_or_else(|| "<your-machine-ip>".to_string());

    println!();
    println!("kroak-time-ticker is running");
    println!("────────────────────────────────────────────────");
    println!("  Upstream  :  {upstream_url}");
    println!("  Dashboard :  http://localhost:{port}/");
    println!("  OBS Ticker:  http://localhost:{port}/ticker");
    println!("  OBS Scroll:  http://localhost:{port}/scroll  (1920×1080)");
    println!("  JSON API  :  http://localhost:{port}/api/state");
    println!();
    println!("  From other machines on your network:");
    println!("  Dashboard :  http://{local_ip}:{port}/");
    println!("  OBS Ticker:  http://{local_ip}:{port}/ticker");
    println!("  OBS Scroll:  http://{local_ip}:{port}/scroll");
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

async fn ticker_handler() -> impl IntoResponse {
    Html(TICKER_HTML)
}

async fn list_handler() -> impl IntoResponse {
    Html(LIST_HTML)
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
