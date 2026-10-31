use crate::config::Config;
use crate::requests::{RequestStore, Stage, Tracked};
use crate::server::{SharedConfig, StartupInfo};
use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Tabs, Wrap},
    Frame, Terminal,
};
use std::{
    io,
    path::{Path, PathBuf},
    sync::{mpsc::Receiver, Arc},
    time::Duration,
};
use tokio::sync::Notify;

// ── Field values ────────────────────────────────────────────────────────────
//
// Every config field the TUI edits is reached through `Value`, so reading,
// editing and toggling a field is one code path regardless of its type.

trait Value {
    fn raw(&self) -> String;
    fn set_text(&mut self, _text: &str) -> std::result::Result<(), String> {
        Err("not a text field".to_string())
    }
    fn toggle(&mut self) {}
    fn as_list(&self) -> Option<&Vec<String>> {
        None
    }
    fn as_list_mut(&mut self) -> Option<&mut Vec<String>> {
        None
    }
}

impl Value for String {
    fn raw(&self) -> String {
        self.clone()
    }
    fn set_text(&mut self, text: &str) -> std::result::Result<(), String> {
        *self = text.to_string();
        Ok(())
    }
}

impl Value for bool {
    fn raw(&self) -> String {
        self.to_string()
    }
    fn toggle(&mut self) {
        *self = !*self;
    }
}

impl Value for Vec<String> {
    fn raw(&self) -> String {
        self.join("; ")
    }
    fn as_list(&self) -> Option<&Vec<String>> {
        Some(self)
    }
    fn as_list_mut(&mut self) -> Option<&mut Vec<String>> {
        Some(self)
    }
}

macro_rules! num_value {
    ($($t:ty),*) => {$(
        impl Value for $t {
            fn raw(&self) -> String {
                self.to_string()
            }
            fn set_text(&mut self, text: &str) -> std::result::Result<(), String> {
                *self = text.trim().parse().map_err(|_| format!("\"{text}\" is not a valid number"))?;
                Ok(())
            }
        }
    )*};
}
num_value!(u16, u32, u64, usize);

#[derive(Clone, Copy, PartialEq, Eq)]
enum FieldKind {
    Bool,
    Text,
    Num,
    List,
    Color,
    Secret,
    Choice(&'static [&'static str]),
}

/// Declares every editable field once: its `Key`, where it lives in `Config`
/// (which is also its TOML key), its kind, whether it needs a restart, and
/// its help text.
macro_rules! define_fields {
    ($( $key:ident => $($p:ident).+ , $kind:expr , $restart:expr , $help:expr ; )*) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        enum Key { $($key),* }

        fn field_ref(cfg: &Config, k: Key) -> &dyn Value {
            match k { $(Key::$key => &cfg.$($p).+),* }
        }

        fn field_mut(cfg: &mut Config, k: Key) -> &mut dyn Value {
            match k { $(Key::$key => &mut cfg.$($p).+),* }
        }

        /// Dotted TOML key, e.g. "ticker.queue_info.show_at_start".
        fn config_key(k: Key) -> &'static str {
            let s: &'static str = match k { $(Key::$key => concat!($(stringify!($p), "."),+)),* };
            s.trim_end_matches('.')
        }

        fn kind(k: Key) -> FieldKind {
            use FieldKind::*;
            match k { $(Key::$key => $kind),* }
        }

        fn needs_restart(k: Key) -> bool {
            match k { $(Key::$key => $restart),* }
        }

        fn help(k: Key) -> &'static str {
            match k { $(Key::$key => $help),* }
        }
    };
}

