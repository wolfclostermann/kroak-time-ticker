use crate::config::NgrokConfig;
use ngrok::{forwarder::Forwarder, prelude::*, tunnel::HttpTunnel};
use url::Url;

/// A running ngrok tunnel. Keep this alive for as long as the tunnel should
/// stay open — dropping it tears the tunnel down.
pub struct NgrokTunnel {
    pub url: String,
    _forwarder: Forwarder<HttpTunnel>,
}

/// Opens a public ngrok tunnel that forwards to the local server, if enabled
/// in config. Returns `None` (after logging a warning) if it's disabled or
/// fails to start — the local/LAN server keeps running either way.
pub async fn start(cfg: &NgrokConfig, forward_host: &str, port: u16) -> Option<NgrokTunnel> {
    if !cfg.enabled {
        return None;
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
            tracing::warn!("ngrok: failed to connect session: {e}");
            return None;
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
            Some(NgrokTunnel {
                url,
                _forwarder: forwarder,
            })
        }
        Err(e) => {
            tracing::warn!("ngrok: failed to start tunnel: {e}");
            None
        }
    }
}
