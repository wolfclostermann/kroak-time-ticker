//! The ngrok tunnel, kept alive for as long as the ticker runs.
//!
//! The ngrok crate reconnects a dropped session by itself, but its retry
//! delay grows as 50ms, 2.5s, 125s, ~1.7h… — so a Wi-Fi drop of more than
//! a few seconds left the tunnel effectively dead for the rest of the night,
//! and nothing said so. This module takes over:
//!
//! - A custom connector retries the TCP/TLS connection itself, every few
//!   seconds, so the crate's own backoff never escalates while the network
//!   is down. The crate then rebinds the tunnel, keeping its URL.
//! - If the crate's reconnect fails past the connection step (auth, rebind),
//!   or the tunnel ends for any other reason, a supervisor opens a whole new
//!   session, again retrying every few seconds, forever.
//! - Heartbeats, the connector and the supervisor all report into a shared
//!   [`NgrokState`], which the TUI shows live and transitions get logged.

use crate::config::NgrokConfig;
use ngrok::{prelude::*, session::default_connect, tunnel::AcceptError};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, RwLock,
    },
    time::{Duration, Instant},
};
use url::Url;

/// ngrok rejects intervals under 10s.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
const HEARTBEAT_TOLERANCE: Duration = Duration::from_secs(15);
/// Retry delays: start quick (a blip), settle at a steady pace.
const RETRY_MIN: Duration = Duration::from_secs(1);
const RETRY_MAX: Duration = Duration::from_secs(10);
/// With no heartbeat for this long and no reconnect in progress, the
/// connection is silently hung (the OS never reported it dead): start over.
const STALL_LIMIT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq)]
pub enum NgrokState {
    Disabled,
    /// No tunnel yet — first start, or a fresh session after the last one
    /// ended. `error` is why the previous attempt failed.
    Connecting { error: Option<String> },
    Online { url: String },
    /// The tunnel at `url` lost its connection at `since` and is being
    /// brought back.
    Reconnecting { url: String, since: Instant, error: String },
}

pub type SharedNgrokState = Arc<RwLock<NgrokState>>;

/// Starts keeping the ngrok tunnel up in the background, if enabled in
/// config, forwarding to the local server. Returns its live state.
pub fn spawn(cfg: &NgrokConfig, forward_host: &str, port: u16) -> SharedNgrokState {
    if !cfg.enabled {
        return Arc::new(RwLock::new(NgrokState::Disabled));
    }
    let state = Arc::new(RwLock::new(NgrokState::Connecting { error: None }));
    let to_url = Url::parse(&format!("http://{forward_host}:{port}")).expect("forward URL is always valid");
    tokio::spawn(supervise(cfg.clone(), to_url, state.clone()));
    state
}

fn set(state: &SharedNgrokState, new: NgrokState) {
    let mut cur = state.write().unwrap();
    if *cur == new {
        return;
    }
    match &new {
        NgrokState::Online { url } => tracing::info!("ngrok: online at {url}"),
        NgrokState::Reconnecting { error, .. } => tracing::warn!("ngrok: connection lost, reconnecting ({error})"),
        NgrokState::Connecting { error: Some(e) } => tracing::warn!("ngrok: not connected ({e}), retrying"),
        _ => {}
    }
    *cur = new;
}

/// Marks the tunnel as down (keeping its URL if it had one), or just
/// updates why, without resetting how long it has been down.
fn mark_down(state: &SharedNgrokState, error: String) {
    let new = match &*state.read().unwrap() {
        NgrokState::Online { url } => NgrokState::Reconnecting { url: url.clone(), since: Instant::now(), error },
        NgrokState::Reconnecting { url, since, .. } => {
            NgrokState::Reconnecting { url: url.clone(), since: *since, error }
        }
        _ => NgrokState::Connecting { error: Some(error) },
    };
    set(state, new);
}

struct Backoff(Duration);

impl Backoff {
    fn new() -> Self {
        Backoff(RETRY_MIN)
    }
    fn next(&mut self) -> Duration {
        let d = self.0;
        self.0 = (self.0 * 2).min(RETRY_MAX);
        d
    }
}

/// Shared between one session's connector, heartbeat handler and the
/// supervisor.
struct Link {
    state: SharedNgrokState,
    /// Set when the supervisor abandons this session, so a connector still
    /// retrying for it stops.
    abandoned: AtomicBool,
    /// The connector is currently retrying a lost connection.
    retrying: AtomicBool,
    last_heartbeat: RwLock<Instant>,
}

async fn supervise(cfg: NgrokConfig, to_url: Url, state: SharedNgrokState) {
    let mut backoff = Backoff::new();
    loop {
        let link = Arc::new(Link {
            state: state.clone(),
            abandoned: AtomicBool::new(false),
            retrying: AtomicBool::new(false),
            last_heartbeat: RwLock::new(Instant::now()),
        });

        match open(&cfg, &to_url, &link).await {
            Ok((mut session, mut forwarder)) => {
                backoff = Backoff::new();
                set(&state, NgrokState::Online { url: forwarder.url().to_string() });

                let ended = tokio::select! {
                    res = forwarder.join() => match res {
                        Ok(Ok(())) => "tunnel closed".to_string(),
                        Ok(Err(e)) => format!("tunnel closed: {e}"),
                        Err(e) => format!("tunnel task failed: {e}"),
                    },
                    _ = stalled(&link) => "no response from ngrok".to_string(),
                };
                link.abandoned.store(true, Ordering::SeqCst);
                mark_down(&state, ended);
                drop(forwarder);
                // Closing talks to ngrok, which may be unreachable — don't
                // let it hold up the retry.
                let _ = tokio::time::timeout(Duration::from_secs(2), session.close()).await;
            }
            Err(e) => {
                link.abandoned.store(true, Ordering::SeqCst);
                mark_down(&state, e);
            }
        }
        tokio::time::sleep(backoff.next()).await;
    }
}