define_fields! {
    ServerPort => server.port, Num, true, "HTTP port this ticker listens on (/, /scroll, /api/state).";
    BindAddress => server.bind_address, Text, true, "Interface to bind. Use \"127.0.0.1\" to restrict to localhost only.";
    UpstreamUrl => ticker.upstream_url, Text, false, "kroak-time's /api/state endpoint to poll.";
    PollIntervalMs => ticker.poll_interval_ms, Num, false, "How often to poll the upstream API, in milliseconds.";
    SingerCount => ticker.singer_count, Num, false, "Local cap on how many upcoming singers /scroll shows, beyond NOW/NEXT. 0 = no override, use whatever kroak-time reports.";
    ShowAllSingers => ticker.show_all_singers, Bool, false, "Ignore singer_count and kroak-time's reported cap; /scroll always shows every singer in the rotation.";
    ShowEmptySingers => ticker.show_empty_singers, Bool, false, "Show singers with no song queued in the THEN list.";
    EmptyNextText => ticker.empty_next_text, Text, false, "NEXT placeholder text shown when the rotation is empty.";
    EmptyThenText => ticker.empty_then_text, List, false, "THEN placeholder entries shown when the rotation is empty — the call to action, e.g. \"Just fill out a slip\" or \"Scan the QR code\".";
    ShowRequestButton => ticker.show_request_button, Bool, false, "Show the \"Request your song\" button at the top of the dashboard (/), linking to request_button_url.";
    RequestButtonText => ticker.request_button_text, Text, false, "Text of the \"Request your song\" button.";
    RequestButtonUrl => ticker.request_button_url, Text, false, "URL the \"Request your song\" button links to when phone requests are off.";
    ShowTiktokLive => ticker.show_tiktok_live, Bool, false, "Show the \"Watch live on TikTok\" button beside the request button on the dashboard (/).";
    TiktokButtonText => ticker.tiktok_button_text, Text, false, "Text of the \"Watch live on TikTok\" button.";
    TiktokButtonUrl => ticker.tiktok_button_url, Text, false, "URL the \"Watch live on TikTok\" button links to.";
    HeroNowLabel => ticker.hero_now_label, Text, false, "Label above the current singer's name on the dashboard (/).";
    HeroNextLabel => ticker.hero_next_label, Text, false, "Label above the next singer's name on the dashboard (/).";
    BadgeNowText => ticker.badge_now_text, Text, false, "Badge text marking the current singer's row in the dashboard table.";
    BadgeNextText => ticker.badge_next_text, Text, false, "Badge text marking the next singer's row in the dashboard table.";
    ShowSongProgress => ticker.show_song_progress, Bool, false, "Show elapsed / remaining time and a progress bar for the song on stage: under the current singer on the dashboard (/), and along the bottom of /scroll.";
    QiShowAtStart => ticker.queue_info.show_at_start, Bool, false, "Show the singer-count/queue-time banner as soon as the ticker loads.";
    QiEveryN => ticker.queue_info.show_every_n_singers, Num, false, "Re-show the singer-count/queue-time banner after this many singers take their turn. 0 disables the recurring banner.";
    QiPrecise => ticker.queue_info.precise_duration, Bool, false, "On = precise duration (\"2 hours 1 minute\"). Off = friendly rounded duration (\"about an hour and a half\").";
    QiCountLegend => ticker.queue_info.count_legend, Text, false, "Legend template for the singer count. {count} is replaced with the number.";
    QiTimeLegend => ticker.queue_info.time_legend, Text, false, "Legend template for the queue time. {time} is replaced with the formatted duration.";
    StEnabled => ticker.sing_time.enabled, Bool, false, "Master switch for showing estimated singing times anywhere on the ticker.";
    StPerSinger => ticker.sing_time.show_per_singer, Bool, false, "Show each singer's own estimated time next to their name, everywhere they're listed.";
    StInBanner => ticker.sing_time.show_in_banner, Bool, false, "Call out the time of whoever's up next right after each queue-info banner.";
    StFormat => ticker.sing_time.format, Choice(&["relative", "clock"]), false, "\"relative\" = duration from now (\"15m\", \"1h20m\"). \"clock\" = absolute local time (\"21:42\").";
    ScHeight => scroll.height, Num, false, "Banner height in pixels.";
    ScBg => scroll.bg, Color, false, "Banner background — any CSS color value.";
    ScFont => scroll.font, Text, false, "Font family.";
    ScSize => scroll.size, Num, false, "Font size in pixels.";
    ScSpeed => scroll.speed, Num, false, "Scroll speed in pixels per second.";
    ScColorNow => scroll.color_now, Color, false, "\"NOW\" label color.";
    ScColorNext => scroll.color_next, Color, false, "\"NEXT\" label color.";
    ScColorUp => scroll.color_up, Color, false, "\"THEN\" label color.";
    ScColorSinger => scroll.color_singer, Color, false, "Singer name color.";
    ScColorSong => scroll.color_song, Color, false, "Song title color.";
    ScColorArtist => scroll.color_artist, Color, false, "Song artist color.";
    ScColorInfo => scroll.color_info, Color, false, "\"QUEUE\" label color (singer-count/queue-time banner).";
    ScLabelNow => scroll.label_now, Text, false, "Text of the \"NOW\" label before the current singer.";
    ScLabelNext => scroll.label_next, Text, false, "Text of the \"NEXT\" label before the next singer.";
    ScLabelUp => scroll.label_up, Text, false, "Text of the \"THEN\" label before each upcoming-singers group.";
    ScLabelInfo => scroll.label_info, Text, false, "Text of the \"QUEUE\" label on the singer-count/queue-time banner.";
    ScColorProgress => scroll.color_progress, Color, false, "Song progress bar color, along the bottom of the banner.";
    ScProgressHeight => scroll.progress_height, Num, false, "Song progress bar height in pixels. 0 hides it.";
    NgEnabled => ngrok.enabled, Bool, true, "Share the ticker over the internet via an ngrok tunnel, in addition to serving it locally.";
    NgAuthtoken => ngrok.authtoken, Secret, true, "ngrok authtoken. Leave blank to fall back to the NGROK_AUTHTOKEN environment variable.";
    NgDomain => ngrok.domain, Text, true, "Reserved ngrok domain (e.g. \"my-ticker.ngrok-free.app\"). Leave blank for a random URL each run.";
    RqEnabled => requests.enabled, Bool, true, "Turn the request button into an in-page song request form that sends requests to kroak-time. Off = the button links to request_button_url instead.";
    RqCatalog => requests.catalog_path, Text, false, "Song catalog: .csv/.tsv/.txt with artist + title columns, .json, or kroak-time's catalog.redb. Blank = kroak-time's default catalog.redb.";
    RqContract => requests.contract_path, Text, true, "kroak-time request-source definition describing the API served to kroak-time. Copy the same file into kroak-time's request-sources folder.";
    RqMaxPending => requests.max_pending_per_device, Num, false, "Most waiting requests one phone can have at once. 0 = unlimited.";
    RqComment => requests.comment_field, Bool, false, "Show an optional free-text \"Comment\" box on the request form. Sent to kroak-time via the contract's {comment} placeholder.";
    RqDupWindow => requests.duplicate_window_minutes, Num, false, "Minutes during which the same song is refused from the same phone or singer name (the singer is told it already reached you). Once sung, it can be requested again. 0 = allow repeats.";
}

// ── Tabs ────────────────────────────────────────────────────────────────────
//
// Tabs are grouped by how often a setting changes, not by config struct:
// "Tonight" holds the per-gig switches, the rest is set-once setup.

