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

    #[serde(default)]
    pub requests: RequestsConfig,
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

    /// Show the "Request your song" button at the top of the dashboard (/).
    /// Default true. Can still be overridden per-view with ?request=0/1 in
    /// the URL regardless of this setting.
    #[serde(default = "default_true")]
    pub show_request_button: bool,

    /// Text of the "Request your song" button.
    #[serde(default = "default_request_button_text")]
    pub request_button_text: String,

    /// URL the "Request your song" button links to.
    #[serde(default = "default_request_button_url")]
    pub request_button_url: String,

    /// Show the "Watch live on TikTok" button beside the request button on
    /// the dashboard (/). Default false.
    #[serde(default)]
    pub show_tiktok_live: bool,

    /// Text of the "Watch live on TikTok" button.
    #[serde(default = "default_tiktok_button_text")]
    pub tiktok_button_text: String,

    /// URL the "Watch live on TikTok" button links to.
    #[serde(default = "default_tiktok_button_url")]
    pub tiktok_button_url: String,

    /// Label above the current singer's name on the dashboard (/).
    #[serde(default = "default_hero_now_label")]
    pub hero_now_label: String,

    /// Label above the next singer's name on the dashboard (/).
    #[serde(default = "default_hero_next_label")]
    pub hero_next_label: String,

    /// Text of the badge marking the current singer's row in the dashboard
    /// rotation table.
    #[serde(default = "default_badge_now_text")]
    pub badge_now_text: String,

    /// Text of the badge marking the next singer's row in the dashboard
    /// rotation table.
    #[serde(default = "default_badge_next_text")]
    pub badge_next_text: String,

    /// Show elapsed / remaining time and a progress bar for the song on
    /// stage, under the current singer on the dashboard (/), and a progress
    /// bar along the bottom of the /scroll banner. Needs a
    /// kroak-time that sends `song_position_ms`; hidden otherwise.
    #[serde(default = "default_true")]
    pub show_song_progress: bool,

    /// Singer-count / queue-time banner: when and how to show it.
    #[serde(default)]
    pub queue_info: QueueInfoConfig,

    /// Per-singer estimated singing time (kroak-time's `sings_in_secs` /
    /// `sings_at`): whether to show it at all, where, and how to format it.
    #[serde(default)]
    pub sing_time: SingTimeConfig,
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
            show_request_button: default_true(),
            request_button_text: default_request_button_text(),
            request_button_url: default_request_button_url(),
            show_tiktok_live: false,
            tiktok_button_text: default_tiktok_button_text(),
            tiktok_button_url: default_tiktok_button_url(),
            hero_now_label: default_hero_now_label(),
            hero_next_label: default_hero_next_label(),
            badge_now_text: default_badge_now_text(),
            badge_next_text: default_badge_next_text(),
            show_song_progress: true,
            queue_info: QueueInfoConfig::default(),
            sing_time: SingTimeConfig::default(),
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
    "{count} singers".to_string()
}
fn default_time_legend() -> String {
    "{time} to get through the queue".to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SingTimeConfig {
    /// Master switch for showing kroak-time's estimated singing times
    /// (`sings_in_secs` / `sings_at`) anywhere on the ticker. Default off —
    /// everything below only takes effect when this is also true.
    #[serde(default)]
    pub enabled: bool,

    /// Show each singer's own estimated time next to their name, everywhere
    /// they're listed (NOW/NEXT/THEN on /scroll, the rotation table on /).
    #[serde(default)]
    pub show_per_singer: bool,

    /// Instead of (or alongside) per-singer times, call out the estimated
    /// time of whoever's coming up right after the queue-info banner, each
    /// time it shows — including the recurring one from
    /// `queue_info.show_every_n_singers`. A lighter-touch way to share timing
    /// periodically without a time next to every single name.
    #[serde(default)]
    pub show_in_banner: bool,

    /// How to format a singing time: "relative" = duration from now ("15m",
    /// "1h20m", "now" for 0), "clock" = absolute local time ("21:42").
    #[serde(default = "default_sing_time_format")]
    pub format: String,
}

impl Default for SingTimeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            show_per_singer: false,
            show_in_banner: false,
            format: default_sing_time_format(),
        }
    }
}

