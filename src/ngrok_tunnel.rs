use crate::config::NgrokConfig;
use ngrok::{forwarder::Forwarder, prelude::*, tunnel::HttpTunnel};
use url::Url;

/// A running ngrok tunnel. Keep this alive for as long as the tunnel should
/// stay open — dropping it tears the tunnel down.
pub struct NgrokTunnel {
    pub url: String,
    _forwarder: Forwarder<HttpTunnel>,
}

/// Result of attempting to open the ngrok tunnel: disabled in config, up and
/// running, or enabled but failed to connect/start (with the error message,
/// so callers — e.g. the TUI's status pane — can tell that apart from
/// "disabled" instead of just seeing no URL either way).
pub enum NgrokStatus {
    Disabled,
    Running(Box<NgrokTunnel>),
    Failed(String),
}

/// Opens a public ngrok tunnel that forwards to the local server, if enabled
/// in config. The local/LAN server keeps running regardless of the outcome.
pub async fn start(cfg: &NgrokConfig, forward_host: &str, port: u16) -> NgrokStatus {
    if !cfg.enabled {
        return NgrokStatus::Disabled;
    }

    let mut builder = ngrok::Session::builder();
    if cfg.authtoken.is_empty() {
        builder.authtoken_from_env();
    } else {
        builder.authtoken(cfg.authtoken.clone());
    }

    let session = match builder.connect().await {
        Ok(session) => session,
        Err(e) => {
            let msg = format!("failed to connect session: {e}");
            tracing::warn!("ngrok: {msg}");
            return NgrokStatus::Failed(msg);
        }
    };

    let mut endpoint = session.http_endpoint();
    if !cfg.domain.is_empty() {
        endpoint.domain(cfg.domain.clone());
    }

    let to_url = Url::parse(&format!("http://{forward_host}:{port}"))
        .expect("forward URL is always valid");

    match endpoint.listen_and_forward(to_url).await {
        Ok(forwarder) => {
            let url = forwarder.url().to_string();
            NgrokStatus::Running(Box::new(NgrokTunnel {
                url,
                _forwarder: forwarder,
            }))
        }
        Err(e) => {
            let msg = format!("failed to start tunnel: {e}");
            tracing::warn!("ngrok: {msg}");
            NgrokStatus::Failed(msg)
        }
    }
}