enum Row {
    Group(&'static str),
    /// A field, with an optional label that replaces its TOML name.
    Field(Key, Option<&'static str>),
}

struct Tab {
    title: &'static str,
    applies: &'static str,
    rows: &'static [Row],
}

use Row::{Field as F, Group as G};

const TABS: [Tab; 4] = [
    Tab {
        title: "Tonight",
        applies: "Things that change from gig to gig. Check this tab before doors open.",
        rows: &[
            G("Streaming & sharing"),
            F(Key::ShowTiktokLive, Some("tiktok_button")),
            F(Key::NgEnabled, Some("internet_access")),
            G("Sign-up"),
            F(Key::RqEnabled, Some("phone_requests")),
            F(Key::EmptyThenText, Some("empty_queue_prompt")),
            G("Queue display"),
            F(Key::ShowAllSingers, None),
            F(Key::SingerCount, None),
            F(Key::QiEveryN, Some("queue_banner_every")),
            F(Key::StEnabled, Some("sing_times")),
            F(Key::StPerSinger, Some("per_singer_times")),
            F(Key::StFormat, Some("time_format")),
            F(Key::ShowSongProgress, Some("song_progress")),
        ],
    },
    Tab {
        title: "Look",
        applies: "Branding and wording. Set once.",
        rows: &[
            G("Dashboard (/)"),
            F(Key::HeroNowLabel, None),
            F(Key::HeroNextLabel, None),
            F(Key::BadgeNowText, None),
            F(Key::BadgeNextText, None),
            F(Key::TiktokButtonText, None),
            F(Key::TiktokButtonUrl, None),
            G("Scroll (/scroll)"),
            F(Key::ScSpeed, None),
            F(Key::ScHeight, None),
            F(Key::ScSize, None),
            F(Key::ScFont, None),
            F(Key::ScLabelNow, None),
            F(Key::ScLabelNext, None),
            F(Key::ScLabelUp, None),
            F(Key::ScLabelInfo, None),
            F(Key::ScProgressHeight, None),
            G("Scroll colours"),
            F(Key::ScBg, None),
            F(Key::ScColorNow, None),
            F(Key::ScColorNext, None),
            F(Key::ScColorUp, None),
            F(Key::ScColorSinger, None),
            F(Key::ScColorSong, None),
            F(Key::ScColorArtist, None),
            F(Key::ScColorInfo, None),
            F(Key::ScColorProgress, None),
            G("Queue banner & wording"),
            F(Key::ShowEmptySingers, None),
            F(Key::EmptyNextText, None),
            F(Key::QiShowAtStart, None),
            F(Key::QiPrecise, None),
            F(Key::QiCountLegend, None),
            F(Key::QiTimeLegend, None),
            F(Key::StInBanner, None),
        ],
    },
    Tab {
        title: "Requests",
        applies: "How phone requests work. Switched on and off from the Tonight tab.",
        rows: &[
            G("Button"),
            F(Key::RequestButtonText, None),
            F(Key::ShowRequestButton, None),
            F(Key::RequestButtonUrl, None),
            G("Form"),
            F(Key::RqComment, None),
            F(Key::RqMaxPending, None),
            F(Key::RqDupWindow, None),
            F(Key::RqCatalog, None),
            F(Key::RqContract, None),
        ],
    },
    Tab {
        title: "Connection",
        applies: "kroak-time, the local server and the ngrok tunnel. Set once.",
        rows: &[
            G("kroak-time"),
            F(Key::UpstreamUrl, None),
            F(Key::PollIntervalMs, None),
            G("Local server"),
            F(Key::ServerPort, None),
            F(Key::BindAddress, None),
            G("ngrok tunnel (switched on and off from Tonight)"),
            F(Key::NgDomain, Some("ngrok_domain")),
            F(Key::NgAuthtoken, Some("ngrok_authtoken")),
        ],
    },
];

/// The field rows of a tab (group headings skipped), as (key, label).
fn tab_fields(tab: &Tab) -> Vec<(Key, &'static str)> {
    tab.rows
        .iter()
        .filter_map(|r| match r {
            Row::Field(k, label) => Some((*k, label.unwrap_or_else(|| default_label(*k)))),
            Row::Group(_) => None,
        })
        .collect()
}

fn default_label(k: Key) -> &'static str {
    config_key(k).rsplit('.').next().unwrap_or("")
}

/// Why a field currently does nothing, given the other values — or `None`
/// when it's in effect. Inactive fields are greyed out with this reason.
fn inactive(cfg: &Config, k: Key) -> Option<&'static str> {
    let t = &cfg.ticker;
    let q = &t.queue_info;
    let banner_never = !q.show_at_start && q.show_every_n_singers == 0;
    use Key::*;
    match k {
        ShowRequestButton if cfg.requests.enabled => Some("always shown while phone requests are on"),
        RequestButtonUrl if cfg.requests.enabled => Some("ignored while phone requests are on"),
        RequestButtonText if !cfg.requests.enabled && !t.show_request_button => Some("button hidden"),
        RqComment | RqMaxPending | RqDupWindow | RqCatalog | RqContract if !cfg.requests.enabled => Some("phone requests are off (Tonight)"),
        TiktokButtonText | TiktokButtonUrl if !t.show_tiktok_live => Some("TikTok button is off (Tonight)"),
        SingerCount if t.show_all_singers => Some("overridden by show_all_singers"),
        ScColorProgress | ScProgressHeight if !t.show_song_progress => Some("song_progress is off (Tonight)"),
        StPerSinger | StFormat if !t.sing_time.enabled => Some("sing times are off"),
        StInBanner if !t.sing_time.enabled => Some("sing times are off (Tonight)"),
        StInBanner | QiPrecise | QiCountLegend | QiTimeLegend if banner_never => Some("queue banner never shows"),
        NgAuthtoken | NgDomain if !cfg.ngrok.enabled => Some("internet access is off (Tonight)"),
        _ => None,
    }
}

/// Quick toggles available from any tab, also shown in the Status box.
const QUICK_TOGGLES: [(char, Key, &str); 3] = [
    ('t', Key::ShowTiktokLive, "TikTok"),
    ('r', Key::RqEnabled, "Requests"),
    ('n', Key::NgEnabled, "ngrok"),
];

fn bool_value(cfg: &Config, k: Key) -> bool {
    field_ref(cfg, k).raw() == "true"
}

fn display_value(cfg: &Config, k: Key) -> String {
    let v = field_ref(cfg, k);
    match kind(k) {
        FieldKind::Bool => if v.raw() == "true" { "On" } else { "Off" }.to_string(),
        FieldKind::List => format!("{} entries", v.as_list().map(|l| l.len()).unwrap_or(0)),
        FieldKind::Secret => {
            let s = v.raw();
            if s.is_empty() {
                "(unset — using NGROK_AUTHTOKEN env var)".to_string()
            } else {
                "•".repeat(s.chars().count().min(24))
            }
        }
        _ => v.raw(),
    }
}

fn cycle_choice(cfg: &mut Config, k: Key, choices: &[&str], forward: bool) {
    let field = field_mut(cfg, k);
    let cur = field.raw();
    let pos = choices.iter().position(|c| *c == cur).unwrap_or(0);
    let len = choices.len();
    let next = if forward { (pos + 1) % len } else { (pos + len - 1) % len };
    let _ = field.set_text(choices[next]);
}

fn parse_css_color(s: &str) -> Option<Color> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        return match hex.len() {
            6 => Some(Color::Rgb(
                u8::from_str_radix(&hex[0..2], 16).ok()?,
                u8::from_str_radix(&hex[2..4], 16).ok()?,
                u8::from_str_radix(&hex[4..6], 16).ok()?,
            )),
            3 => Some(Color::Rgb(
                u8::from_str_radix(&hex[0..1].repeat(2), 16).ok()?,
                u8::from_str_radix(&hex[1..2].repeat(2), 16).ok()?,
                u8::from_str_radix(&hex[2..3].repeat(2), 16).ok()?,
            )),
            _ => None,
        };
    }
    let inner = s.strip_prefix("rgba(").or_else(|| s.strip_prefix("rgb("))?.strip_suffix(')')?;
    let parts: Vec<&str> = inner.split(',').map(|p| p.trim()).collect();
    if parts.len() < 3 {
        return None;
    }
    Some(Color::Rgb(parts[0].parse().ok()?, parts[1].parse().ok()?, parts[2].parse().ok()?))
}

