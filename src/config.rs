use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,

    #[serde(default)]
    pub ticker: TickerConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerConfig {
    #[serde(default = "default_port")]
    pub port: u16,

    #[serde(default = "default_bind")]
    pub bind_address: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: default_port(),
            bind_address: default_bind(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TickerConfig {
    /// URL of the kroak-time /api/state endpoint to poll.
    #[serde(default = "default_upstream_url")]
    pub upstream_url: String,

    /// How many singers to show in the ticker (after current + next up).
    #[serde(default = "default_singer_count")]
    pub singer_count: usize,

    /// How often to poll the upstream API (milliseconds).
    #[serde(default = "default_poll_interval")]
    pub poll_interval_ms: u64,
}

impl Default for TickerConfig {
    fn default() -> Self {
        Self {
            upstream_url: default_upstream_url(),
            singer_count: default_singer_count(),
            poll_interval_ms: default_poll_interval(),
        }
    }
}

fn default_port() -> u16 {
    8080
}
fn default_bind() -> String {
    "0.0.0.0".to_string()
}
fn default_upstream_url() -> String {
    "http://localhost:7070/api/state".to_string()
}
fn default_singer_count() -> usize {
    8
}
fn default_poll_interval() -> u64 {
    1500
}

impl Config {
    pub fn load_or_create(path: &Path) -> Result<Self> {
        if path.exists() {
            let content = std::fs::read_to_string(path)
                .with_context(|| format!("Failed to read config: {}", path.display()))?;
            toml::from_str(&content)
                .with_context(|| format!("Failed to parse config: {}", path.display()))
        } else {
            tracing::info!(
                "No config file at {}, creating with defaults",
                path.display()
            );
            let cfg = Config {
                server: ServerConfig::default(),
                ticker: TickerConfig::default(),
            };
            cfg.save(path)?;
            Ok(cfg)
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let body = toml::to_string_pretty(self).context("Failed to serialize config")?;
        let content = format!(
            "# kroak-time-ticker configuration\n\
             # Set upstream_url to point at your kroak-time /api/state endpoint.\n\n{}",
            body
        );
        std::fs::write(path, content)
            .with_context(|| format!("Failed to write config: {}", path.display()))
    }
}
