use crate::catalog::CatalogStore;
use crate::config::Config;
use crate::contract;
use crate::ngrok_tunnel;
use crate::requests::{NewRequest, RequestStore, SubmitError, MAX_COMMENT_LEN, MAX_FIELD_LEN};
use crate::state::{fetch_state, KaraokeState};
use anyhow::Result;
use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Json, Response},
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use tower_http::compression::CompressionLayer;
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
    /// `None` when `[requests]` is disabled; otherwise the address kroak-time
    /// should poll, or why the request contract couldn't be loaded.
    pub request_server: Option<std::result::Result<String, String>>,
}

const LIST_HTML:   &str = include_str!("static/list.html");
const SCROLL_HTML: &str = include_str!("static/scroll.html");

fn render_scroll_html(scroll_cfg: &crate::config::ScrollConfig, ticker_cfg: &crate::config::TickerConfig) -> String {
    SCROLL_HTML
        .replace("__SCROLL_CFG__", &serde_json::to_string(scroll_cfg).expect("ScrollConfig is always serializable"))
        .replace("__TICKER_CFG__", &serde_json::to_string(ticker_cfg).expect("TickerConfig is always serializable"))
}

fn render_list_html(cfg: &crate::config::TickerConfig, requests_cfg: &crate::config::RequestsConfig) -> String {
    // Only what the page needs from [requests] — not file paths.
    let requests_page_cfg = serde_json::json!({
        "enabled": requests_cfg.enabled,
        "comment_field": requests_cfg.comment_field,
    });
    LIST_HTML
        .replace(
            "__TICKER_CFG__",
            &serde_json::to_string(cfg).expect("TickerConfig is always serializable"),
        )
        .replace("__REQUESTS_CFG__", &requests_page_cfg.to_string())
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

    // Song requests: the store, the catalog (loaded in the background so it's
    // warm before the first singer opens the form), and the contract-driven
    // server kroak-time polls. Like `server.*`, whether that server runs is
    // decided once at startup.
    let request_store = RequestStore::load();
    let catalog = CatalogStore::new();
    let request_server = if startup.requests.enabled {
        let warm = catalog.clone();
        let catalog_path = startup.requests.catalog_path.clone();
        tokio::task::spawn_blocking(move || warm.refresh(&catalog_path));

        let contract_path = crate::paths::resolve(&startup.requests.contract_path);
        match contract::ensure_contract_file(&contract_path).and_then(|_| contract::load(&contract_path)) {
            Ok(c) => {
                let addr = c.listen;
                let store = request_store.clone();
                tokio::spawn(async move {
                    if let Err(e) = contract::run_server(c, store).await {
                        tracing::error!("Request server stopped: {e:#}");
                    }
                });
                Some(Ok(format!("http://{addr}/  (contract: {})", contract_path.display())))
            }
            Err(e) => {
                tracing::error!("Request contract: {e:#}");
                Some(Err(format!("{e:#}")))
            }
        }
    } else {
        None
    };

    let list_cfg = cfg.clone();
    let scroll_cfg = cfg.clone();
    let catalog_cfg = cfg.clone();
    let submit_cfg = cfg.clone();

    let app = Router::new()
        .route(
            "/",
            get(move || {
                let cfg = list_cfg.clone();
                async move {
                    let cfg = cfg.read().unwrap();
                    Html(render_list_html(&cfg.ticker, &cfg.requests))
                }
            }),
        )
        .route(
            "/api/catalog",
            get(move |headers: HeaderMap| {
                let cfg = catalog_cfg.clone();
                let catalog = catalog.clone();
                async move { catalog_handler(cfg, catalog, headers).await }
            })
            .layer(CompressionLayer::new()),
        )
        .route(
            "/api/request",
            post(move |Json(body): Json<RequestBody>| {
                let cfg = submit_cfg.clone();
                let store = request_store.clone();
                async move { submit_handler(cfg, store, body) }
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
        request_server,
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
    match &info.request_server {
        Some(Ok(addr)) => {
            println!();
            println!("  Song requests for kroak-time:  {addr}");
        }
        Some(Err(err)) => {
            println!();
            println!("  ⚠ Song requests are enabled but the request contract failed: {err}");
        }
        None => {}
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

/// `GET /api/catalog`: the whole catalog as one compact JSON blob (see
/// catalog.rs), fetched by the singer page only after it has loaded. ETag +
/// `no-cache` means repeat visits revalidate with a cheap 304.
async fn catalog_handler(cfg: SharedConfig, catalog: CatalogStore, headers: HeaderMap) -> Response {
    let (enabled, path) = {
        let cfg = cfg.read().unwrap();
        (cfg.requests.enabled, cfg.requests.catalog_path.clone())
    };
    if !enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    // Cheap when nothing changed (one stat); reloads if the file did.
    let refresher = catalog.clone();
    let _ = tokio::task::spawn_blocking(move || refresher.refresh(&path)).await;

    let Some(served) = catalog.current() else {
        let msg = catalog.last_error().unwrap_or_else(|| "catalog not loaded".into());
        tracing::warn!("Catalog requested but unavailable: {msg}");
        // An empty catalog still lets singers type requests freehand.
        return Json(serde_json::json!({ "artists": [], "songs": [] })).into_response();
    };
    let not_modified = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|t| t.trim() == served.etag));
    let common = [(header::ETAG, served.etag.clone()), (header::CACHE_CONTROL, "no-cache".to_string())];
    if not_modified {
        return (StatusCode::NOT_MODIFIED, common).into_response();
    }
    (
        common,
        [(header::CONTENT_TYPE, "application/json")],
        served.json.clone(),
    )
        .into_response()
}

#[derive(Deserialize)]
struct RequestBody {
    #[serde(default)]
    singer: String,
    #[serde(default)]
    artist: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    key_change: i32,
    #[serde(default)]
    device_id: String,
    #[serde(default)]
    fingerprint: String,
    #[serde(default)]
    in_catalog: bool,
    #[serde(default)]
    comment: String,
}

fn clean(s: &str) -> String {
    clean_to(s, MAX_FIELD_LEN)
}

fn clean_to(s: &str, max_len: usize) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(max_len).collect()
}

/// `POST /api/request` from the singer page's request form.
fn submit_handler(cfg: SharedConfig, store: RequestStore, body: RequestBody) -> Response {
    let (enabled, max_pending, comment_field) = {
        let cfg = cfg.read().unwrap();
        (cfg.requests.enabled, cfg.requests.max_pending_per_device, cfg.requests.comment_field)
    };
    let reply = |status: StatusCode, v: serde_json::Value| (status, Json(v)).into_response();
    if !enabled {
        return reply(StatusCode::NOT_FOUND, serde_json::json!({ "ok": false, "error": "Song requests are switched off." }));
    }

    let singer = clean(&body.singer);
    let artist = clean(&body.artist);
    let title = clean(&body.title);
    if singer.is_empty() || title.is_empty() {
        return reply(StatusCode::BAD_REQUEST, serde_json::json!({ "ok": false, "error": "Please fill in your name and a song title." }));
    }
    let fingerprint: String = body.fingerprint.chars().filter(|c| c.is_ascii_alphanumeric()).take(64).collect();
    let mut device_id: String = body
        .device_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(64)
        .collect();
    if device_id.is_empty() {
        // No usable localStorage id (old browser / JS error) — fall back to
        // the fingerprint so the host still sees something stable.
        device_id = format!("fp-{fingerprint}");
    }

    let new = NewRequest {
        singer,
        artist,
        title,
        key_change: body.key_change.clamp(-6, 6),
        device_id,
        fingerprint,
        in_catalog: body.in_catalog,
        // Ignored while the form hides the box, so a stale page can't sneak one in.
        comment: if comment_field { clean_to(&body.comment, MAX_COMMENT_LEN) } else { String::new() },
    };
    match store.submit(new, max_pending) {
        Ok(r) => {
            tracing::info!(
                "Song request #{} from {} ({}): {} — {} (key {:+})",
                r.request_id, r.singer, r.device_code(), r.artist, r.title, r.key_change
            );
            reply(StatusCode::OK, serde_json::json!({ "ok": true, "request_id": r.request_id }))
        }
        Err(SubmitError::NotAccepting) => reply(
            StatusCode::SERVICE_UNAVAILABLE,
            serde_json::json!({ "ok": false, "error": "Requests are closed right now — please ask the host." }),
        ),
        Err(SubmitError::TooManyPending(n)) => reply(
            StatusCode::TOO_MANY_REQUESTS,
            serde_json::json!({ "ok": false, "error": format!("You already have {n} requests waiting — give the host a chance to add them first.") }),
        ),
    }
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