enum Mode {
    Normal,
    Editing { buffer: String, error: Option<String> },
    ListView { cursor: usize },
    ListEditing { cursor: usize, buffer: String, is_new: bool },
    ConfirmQuit,
    /// Tonight's phone requests. The selection is a request id, not a row,
    /// so a new request arriving at the top doesn't move it.
    Singers { selected: Option<i64> },
    /// Typing a message to whoever sent `request_id`.
    Message { request_id: i64, buffer: String },
}

struct App {
    draft: Config,
    dirty: bool,
    tab_idx: usize,
    cursor: [usize; TABS.len()],
    mode: Mode,
    status: String,
    startup_info: Option<StartupInfo>,
    store: RequestStore,
}

/// Ready-made replies, typed into the message box with F1–F5 (then Enter).
const CANNED: [&str; 5] = [
    "Got it — you're on the list!",
    "Sorry, that one isn't in the catalog. Could you pick another?",
    "Please come and see the host.",
    "You're up soon — please be near the stage.",
    "Sorry, we can't fit that one in tonight.",
];

impl App {
    /// The selected field on the current tab, as (key, label).
    fn selected(&self) -> (Key, &'static str) {
        let fields = tab_fields(&TABS[self.tab_idx]);
        fields[self.cursor[self.tab_idx].min(fields.len() - 1)]
    }
}

/// Restores the terminal on drop, so a panic or early return never leaves
/// the user's shell stuck in raw mode / the alternate screen.
struct TerminalGuard;

impl TerminalGuard {
    fn new() -> Result<Self> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen)?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

pub fn run(
    shared_cfg: SharedConfig,
    shutdown: Arc<Notify>,
    config_path: PathBuf,
    info_rx: Receiver<StartupInfo>,
    store: RequestStore,
) -> Result<()> {
    let _guard = TerminalGuard::new()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    let initial = shared_cfg.read().unwrap().clone();
    let mut app = App {
        draft: initial,
        dirty: false,
        tab_idx: 0,
        cursor: [0; TABS.len()],
        mode: Mode::Normal,
        status: "Loaded.".to_string(),
        startup_info: None,
        store,
    };

    let result = event_loop(&mut terminal, &mut app, &shared_cfg, &config_path, &info_rx);

    // Whatever brought the TUI down (quit, error), the server should stop too.
    shutdown.notify_one();
    result
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    shared_cfg: &SharedConfig,
    config_path: &Path,
    info_rx: &Receiver<StartupInfo>,
) -> Result<()> {
    loop {
        if app.startup_info.is_none() {
            if let Ok(info) = info_rx.try_recv() {
                app.startup_info = Some(info);
            }
        }

        terminal.draw(|f| draw(f, app))?;

        if event::poll(Duration::from_millis(150))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press && handle_key(app, key, shared_cfg, config_path)? {
                    return Ok(());
                }
            }
        }
    }
}

fn handle_key(app: &mut App, key: KeyEvent, shared_cfg: &SharedConfig, config_path: &Path) -> Result<bool> {
    let mode = std::mem::replace(&mut app.mode, Mode::Normal);
    let (next_mode, quit) = step(app, key, mode, shared_cfg, config_path)?;
    app.mode = next_mode;
    Ok(quit)
}

/// The list field being edited in the list popup (the selected field).
fn selected_list(app: &App) -> Option<&Vec<String>> {
    field_ref(&app.draft, app.selected().0).as_list()
}

fn selected_list_mut(app: &mut App) -> Option<&mut Vec<String>> {
    let k = app.selected().0;
    field_mut(&mut app.draft, k).as_list_mut()
}

