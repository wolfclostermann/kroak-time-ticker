use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,

    #[serde(default)]
    pub ticker: TickerConfig,

    #[serde(default)]
    pub scroll: ScrollConfig,

    #[serde(default)]
    pub ngrok: NgrokConfig,
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
    ///
    /// This should normally be configured in kroak-time itself (the
    /// `api_ticker_singer_count` setting), which reports it via `/api/state`.
    /// This field is a local override that only takes effect when > 0 — set it
    /// here only if you want THIS ticker instance to show fewer singers than
    /// kroak-time is configured to report (e.g. a smaller display). Default 0
    /// means "no override, use whatever kroak-time reports" (0 upstream = unlimited).
    #[serde(default = "default_singer_count")]
    pub singer_count: usize,

    /// When true, ignore any cap on how many singers to show — both `singer_count`
    /// above and kroak-time's own reported cap — and always show every singer
    /// currently in the rotation. Default false (respect the cap).
    #[serde(default)]
    pub show_all_singers: bool,

    /// How often to poll the upstream API (milliseconds).
    #[serde(default = "default_poll_interval")]
    pub poll_interval_ms: u64,

    /// When true, singers with no song queued are shown in the rotation.
    /// Default false: singers without a queued song are hidden.
    #[serde(default)]
    pub show_empty_singers: bool,

    /// "NEXT" text shown when there is no one in the rotation.
    #[serde(default = "default_empty_next_text")]
    pub empty_next_text: String,

    /// "THEN" entries shown when there is no one in the rotation.
    #[serde(default = "default_empty_then_text")]
    pub empty_then_text: Vec<String>,

    /// Singer-count / queue-time banner: when and how to show it.
    #[serde(default)]
    pub queue_info: QueueInfoConfig,
}