/// Resolves once heartbeats have been missing for [`STALL_LIMIT`] while no
/// reconnect is underway — a connection the OS hasn't noticed is dead.
async fn stalled(link: &Link) {
    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let quiet = link.last_heartbeat.read().unwrap().elapsed();
        if quiet >= STALL_LIMIT && !link.retrying.load(Ordering::SeqCst) {
            return;
        }
    }
}

async fn open(
    cfg: &NgrokConfig,
    to_url: &Url,
    link: &Arc<Link>,
) -> Result<(ngrok::Session, ngrok::forwarder::Forwarder<ngrok::tunnel::HttpTunnel>), String> {
    let mut builder = ngrok::Session::builder();
    if cfg.authtoken.is_empty() {
        builder.authtoken_from_env();
    } else {
        builder.authtoken(cfg.authtoken.clone());
    }
    builder
        .heartbeat_interval(HEARTBEAT_INTERVAL)
        .expect("valid heartbeat interval")
        .heartbeat_tolerance(HEARTBEAT_TOLERANCE)
        .expect("valid heartbeat tolerance");

    let hb_link = link.clone();
    builder.handle_heartbeat(move |latency: Option<Duration>| {
        let link = hb_link.clone();
        async move {
            match latency {
                Some(_) => {
                    *link.last_heartbeat.write().unwrap() = Instant::now();
                    let current = link.state.read().unwrap().clone();
                    if let NgrokState::Reconnecting { url, .. } = current {
                        set(&link.state, NgrokState::Online { url });
                    }
                }
                None if !link.abandoned.load(Ordering::SeqCst) => {
                    mark_down(&link.state, "no response from ngrok".to_string());
                }
                None => {}
            }
            Ok::<(), Box<dyn std::error::Error>>(())
        }
    });

    let conn_link = link.clone();
    builder.connector(
        move |host: String, port: u16, tls: Arc<rustls::ClientConfig>, err: Option<AcceptError>| {
            let link = conn_link.clone();
            async move { connect(&link, host, port, tls, err).await }
        },
    );

    let session = builder.connect().await.map_err(|e| format!("failed to connect session: {e}"))?;

    let mut endpoint = session.http_endpoint();
    if !cfg.domain.is_empty() {
        endpoint.domain(cfg.domain.clone());
    }
    let forwarder = endpoint
        .listen_and_forward(to_url.clone())
        .await
        .map_err(|e| format!("failed to start tunnel: {e}"))?;
    *link.last_heartbeat.write().unwrap() = Instant::now();
    Ok((session, forwarder))
}

/// Establishes the connection to ngrok. The first connect is a single try
/// (the supervisor retries). On a reconnect, retries here on our own steady
/// schedule until the network is back, so the crate's escalating backoff
/// never kicks in. If the crate's reconnect then fails anyway (after the
/// connection step), gives up so the supervisor starts a fresh session.
async fn connect(
    link: &Link,
    host: String,
    port: u16,
    tls: Arc<rustls::ClientConfig>,
    err: Option<AcceptError>,
) -> Result<Box<dyn ngrok::session::IoStream>, ngrok::session::ConnectError> {
    use ngrok::session::ConnectError;

    let Some(err) = err else {
        return default_connect(host, port, tls, None).await;
    };
    if link.abandoned.load(Ordering::SeqCst) {
        return Err(ConnectError::Canceled);
    }
    if let AcceptError::Reconnect(e) = &err {
        mark_down(&link.state, format!("reconnect failed: {e}"));
        return Err(ConnectError::Canceled);
    }

    mark_down(&link.state, format!("connection lost: {err}"));
    link.retrying.store(true, Ordering::SeqCst);
    let mut backoff = Backoff::new();
    let result = loop {
        if link.abandoned.load(Ordering::SeqCst) {
            break Err(ConnectError::Canceled);
        }
        match default_connect(host.clone(), port, tls.clone(), None).await {
            Ok(stream) => break Ok(stream),
            Err(e) => {
                mark_down(&link.state, e.to_string());
                tokio::time::sleep(backoff.next()).await;
            }
        }
    };
    link.retrying.store(false, Ordering::SeqCst);
    // Give the rebind a fresh heartbeat window before the stall check.
    *link.last_heartbeat.write().unwrap() = Instant::now();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_caps() {
        let mut b = Backoff::new();
        let delays: Vec<_> = (0..6).map(|_| b.next().as_secs()).collect();
        assert_eq!(delays, vec![1, 2, 4, 8, 10, 10]);
    }

    #[test]
    fn going_down_keeps_url_and_start_time() {
        let state: SharedNgrokState = Arc::new(RwLock::new(NgrokState::Online { url: "https://x".into() }));
        mark_down(&state, "a".into());
        let since = match &*state.read().unwrap() {
            NgrokState::Reconnecting { url, since, error } => {
                assert_eq!(url, "https://x");
                assert_eq!(error, "a");
                *since
            }
            s => panic!("unexpected {s:?}"),
        };
        mark_down(&state, "b".into());
        assert!(matches!(
            &*state.read().unwrap(),
            NgrokState::Reconnecting { since: s, error, .. } if *s == since && error == "b"
        ));
    }

    #[test]
    fn failing_before_any_tunnel_stays_connecting() {
        let state: SharedNgrokState = Arc::new(RwLock::new(NgrokState::Connecting { error: None }));
        mark_down(&state, "offline".into());
        assert_eq!(*state.read().unwrap(), NgrokState::Connecting { error: Some("offline".into()) });
    }
}