fn step(
    app: &mut App,
    key: KeyEvent,
    mode: Mode,
    shared_cfg: &SharedConfig,
    config_path: &Path,
) -> Result<(Mode, bool)> {
    let is_text_input = matches!(mode, Mode::Editing { .. } | Mode::ListEditing { .. } | Mode::Message { .. });
    if !is_text_input && key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Ok(request_quit(app));
    }

    match mode {
        Mode::ConfirmQuit => match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => Ok((Mode::ConfirmQuit, true)),
            _ => {
                app.status = "Quit cancelled.".into();
                Ok((Mode::Normal, false))
            }
        },

        Mode::Editing { mut buffer, mut error } => match key.code {
            KeyCode::Esc => {
                app.status = "Edit cancelled.".into();
                Ok((Mode::Normal, false))
            }
            KeyCode::Enter => {
                let (k, label) = app.selected();
                match field_mut(&mut app.draft, k).set_text(&buffer) {
                    Ok(()) => {
                        app.dirty = true;
                        app.status = format!("Set {label} (unsaved).");
                        Ok((Mode::Normal, false))
                    }
                    Err(e) => {
                        error = Some(e);
                        Ok((Mode::Editing { buffer, error }, false))
                    }
                }
            }
            KeyCode::Backspace => {
                buffer.pop();
                Ok((Mode::Editing { buffer, error: None }, false))
            }
            KeyCode::Char(c) if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT => {
                buffer.push(c);
                Ok((Mode::Editing { buffer, error: None }, false))
            }
            _ => Ok((Mode::Editing { buffer, error }, false)),
        },

        Mode::ListView { cursor } => {
            let len = selected_list(app).map(|l| l.len()).unwrap_or(0);
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => Ok((Mode::Normal, false)),
                KeyCode::Up | KeyCode::Char('k') => Ok((Mode::ListView { cursor: cursor.saturating_sub(1) }, false)),
                KeyCode::Down | KeyCode::Char('j') => {
                    let next = if cursor + 1 < len { cursor + 1 } else { cursor };
                    Ok((Mode::ListView { cursor: next }, false))
                }
                KeyCode::Enter if len > 0 => {
                    let buf = selected_list(app).and_then(|l| l.get(cursor)).cloned().unwrap_or_default();
                    Ok((Mode::ListEditing { cursor, buffer: buf, is_new: false }, false))
                }
                KeyCode::Char('a') => Ok((Mode::ListEditing { cursor: len, buffer: String::new(), is_new: true }, false)),
                KeyCode::Char('d') if len > 0 => {
                    if let Some(l) = selected_list_mut(app) {
                        l.remove(cursor);
                    }
                    app.dirty = true;
                    app.status = "Entry deleted (unsaved).".into();
                    let new_len = len - 1;
                    let next_cursor = if new_len == 0 { 0 } else { cursor.min(new_len - 1) };
                    Ok((Mode::ListView { cursor: next_cursor }, false))
                }
                _ => Ok((Mode::ListView { cursor }, false)),
            }
        }

        Mode::ListEditing { cursor, mut buffer, is_new } => match key.code {
            KeyCode::Esc => {
                let len = selected_list(app).map(|l| l.len()).unwrap_or(0);
                let clamped = if len == 0 { 0 } else { cursor.min(len - 1) };
                Ok((Mode::ListView { cursor: clamped }, false))
            }
            KeyCode::Enter => {
                let new_len = if let Some(l) = selected_list_mut(app) {
                    if is_new {
                        l.push(buffer.clone());
                    } else if let Some(slot) = l.get_mut(cursor) {
                        *slot = buffer.clone();
                    }
                    l.len()
                } else {
                    0
                };
                app.dirty = true;
                if is_new {
                    app.status = "Entry added (unsaved). Keep typing to add another, Esc when done.".into();
                    Ok((Mode::ListEditing { cursor: new_len, buffer: String::new(), is_new: true }, false))
                } else {
                    app.status = "Entry saved (unsaved).".into();
                    Ok((Mode::ListView { cursor }, false))
                }
            }
            KeyCode::Backspace => {
                buffer.pop();
                Ok((Mode::ListEditing { cursor, buffer, is_new }, false))
            }
            KeyCode::Char(c) if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT => {
                buffer.push(c);
                Ok((Mode::ListEditing { cursor, buffer, is_new }, false))
            }
            _ => Ok((Mode::ListEditing { cursor, buffer, is_new }, false)),
        },

        Mode::Singers { selected } => {
            let list = app.store.recent();
            let idx = selected.and_then(|id| list.iter().position(|t| t.request_id == id)).unwrap_or(0);
            let id_at = |i: usize| list.get(i).map(|t| t.request_id);
            let stay = Mode::Singers { selected: id_at(idx) };
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('m') => Ok((Mode::Normal, false)),
                KeyCode::Up | KeyCode::Char('k') => Ok((Mode::Singers { selected: id_at(idx.saturating_sub(1)) }, false)),
                KeyCode::Down | KeyCode::Char('j') => {
                    Ok((Mode::Singers { selected: id_at((idx + 1).min(list.len().saturating_sub(1))) }, false))
                }
                KeyCode::Enter if !list.is_empty() => {
                    let t = &list[idx];
                    let buffer = t.message.as_ref().map(|m| m.text.clone()).unwrap_or_default();
                    Ok((Mode::Message { request_id: t.request_id, buffer }, false))
                }
                KeyCode::Char('c') if list.get(idx).is_some_and(|t| t.message.is_some()) => {
                    app.store.set_message(list[idx].request_id, "");
                    app.status = format!("Message to {} cleared.", list[idx].singer);
                    Ok((stay, false))
                }
                _ => Ok((stay, false)),
            }
        }

        Mode::Message { request_id, mut buffer } => match key.code {
            KeyCode::Esc => Ok((Mode::Singers { selected: Some(request_id) }, false)),
            KeyCode::Enter => {
                let who = app
                    .store
                    .recent()
                    .into_iter()
                    .find(|t| t.request_id == request_id)
                    .map(|t| t.singer)
                    .unwrap_or_else(|| "that singer".into());
                app.status = if !app.store.set_message(request_id, &buffer) {
                    "That request is too old to message.".into()
                } else if buffer.trim().is_empty() {
                    format!("Message to {who} cleared.")
                } else {
                    format!("Message sent to {who} — they'll see it while the page is open.")
                };
                Ok((Mode::Singers { selected: Some(request_id) }, false))
            }
            KeyCode::F(n @ 1..=5) => {
                buffer = CANNED[n as usize - 1].to_string();
                Ok((Mode::Message { request_id, buffer }, false))
            }
            KeyCode::Backspace => {
                buffer.pop();
                Ok((Mode::Message { request_id, buffer }, false))
            }
            KeyCode::Char(c) if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT => {
                if buffer.chars().count() < crate::requests::MAX_COMMENT_LEN {
                    buffer.push(c);
                }
                Ok((Mode::Message { request_id, buffer }, false))
            }
            _ => Ok((Mode::Message { request_id, buffer }, false)),
        },

        Mode::Normal => {
            let count = tab_fields(&TABS[app.tab_idx]).len();
            match key.code {
                KeyCode::Char('q') => Ok(request_quit(app)),
                KeyCode::Char('m') => Ok((Mode::Singers { selected: None }, false)),
                KeyCode::Tab | KeyCode::Right => {
                    app.tab_idx = (app.tab_idx + 1) % TABS.len();
                    Ok((Mode::Normal, false))
                }
                KeyCode::BackTab | KeyCode::Left => {
                    app.tab_idx = (app.tab_idx + TABS.len() - 1) % TABS.len();
                    Ok((Mode::Normal, false))
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    if app.cursor[app.tab_idx] > 0 {
                        app.cursor[app.tab_idx] -= 1;
                    }
                    Ok((Mode::Normal, false))
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if app.cursor[app.tab_idx] + 1 < count {
                        app.cursor[app.tab_idx] += 1;
                    }
                    Ok((Mode::Normal, false))
                }
                KeyCode::Char(c) if QUICK_TOGGLES.iter().any(|(qc, _, _)| *qc == c) => {
                    let (_, k, name) = *QUICK_TOGGLES.iter().find(|(qc, _, _)| *qc == c).unwrap();
                    field_mut(&mut app.draft, k).toggle();
                    app.dirty = true;
                    let state = if bool_value(&app.draft, k) { "On" } else { "Off" };
                    let restart = if needs_restart(k) { ", takes effect after a restart" } else { "" };
                    app.status = format!("{name} → {state} (unsaved{restart}).");
                    Ok((Mode::Normal, false))
                }
                KeyCode::Enter | KeyCode::Char(' ') => {
                    let (k, _) = app.selected();
                    match kind(k) {
                        FieldKind::Bool => {
                            field_mut(&mut app.draft, k).toggle();
                            app.dirty = true;
                            app.status = "Toggled (unsaved).".into();
                            Ok((Mode::Normal, false))
                        }
                        FieldKind::Choice(choices) => {
                            cycle_choice(&mut app.draft, k, choices, true);
                            app.dirty = true;
                            app.status = "Changed (unsaved).".into();
                            Ok((Mode::Normal, false))
                        }
                        FieldKind::List => Ok((Mode::ListView { cursor: 0 }, false)),
                        FieldKind::Text | FieldKind::Num | FieldKind::Color | FieldKind::Secret => {
                            let seed = field_ref(&app.draft, k).raw();
                            Ok((Mode::Editing { buffer: seed, error: None }, false))
                        }
                    }
                }
                KeyCode::Char('s') => {
                    match save(app, shared_cfg, config_path) {
                        Ok(()) => {
                            app.dirty = false;
                            app.status = format!("Saved to {}.", config_path.display());
                        }
                        Err(e) => {
                            app.status = format!("Save failed: {e}");
                        }
                    }
                    Ok((Mode::Normal, false))
                }
                _ => Ok((Mode::Normal, false)),
            }
        }
    }
}