fn default_sing_time_format() -> String {
    "relative".to_string()
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

    /// Text of the "NOW" label before the current singer.
    #[serde(default = "default_label_now")]
    pub label_now: String,

    /// Text of the "NEXT" label before the next singer.
    #[serde(default = "default_label_next")]
    pub label_next: String,

    /// Text of the "THEN" label before each upcoming-singers group.
    #[serde(default = "default_label_up")]
    pub label_up: String,

    /// Text of the "QUEUE" label on the singer-count / queue-time banner.
    #[serde(default = "default_label_info")]
    pub label_info: String,

    /// Color of the song progress bar along the bottom of the banner
    /// (shown when `[ticker].show_song_progress` is on).
    #[serde(default = "default_color_progress")]
    pub color_progress: String,

    /// Height of the song progress bar in pixels. 0 hides it.
    #[serde(default = "default_progress_height")]
    pub progress_height: u32,
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
            label_now:    default_label_now(),
            label_next:   default_label_next(),
            label_up:     default_label_up(),
            label_info:   default_label_info(),
            color_progress:  default_color_progress(),
            progress_height: default_progress_height(),
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
fn default_label_now()     -> String { "NOW".into() }
fn default_label_next()    -> String { "NEXT".into() }
fn default_label_up()      -> String { "THEN".into() }
fn default_label_info()    -> String { "QUEUE".into() }
fn default_color_info()    -> String { "#7cfc8a".into() }
fn default_color_progress() -> String { "#ff8c1a".into() }
fn default_progress_height() -> u32  { 6 }

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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RequestsConfig {
    /// Turn the dashboard's request button into an in-page song request form
    /// that sends requests to kroak-time. Default off (the button keeps
    /// linking to `request_button_url`).
    #[serde(default)]
    pub enabled: bool,

    /// Song catalog the form searches: a .csv/.tsv/.txt (with "artist" and
    /// "title" header columns), a .json array of {artist, title}, or
    /// kroak-time's catalog.redb. Blank = kroak-time's default catalog.redb
    /// location, if it exists. See CATALOG_FORMAT.md.
    #[serde(default)]
    pub catalog_path: String,

    /// kroak-time request-source definition file describing the API served
    /// to kroak-time (address, operations, field names). Written with the
    /// built-in default if missing; copy the same file into kroak-time's
    /// request-sources folder.
    #[serde(default = "default_contract_path")]
    pub contract_path: String,

    /// Most not-yet-handled requests one phone can have waiting at once.
    /// 0 = unlimited.
    #[serde(default = "default_max_pending_per_device")]
    pub max_pending_per_device: usize,

    /// Show an optional free-text "Comment" box on the request form. The
    /// comment reaches kroak-time through the contract's `{comment}`
    /// placeholder (by default, in the "requested by" column).
    #[serde(default = "default_true")]
    pub comment_field: bool,
}

impl Default for RequestsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            catalog_path: String::new(),
            contract_path: default_contract_path(),
            max_pending_per_device: default_max_pending_per_device(),
            comment_field: true,
        }
    }
}