impl Default for TickerConfig {
    fn default() -> Self {
        Self {
            upstream_url: default_upstream_url(),
            singer_count: default_singer_count(),
            show_all_singers: false,
            poll_interval_ms: default_poll_interval(),
            show_empty_singers: false,
            empty_next_text: default_empty_next_text(),
            empty_then_text: default_empty_then_text(),
            queue_info: QueueInfoConfig::default(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct QueueInfoConfig {
    /// Show the singer-count / queue-time banner as soon as the ticker loads.
    #[serde(default = "default_true")]
    pub show_at_start: bool,

    /// Re-show the banner after this many singers have taken their turn.
    /// 0 disables the recurring banner (it will still show at start if enabled above).
    #[serde(default = "default_show_every_n_singers")]
    pub show_every_n_singers: u32,

    /// true = precise duration ("2 hours 1 minute"). false = friendly rounded duration
    /// ("about an hour and a half"), rounded to 15-minute increments.
    #[serde(default)]
    pub precise_duration: bool,

    /// Legend template for the singer count. `{count}` is replaced with the number.
    #[serde(default = "default_count_legend")]
    pub count_legend: String,

    /// Legend template for the queue time. `{time}` is replaced with the formatted duration.
    #[serde(default = "default_time_legend")]
    pub time_legend: String,
}

impl Default for QueueInfoConfig {
    fn default() -> Self {
        Self {
            show_at_start: default_true(),
            show_every_n_singers: default_show_every_n_singers(),
            precise_duration: false,
            count_legend: default_count_legend(),
            time_legend: default_time_legend(),
        }
    }
}

fn default_true() -> bool {
    true
}
fn default_show_every_n_singers() -> u32 {
    8
}
fn default_count_legend() -> String {
    "{count} singers in the queue".to_string()
}
fn default_time_legend() -> String {
    "{time} to get through the queue".to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ScrollConfig {
    /// Banner height in pixels.
    #[serde(default = "default_scroll_height")]
    pub height: u32,

    /// Banner background — any CSS color value.
    #[serde(default = "default_scroll_bg")]
    pub bg: String,

    /// Font family.
    #[serde(default = "default_scroll_font")]
    pub font: String,

    /// Font size in pixels.
    #[serde(default = "default_scroll_size")]
    pub size: u32,

    /// Scroll speed in pixels per second.
    #[serde(default = "default_scroll_speed")]
    pub speed: u32,

    #[serde(default = "default_color_now")]
    pub color_now: String,

    #[serde(default = "default_color_next")]
    pub color_next: String,

    #[serde(default = "default_color_up")]
    pub color_up: String,

    #[serde(default = "default_color_singer")]
    pub color_singer: String,

    #[serde(default = "default_color_song")]
    pub color_song: String,

    #[serde(default = "default_color_artist")]
    pub color_artist: String,

    /// Color of the "QUEUE" label on the singer-count / queue-time banner.
    #[serde(default = "default_color_info")]
    pub color_info: String,
}

impl Default for ScrollConfig {
    fn default() -> Self {
        Self {
            height:       default_scroll_height(),
            bg:           default_scroll_bg(),
            font:         default_scroll_font(),
            size:         default_scroll_size(),
            speed:        default_scroll_speed(),
            color_now:    default_color_now(),
            color_next:   default_color_next(),
            color_up:     default_color_up(),
            color_singer: default_color_singer(),
            color_song:   default_color_song(),
            color_artist: default_color_artist(),
            color_info:   default_color_info(),
        }
    }
}

fn default_scroll_height() -> u32  { 80 }
fn default_scroll_bg()     -> String { "rgba(8,8,12,0.90)".into() }
fn default_scroll_font()   -> String { "Segoe UI, Helvetica Neue, Arial, sans-serif".into() }
fn default_scroll_size()   -> u32  { 32 }
fn default_scroll_speed()  -> u32  { 120 }
fn default_color_now()     -> String { "#ffd44f".into() }
fn default_color_next()    -> String { "#5bc8ff".into() }
fn default_color_up()      -> String { "#aaa".into() }
fn default_color_singer()  -> String { "#fff".into() }
fn default_color_song()    -> String { "#ddd".into() }
fn default_color_artist()  -> String { "#aaa".into() }
fn default_color_info()    -> String { "#7cfc8a".into() }

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct NgrokConfig {
    /// Share the ticker over the internet via an ngrok tunnel, in addition to
    /// serving it locally / on the LAN. Off by default.
    #[serde(default)]
    pub enabled: bool,

    /// ngrok authtoken. Leave blank to fall back to the NGROK_AUTHTOKEN
    /// environment variable. Get one at:
    /// https://dashboard.ngrok.com/get-started/your-authtoken
    #[serde(default)]
    pub authtoken: String,

    /// Reserved ngrok domain to use (e.g. "my-ticker.ngrok-free.app"), for
    /// accounts with a static domain. Leave blank for a random ngrok-assigned
    /// URL each time the ticker starts.
    #[serde(default)]
    pub domain: String,
}

impl Default for NgrokConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            authtoken: String::new(),
            domain: String::new(),
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
    "http://localhost:8765/api/state".to_string()
}
fn default_singer_count() -> usize {
    0
}
fn default_poll_interval() -> u64 {
    1500
}
fn default_empty_next_text() -> String {
    "Your Name Could Be Here".to_string()
}
fn default_empty_then_text() -> Vec<String> {
    vec![
        "Just scan the QR code".to_string(),
        "Or Fill Out A Slip".to_string(),
    ]
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
                scroll: ScrollConfig::default(),
                ngrok: NgrokConfig::default(),
            };
            cfg.save(path)?;
            Ok(cfg)
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let body = toml::to_string_pretty(self).context("Failed to serialize config")?;
        let content = format!("{CONFIG_HEADER}\n{body}");
        std::fs::write(path, content)
            .with_context(|| format!("Failed to write config: {}", path.display()))
    }
}

/// Reference documentation for every config key, written at the top of the file.
/// Keep in sync with the `#[serde(default = ...)]` values on the structs above.
const CONFIG_HEADER: &str = r##"# kroak-time-ticker configuration
#
# Reference of every option below (all commented out here — the active values,
# which may differ from these defaults, are set further down in this file).
#
# [server]
# port = 8080                # HTTP port this ticker listens on (/, /ticker, /scroll, /api/state).
# bind_address = "0.0.0.0"   # Interface to bind. Use "127.0.0.1" to restrict to localhost only.
#
# [ticker]
# upstream_url = "http://localhost:8765/api/state"   # kroak-time's /api/state endpoint to poll.
# poll_interval_ms = 1500    # How often to poll the upstream API, in milliseconds.
# singer_count = 0           # Local cap on how many upcoming singers to show, beyond NOW/NEXT.
#                             # 0 = no override, use whatever kroak-time reports via its own
#                             # `api_ticker_singer_count` setting (which itself defaults to
#                             # unlimited). Set >0 here to show fewer than kroak-time reports.
# show_all_singers = false   # When true, ignore singer_count above AND kroak-time's reported
#                             # cap entirely, and always show every singer in the rotation.
# show_empty_singers = false # When true, singers with no song queued are still shown in the
#                             # THEN list. Default false hides them.
# empty_next_text = "Your Name Could Be Here"   # NEXT placeholder shown when rotation is empty.
# empty_then_text = ["Just scan the QR code", "Or Fill Out A Slip"]   # THEN placeholder entries.
#
# [ticker.queue_info]        # Singer-count / queue-time banner: when and how to show it.
# show_at_start = true       # Show the banner once as soon as the ticker loads.
# show_every_n_singers = 8   # Re-show the banner after this many singers take their turn.
#                             # 0 disables the recurring banner (it can still show at start).
# precise_duration = false   # false = friendly rounded wording ("about an hour and a half",
#                             # rounded to 15-minute increments). true = precise wording
#                             # ("2 hours 1 minute").
# count_legend = "{count} singers in the queue"          # {count} is replaced with the number.
# time_legend = "{time} to get through the queue"  # {time} is replaced with the duration.
#                             # Note: the rounded formatter already prepends its own "about"
#                             # (e.g. "about an hour and a half") — don't add another one here.
#
# [scroll]                   # Visual styling for the 1920x1080 /scroll overlay.
# height = 80                # Banner height in pixels.
# bg = "rgba(8,8,12,0.90)"   # Background, any CSS color value.
# font = "Segoe UI, Helvetica Neue, Arial, sans-serif"   # Font family.
# size = 32                  # Font size in pixels.
# speed = 120                # Scroll speed in pixels per second.
# color_now = "#ffd44f"      # "NOW" label color.
# color_next = "#5bc8ff"     # "NEXT" label color.
# color_up = "#aaa"          # "THEN" label color.
# color_singer = "#fff"      # Singer name color.
# color_song = "#ddd"        # Song title color.
# color_artist = "#aaa"      # Song artist color.
# color_info = "#7cfc8a"     # "QUEUE" label color (singer-count / queue-time banner).
"##;