fn request_quit(app: &mut App) -> (Mode, bool) {
    if app.dirty {
        app.status = "Unsaved changes — press y to quit anyway, any other key to cancel.".into();
        (Mode::ConfirmQuit, false)
    } else {
        (Mode::Normal, true)
    }
}

fn save(app: &App, shared_cfg: &SharedConfig, config_path: &Path) -> Result<()> {
    app.draft.save(config_path)?;
    *shared_cfg.write().unwrap() = app.draft.clone();
    Ok(())
}

fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(5), Constraint::Length(9), Constraint::Length(2)])
        .split(area);

    draw_tabs(f, chunks[0], app);
    draw_body(f, chunks[1], app);
    draw_status(f, chunks[2], app);
    draw_footer(f, chunks[3], app);

    match &app.mode {
        Mode::Editing { buffer, error } => draw_edit_popup(f, area, app, buffer, error.as_deref()),
        Mode::ListView { cursor } => draw_list_popup(f, area, app, *cursor, None),
        Mode::ListEditing { cursor, buffer, is_new } => {
            draw_list_popup(f, area, app, *cursor, Some((buffer.as_str(), *is_new)))
        }
        Mode::ConfirmQuit => draw_confirm_popup(f, area),
        Mode::Singers { selected } => draw_singers_popup(f, area, app, *selected, None),
        Mode::Message { request_id, buffer } => {
            draw_singers_popup(f, area, app, Some(*request_id), Some(buffer.as_str()))
        }
        Mode::Normal => {}
    }
}

fn ago(secs: i64) -> String {
    match secs.max(0) {
        s @ 0..=59 => format!("{s}s"),
        s @ 60..=3599 => format!("{}m", s / 60),
        s => format!("{}h{:02}m", s / 3600, s % 3600 / 60),
    }
}

fn stage_color(stage: Stage) -> Color {
    match stage {
        Stage::Sent => Color::Gray,
        Stage::WithHost => Color::Cyan,
        Stage::Queued => Color::Blue,
        Stage::UpNext => Color::Yellow,
        Stage::OnNow => Color::Green,
        Stage::Sung => Color::DarkGray,
    }
}

fn singer_row(t: &Tracked, now: i64, selected: bool) -> ListItem<'static> {
    let base = if selected {
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let song = if t.artist.is_empty() { t.title.clone() } else { format!("{} – {}", t.artist, t.title) };
    let mut spans = vec![
        Span::styled(format!("{:>6} ", ago(now - t.sent_at)), Style::default().fg(Color::DarkGray)),
        Span::styled(format!("{} ", t.device_code), Style::default().fg(Color::DarkGray)),
        Span::styled(format!("{:<22.22} ", t.singer), base),
        Span::styled(format!("{:<10}", t.stage.label()), Style::default().fg(stage_color(t.stage))),
        Span::styled(song, base),
    ];
    if let Some(m) = &t.message {
        spans.push(Span::styled(format!("  ✉ {}", m.text), Style::default().fg(Color::Magenta)));
    }
    ListItem::new(Line::from(spans))
}

/// Tonight's phone requests, newest first, with a message box underneath
/// while one is being written.
fn draw_singers_popup(f: &mut Frame, area: Rect, app: &App, selected: Option<i64>, editing: Option<&str>) {
    let list = app.store.recent();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default();
    let idx = selected.and_then(|id| list.iter().position(|t| t.request_id == id)).unwrap_or(0);

    let popup = centered_rect(92, 85, area);
    f.render_widget(Clear, popup);
    let edit_rows: u16 = if editing.is_some() { 5 } else { 0 };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(edit_rows)])
        .split(popup);

    let items: Vec<ListItem> = if list.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "No phone requests in the last 12 hours.",
            Style::default().fg(Color::DarkGray),
        )))]
    } else {
        list.iter().enumerate().map(|(i, t)| singer_row(t, now, i == idx && editing.is_none())).collect()
    };
    let title = if editing.is_some() {
        "Phone requests tonight".to_string()
    } else {
        format!("Phone requests tonight ({}) — ↑/↓ select · Enter message · c clear message · Esc back", list.len())
    };
    let mut state = ListState::default().with_selected((!list.is_empty()).then_some(idx));
    f.render_stateful_widget(
        List::new(items).block(Block::default().borders(Borders::ALL).title(title)),
        rows[0],
        &mut state,
    );

    if let Some(buf) = editing {
        let who = list.get(idx).map(|t| format!("{} ({})", t.singer, t.title)).unwrap_or_default();
        let gray = Style::default().fg(Color::DarkGray);
        let canned: Vec<Span> = CANNED
            .iter()
            .enumerate()
            .flat_map(|(i, c)| {
                let short: String = c.chars().take(18).collect();
                [Span::styled(format!("F{} ", i + 1), Style::default().fg(Color::Gray)), Span::styled(format!("{short}…  "), gray)]
            })
            .collect();
        let lines = vec![
            Line::from(format!("{buf}_")),
            Line::from(""),
            Line::from(canned),
        ];
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!("Message to {who}  (Enter send · empty Enter clears · Esc cancel)"));
        f.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: false }), rows[1]);
    }
}