fn default_contract_path() -> String {
    "request-source.toml".to_string()
}
fn default_max_pending_per_device() -> usize {
    5
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
fn default_request_button_text() -> String {
    "Request your song".to_string()
}
fn default_request_button_url() -> String {
    "https://linktr.ee/wolflike".to_string()
}
fn default_tiktok_button_text() -> String {
    "Watch live on TikTok".to_string()
}
fn default_tiktok_button_url() -> String {
    "https://tiktok.com/@lordjohnrussell/live".to_string()
}
fn default_hero_now_label() -> String {
    "Now Singing".to_string()
}
fn default_hero_next_label() -> String {
    "Next Up".to_string()
}
fn default_badge_now_text() -> String {
    "NOW".to_string()
}
fn default_badge_next_text() -> String {
    "NEXT".to_string()
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
                requests: RequestsConfig::default(),
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
# port = 8080                # HTTP port this ticker listens on (/, /scroll, /api/state).
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
# show_request_button = true # Show the "Request your song" button at the top of the
#                             # dashboard (/). Can still be overridden per-view with
#                             # ?request=0/1 in the URL regardless of this setting.
# request_button_text = "Request your song"          # Text of that button.
# request_button_url = "https://linktr.ee/wolflike"   # URL that button links to.
# show_tiktok_live = false   # Show the "Watch live on TikTok" button beside the request
#                             # button on the dashboard (/). Default false.
# tiktok_button_text = "Watch live on TikTok"                  # Text of that button.
# tiktok_button_url = "https://tiktok.com/@lordjohnrussell/live"   # URL that button links to.
# hero_now_label = "Now Singing"   # Label above the current singer's name on the dashboard.
# hero_next_label = "Next Up"      # Label above the next singer's name on the dashboard.
# badge_now_text = "NOW"           # Badge text marking the current singer's row in the table.
# badge_next_text = "NEXT"         # Badge text marking the next singer's row in the table.
# show_song_progress = true        # Elapsed / remaining time + progress bar for the song on
#                                   # stage, under the current singer on the dashboard.
#
# [ticker.queue_info]        # Singer-count / queue-time banner: when and how to show it.
# show_at_start = true       # Show the banner once as soon as the ticker loads.
# show_every_n_singers = 8   # Re-show the banner after this many singers take their turn.
#                             # 0 disables the recurring banner (it can still show at start).
# precise_duration = false   # false = friendly rounded wording ("about an hour and a half",
#                             # rounded to 15-minute increments). true = precise wording
#                             # ("2 hours 1 minute").
# count_legend = "{count} singers"          # {count} is replaced with the number.
# time_legend = "{time} to get through the queue"  # {time} is replaced with the duration.
#                             # Note: the rounded formatter already prepends its own "about"
#                             # (e.g. "about an hour and a half") — don't add another one here.
#
# [ticker.sing_time]         # Per-singer estimated singing time, from kroak-time's
#                             # sings_in_secs/sings_at (rotation-timing release). Null/absent
#                             # upstream (timing off in kroak-time, or an older kroak-time)
#                             # just means nothing is shown, even with this enabled.
# enabled = false            # Master switch. Everything below only matters if this is true.
# show_per_singer = false    # Show each singer's own time next to their name, everywhere
#                             # they're listed (NOW/NEXT/THEN on /scroll, the table on /).
# show_in_banner = false     # Instead of (or alongside) per-singer times, call out the time
#                             # of whoever's up next right after each queue-info banner —
#                             # including the recurring one from show_every_n_singers above.
# format = "relative"        # "relative" = duration from now ("15m", "1h20m", "now").
#                             # "clock" = absolute local time ("21:42").
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
# label_now = "NOW"          # Text of the "NOW" label before the current singer.
# label_next = "NEXT"        # Text of the "NEXT" label before the next singer.
# label_up = "THEN"          # Text of the "THEN" label before each upcoming-singers group.
# label_info = "QUEUE"       # Text of the "QUEUE" label on the singer-count / queue-time banner.
# color_progress = "#ff8c1a" # Song progress bar along the bottom (needs show_song_progress).
# progress_height = 6        # Song progress bar height in pixels. 0 hides it.
#
# [requests]                 # In-page song requests on the dashboard (/), sent to kroak-time.
# enabled = false            # When true, the request button (show_request_button /
#                             # request_button_text above) opens a request form instead of
#                             # linking to request_button_url.
# catalog_path = ""          # .csv/.tsv/.txt with "artist" and "title" header columns, a .json
#                             # array of {"artist", "title"}, or kroak-time's catalog.redb.
#                             # Blank = kroak-time's default catalog.redb, if found. See
#                             # CATALOG_FORMAT.md. Re-read automatically when the file changes.
# contract_path = "request-source.toml"   # API served to kroak-time, as a kroak-time
#                             # request-source definition (written with defaults if missing).
#                             # Copy the same file into kroak-time's request-sources folder.
# max_pending_per_device = 5 # Most waiting requests one phone can have. 0 = unlimited.
# comment_field = true       # Show an optional free-text "Comment" box on the request form.
"##;