/// "[t] TikTok On   [r] Requests Off   [n] ngrok On" — the per-gig switches,
/// as currently set in the draft.
fn quick_toggle_line(app: &App) -> Line<'static> {
    let gray = Style::default().fg(Color::Gray);
    let mut spans = vec![Span::styled(format!("{:<11}", "Tonight"), Style::default().fg(Color::Yellow))];
    for (i, (c, k, name)) in QUICK_TOGGLES.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("   "));
        }
        let on = bool_value(&app.draft, *k);
        spans.push(Span::styled(format!("[{c}] "), gray));
        spans.push(Span::raw(format!("{name} ")));
        spans.push(Span::styled(
            if on { "On " } else { "Off" },
            Style::default().fg(if on { Color::Green } else { Color::DarkGray }),
        ));
    }
    Line::from(spans)
}

fn draw_status(f: &mut Frame, area: Rect, app: &App) {
    let mut lines = vec![quick_toggle_line(app)];
    match &app.startup_info {
        None => lines.push(Line::from(Span::styled("Starting server…", Style::default().fg(Color::Gray)))),
        Some(info) => {
            let label = |s: &'static str| Span::styled(format!("{s:<11}"), Style::default().fg(Color::Gray));

            lines.push(Line::from(vec![
                label("Dashboard"),
                Span::raw(info.local_dashboard_url.clone()),
                Span::raw("    "),
                Span::styled("Scroll  ", Style::default().fg(Color::Gray)),
                Span::raw(info.local_scroll_url.clone()),
            ]));
            lines.push(Line::from(vec![
                label("LAN"),
                Span::raw(info.lan_dashboard_url.clone()),
                Span::raw("    "),
                Span::styled("Scroll  ", Style::default().fg(Color::Gray)),
                Span::raw(info.lan_scroll_url.clone()),
            ]));

            let ngrok_line = if let Some(url) = &info.ngrok_dashboard_url {
                let ok = Style::default().fg(Color::Green);
                Line::from(vec![
                    Span::styled(format!("{:<11}", "ngrok"), ok),
                    Span::styled(url.clone(), ok),
                    Span::raw("    "),
                    Span::styled("Scroll  ", ok),
                    Span::styled(info.ngrok_scroll_url.clone().unwrap_or_default(), ok),
                ])
            } else if let Some(err) = &info.ngrok_error {
                Line::from(Span::styled(
                    format!("⚠ ngrok enabled but could not connect: {err}"),
                    Style::default().fg(Color::Red),
                ))
            } else {
                Line::from(Span::styled(format!("{:<11}disabled", "ngrok"), Style::default().fg(Color::DarkGray)))
            };
            lines.push(ngrok_line);

            lines.push(Line::from(vec![label("Upstream"), Span::raw(info.upstream_url.clone())]));

            lines.push(match &info.request_server {
                Some(Ok(addr)) => Line::from(vec![
                    Span::styled(format!("{:<11}", "Requests"), Style::default().fg(Color::Green)),
                    Span::styled(addr.clone(), Style::default().fg(Color::Green)),
                ]),
                Some(Err(err)) => Line::from(Span::styled(
                    format!("⚠ Song requests enabled but the request contract failed: {err}"),
                    Style::default().fg(Color::Red),
                )),
                None => Line::from(Span::styled(format!("{:<11}disabled", "Requests"), Style::default().fg(Color::DarkGray))),
            });

            lines.push(Line::from(vec![label("Data"), Span::raw(crate::paths::data_dir().display().to_string())]));
        }
    }

    let block = Block::default().borders(Borders::ALL).title("Status");
    f.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: true }), area);
}

fn draw_tabs(f: &mut Frame, area: Rect, app: &App) {
    let titles: Vec<Line> = TABS.iter().map(|t| Line::from(t.title)).collect();
    let tabs = Tabs::new(titles)
        .select(app.tab_idx)
        .block(Block::default().borders(Borders::ALL).title("kroak-time-ticker config"))
        .highlight_style(Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD));
    f.render_widget(tabs, area);
}

fn draw_body(f: &mut Frame, area: Rect, app: &App) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    let tab = &TABS[app.tab_idx];
    let cursor = app.cursor[app.tab_idx];
    let dim = Style::default().fg(Color::DarkGray);

    let mut items: Vec<ListItem> = Vec::new();
    let mut selected_row = 0;
    let mut field_idx = 0;
    for row in tab.rows {
        match row {
            Row::Group(title) => {
                let rule = "─".repeat(40usize.saturating_sub(title.chars().count()).max(2));
                items.push(ListItem::new(Line::from(Span::styled(
                    format!("── {title} {rule}"),
                    Style::default().fg(Color::Cyan),
                ))));
            }
            Row::Field(k, label) => {
                let k = *k;
                let selected = field_idx == cursor;
                if selected {
                    selected_row = items.len();
                }
                field_idx += 1;

                let why = inactive(&app.draft, k);
                let label_style = match (selected, why.is_some()) {
                    (true, _) => Style::default().add_modifier(Modifier::BOLD).fg(Color::Yellow),
                    (false, true) => dim,
                    (false, false) => Style::default(),
                };
                let label = format!("{}{}", label.unwrap_or_else(|| default_label(k)), if needs_restart(k) { " *" } else { "" });
                let mut spans = vec![Span::styled(format!("{label:<24}"), label_style)];

                if kind(k) == FieldKind::Color {
                    if let Some(c) = parse_css_color(&field_ref(&app.draft, k).raw()) {
                        spans.push(Span::styled("■ ", Style::default().fg(c)));
                    }
                }
                let value_style = if why.is_some() { dim } else { Style::default() };
                spans.push(Span::styled(display_value(&app.draft, k), value_style));
                if let Some(why) = why {
                    spans.push(Span::styled(format!("  — {why}"), dim.add_modifier(Modifier::ITALIC)));
                }
                items.push(ListItem::new(Line::from(spans)));
            }
        }
    }

    // A stateful list scrolls to keep the selection visible on tabs taller
    // than the terminal (Look has ~35 rows).
    let mut state = ListState::default().with_selected(Some(selected_row));
    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(tab.title));
    f.render_stateful_widget(list, cols[0], &mut state);

    let (k, _) = app.selected();
    let gray = Style::default().fg(Color::Gray);
    let mut lines = vec![
        Line::from(Span::styled(tab.applies, Style::default().fg(Color::Cyan))),
        Line::from(""),
        Line::from(help(k)),
        Line::from(""),
        Line::from(Span::styled(format!("config key: {}", config_key(k)), gray)),
    ];
    if let Some(why) = inactive(&app.draft, k) {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(format!("Inactive: {why}."), Style::default().fg(Color::Yellow))));
    }
    if needs_restart(k) {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("* takes effect after a restart of kroak-time-ticker.", gray)));
    }
    let help = Paragraph::new(lines)
        .wrap(Wrap { trim: true })
        .block(Block::default().borders(Borders::ALL).title("About this setting"));
    f.render_widget(help, cols[1]);
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let dirty_tag = if app.dirty { " [UNSAVED]" } else { "" };
    let text = format!(
        "{}{}\n←/→ tab  ↑/↓ field  Enter edit/toggle  t/r/n quick toggles  m message singers  s save  q quit",
        app.status, dirty_tag
    );
    f.render_widget(Paragraph::new(text), area);
}

/// Like `centered_rect`, but with an exact row count instead of a
/// percentage of the available height — used where the popup should be
/// sized to its content rather than stretched to fill the screen.
fn centered_rect_height(percent_x: u16, height: u16, r: Rect) -> Rect {
    let height = height.min(r.height);
    let top = (r.height - height) / 2;
    let vertical = Rect { x: r.x, y: r.y + top, width: r.width, height };
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical)[1]
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

fn draw_edit_popup(f: &mut Frame, area: Rect, app: &App, buffer: &str, error: Option<&str>) {
    let (k, label) = app.selected();
    let mask = kind(k) == FieldKind::Secret;
    let shown = if mask { "•".repeat(buffer.chars().count()) } else { buffer.to_string() };

    let popup = centered_rect(60, 25, area);
    f.render_widget(Clear, popup);
    let mut lines = vec![Line::from(format!("{shown}_"))];
    if let Some(e) = error {
        lines.push(Line::from(Span::styled(e, Style::default().fg(Color::Red))));
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!("Edit {label}  (Enter save · Esc cancel)"));
    f.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: false }), popup);
}

fn draw_list_popup(f: &mut Frame, area: Rect, app: &App, cursor: usize, editing: Option<(&str, bool)>) {
    let list = selected_list(app).cloned().unwrap_or_default();
    let (_, label) = app.selected();

    // Size the popup to its actual content (entries + borders + the edit box
    // when open) instead of a fixed percentage of the screen. A fixed 60%
    // height let the edit box land exactly on the Status panel's top border
    // on small terminals, bleeding both boxes' borders onto the same row.
    let list_rows = (list.len() as u16).max(1) + 2;
    let edit_rows: u16 = if editing.is_some() { 3 } else { 0 };
    let height = (list_rows + edit_rows).min(area.height.saturating_sub(2)).max(6);

    let popup = centered_rect_height(70, height, area);
    f.render_widget(Clear, popup);

    let rows = if editing.is_some() {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(3)])
            .split(popup)
    } else {
        Layout::default().direction(Direction::Vertical).constraints([Constraint::Min(3)]).split(popup)
    };
    let items: Vec<ListItem> = list
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let style = if i == cursor && editing.is_none() {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(Span::styled(s.clone(), style)))
        })
        .collect();

    let title = format!("{label} — ↑/↓ select · Enter edit · a add · d delete · Esc back");
    f.render_widget(List::new(items).block(Block::default().borders(Borders::ALL).title(title)), rows[0]);

    if let Some((buf, is_new)) = editing {
        let edit_title = if is_new {
            "Entry text (Enter add & next · Esc done)"
        } else {
            "Entry text (Enter save · Esc cancel)"
        };
        let edit_block = Block::default().borders(Borders::ALL).title(edit_title);
        f.render_widget(Paragraph::new(format!("{buf}_")).block(edit_block), rows[1]);
    }
}

fn draw_confirm_popup(f: &mut Frame, area: Rect) {
    let popup = centered_rect(50, 20, area);
    f.render_widget(Clear, popup);
    let block = Block::default().borders(Borders::ALL).title("Quit without saving?");
    let text = Paragraph::new("You have unsaved changes.\n\ny = quit anyway   any other key = cancel").block(block);
    f.render_widget(text, popup);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_keys_are_dotted_paths() {
        assert_eq!(config_key(Key::QiShowAtStart), "ticker.queue_info.show_at_start");
        assert_eq!(config_key(Key::ServerPort), "server.port");
        assert_eq!(default_label(Key::RqMaxPending), "max_pending_per_device");
    }

    #[test]
    fn every_field_is_on_exactly_one_tab() {
        let mut seen: Vec<Key> = TABS.iter().flat_map(|t| tab_fields(t).into_iter().map(|(k, _)| k)).collect();
        let total = seen.len();
        seen.sort_by_key(|k| *k as usize);
        seen.dedup();
        assert_eq!(seen.len(), total, "a field appears on two tabs");
        assert_eq!(total, Key::RqDupWindow as usize + 1, "a field is missing from the tabs");
    }

    #[test]
    fn request_button_fields_grey_out_while_form_is_on() {
        let mut cfg: Config = toml::from_str("").unwrap();
        cfg.requests.enabled = true;
        assert!(inactive(&cfg, Key::RequestButtonUrl).is_some());
        assert!(inactive(&cfg, Key::RqCatalog).is_none());
        cfg.requests.enabled = false;
        assert!(inactive(&cfg, Key::RequestButtonUrl).is_none());
        assert!(inactive(&cfg, Key::RqCatalog).is_some());
    }

    #[test]
    fn edit_and_toggle_through_keys() {
        let mut cfg: Config = toml::from_str("").unwrap();
        field_mut(&mut cfg, Key::ServerPort).set_text("9090").unwrap();
        assert_eq!(cfg.server.port, 9090);
        assert!(field_mut(&mut cfg, Key::ServerPort).set_text("x").is_err());
        let before = cfg.ngrok.enabled;
        field_mut(&mut cfg, Key::NgEnabled).toggle();
        assert_eq!(cfg.ngrok.enabled, !before);
        cfg.ticker.sing_time.format = "relative".into();
        cycle_choice(&mut cfg, Key::StFormat, &["relative", "clock"], true);
        assert_eq!(cfg.ticker.sing_time.format, "clock");
    }
}
