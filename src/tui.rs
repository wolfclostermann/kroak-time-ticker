use crate::config::Config;
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
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Tabs, Wrap},
    Frame, Terminal,
};
use std::{
    io,
    path::{Path, PathBuf},
    sync::{mpsc::Receiver, Arc},
    time::Duration,
};
use tokio::sync::Notify;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    Server,
    Ticker,
    QueueInfo,
    SingTime,
    Scroll,
    Ngrok,
}

const SECTIONS: [Section; 6] = [
    Section::Ticker,
    Section::QueueInfo,
    Section::SingTime,
    Section::Scroll,
    Section::Server,
    Section::Ngrok,
];

impl Section {
    fn title(&self) -> &'static str {
        match self {
            Section::Server => "Server",
            Section::Ticker => "Ticker",
            Section::QueueInfo => "Queue Info",
            Section::SingTime => "Sing Time",
            Section::Scroll => "Scroll",
            Section::Ngrok => "Ngrok",
        }
    }
}

#[derive(Clone, Copy)]
enum FieldKind {
    Bool,
    Text,
    Num,
    List,
    Choice(&'static [&'static str]),
}

struct FieldMeta {
    label: &'static str,
    help: &'static str,
    kind: FieldKind,
    restart: bool,
}

fn fields(section: Section) -> &'static [FieldMeta] {
    match section {
        Section::Server => &[
            FieldMeta { label: "port", help: "HTTP port this ticker listens on (/, /scroll, /api/state).", kind: FieldKind::Num, restart: true },
            FieldMeta { label: "bind_address", help: "Interface to bind. Use \"127.0.0.1\" to restrict to localhost only.", kind: FieldKind::Text, restart: true },
        ],
        Section::Ticker => &[
            FieldMeta { label: "upstream_url", help: "kroak-time's /api/state endpoint to poll.", kind: FieldKind::Text, restart: false },
            FieldMeta { label: "poll_interval_ms", help: "How often to poll the upstream API, in milliseconds.", kind: FieldKind::Num, restart: false },
            FieldMeta { label: "singer_count", help: "Local cap on how many upcoming singers to show, beyond NOW/NEXT. 0 = no override, use whatever kroak-time reports.", kind: FieldKind::Num, restart: false },
            FieldMeta { label: "show_all_singers", help: "Ignore singer_count above and kroak-time's reported cap; always show every singer in the rotation.", kind: FieldKind::Bool, restart: false },
            FieldMeta { label: "show_empty_singers", help: "Show singers with no song queued in the THEN list.", kind: FieldKind::Bool, restart: false },
            FieldMeta { label: "empty_next_text", help: "NEXT placeholder text shown when the rotation is empty.", kind: FieldKind::Text, restart: false },
            FieldMeta { label: "empty_then_text", help: "THEN placeholder entries shown when the rotation is empty.", kind: FieldKind::List, restart: false },
            FieldMeta { label: "show_request_button", help: "Show the \"Request your song\" button at the top of the dashboard (/).", kind: FieldKind::Bool, restart: false },
            FieldMeta { label: "request_button_text", help: "Text of the \"Request your song\" button.", kind: FieldKind::Text, restart: false },
            FieldMeta { label: "request_button_url", help: "URL the \"Request your song\" button links to.", kind: FieldKind::Text, restart: false },
            FieldMeta { label: "show_tiktok_live", help: "Show the \"Watch live on TikTok\" button beside the request button.", kind: FieldKind::Bool, restart: false },
            FieldMeta { label: "tiktok_button_text", help: "Text of the \"Watch live on TikTok\" button.", kind: FieldKind::Text, restart: false },
            FieldMeta { label: "tiktok_button_url", help: "URL the \"Watch live on TikTok\" button links to.", kind: FieldKind::Text, restart: false },
            FieldMeta { label: "hero_now_label", help: "Label above the current singer's name on the dashboard (/).", kind: FieldKind::Text, restart: false },
            FieldMeta { label: "hero_next_label", help: "Label above the next singer's name on the dashboard (/).", kind: FieldKind::Text, restart: false },
            FieldMeta { label: "badge_now_text", help: "Badge text marking the current singer's row in the dashboard table.", kind: FieldKind::Text, restart: false },
            FieldMeta { label: "badge_next_text", help: "Badge text marking the next singer's row in the dashboard table.", kind: FieldKind::Text, restart: false },
        ],
        Section::QueueInfo => &[
            FieldMeta { label: "show_at_start", help: "Show the singer-count/queue-time banner as soon as the ticker loads.", kind: FieldKind::Bool, restart: false },
            FieldMeta { label: "show_every_n_singers", help: "Re-show the banner after this many singers take their turn. 0 disables the recurring banner.", kind: FieldKind::Num, restart: false },
            FieldMeta { label: "precise_duration", help: "On = precise duration (\"2 hours 1 minute\"). Off = friendly rounded duration (\"about an hour and a half\").", kind: FieldKind::Bool, restart: false },
            FieldMeta { label: "count_legend", help: "Legend template for the singer count. {count} is replaced with the number.", kind: FieldKind::Text, restart: false },
            FieldMeta { label: "time_legend", help: "Legend template for the queue time. {time} is replaced with the formatted duration.", kind: FieldKind::Text, restart: false },
        ],
        Section::SingTime => &[
            FieldMeta { label: "enabled", help: "Master switch for showing estimated singing times anywhere on the ticker.", kind: FieldKind::Bool, restart: false },
            FieldMeta { label: "show_per_singer", help: "Show each singer's own estimated time next to their name, everywhere they're listed.", kind: FieldKind::Bool, restart: false },
            FieldMeta { label: "show_in_banner", help: "Call out the time of whoever's up next right after each queue-info banner.", kind: FieldKind::Bool, restart: false },
            FieldMeta { label: "format", help: "\"relative\" = duration from now (\"15m\", \"1h20m\"). \"clock\" = absolute local time (\"21:42\").", kind: FieldKind::Choice(&["relative", "clock"]), restart: false },
        ],
        Section::Scroll => &[
            FieldMeta { label: "height", help: "Banner height in pixels.", kind: FieldKind::Num, restart: false },
            FieldMeta { label: "bg", help: "Banner background — any CSS color value.", kind: FieldKind::Text, restart: false },
            FieldMeta { label: "font", help: "Font family.", kind: FieldKind::Text, restart: false },
            FieldMeta { label: "size", help: "Font size in pixels.", kind: FieldKind::Num, restart: false },
            FieldMeta { label: "speed", help: "Scroll speed in pixels per second.", kind: FieldKind::Num, restart: false },
            FieldMeta { label: "color_now", help: "\"NOW\" label color.", kind: FieldKind::Text, restart: false },
            FieldMeta { label: "color_next", help: "\"NEXT\" label color.", kind: FieldKind::Text, restart: false },
            FieldMeta { label: "color_up", help: "\"THEN\" label color.", kind: FieldKind::Text, restart: false },
            FieldMeta { label: "color_singer", help: "Singer name color.", kind: FieldKind::Text, restart: false },
            FieldMeta { label: "color_song", help: "Song title color.", kind: FieldKind::Text, restart: false },
            FieldMeta { label: "color_artist", help: "Song artist color.", kind: FieldKind::Text, restart: false },
            FieldMeta { label: "color_info", help: "\"QUEUE\" label color (singer-count/queue-time banner).", kind: FieldKind::Text, restart: false },
            FieldMeta { label: "label_now", help: "Text of the \"NOW\" label before the current singer.", kind: FieldKind::Text, restart: false },
            FieldMeta { label: "label_next", help: "Text of the \"NEXT\" label before the next singer.", kind: FieldKind::Text, restart: false },
            FieldMeta { label: "label_up", help: "Text of the \"THEN\" label before each upcoming-singers group.", kind: FieldKind::Text, restart: false },
            FieldMeta { label: "label_info", help: "Text of the \"QUEUE\" label on the singer-count/queue-time banner.", kind: FieldKind::Text, restart: false },
        ],
        Section::Ngrok => &[
            FieldMeta { label: "enabled", help: "Share the ticker over the internet via an ngrok tunnel, in addition to serving it locally.", kind: FieldKind::Bool, restart: true },
            FieldMeta { label: "authtoken", help: "ngrok authtoken. Leave blank to fall back to the NGROK_AUTHTOKEN environment variable.", kind: FieldKind::Text, restart: true },
            FieldMeta { label: "domain", help: "Reserved ngrok domain (e.g. \"my-ticker.ngrok-free.app\"). Leave blank for a random URL each run.", kind: FieldKind::Text, restart: true },
        ],
    }
}

fn field_count(section: Section) -> usize {
    fields(section).len()
}

fn is_color_field(section: Section, idx: usize) -> bool {
    section == Section::Scroll && matches!(idx, 1 | 5..=11)
}

fn value_str(cfg: &Config, section: Section, idx: usize) -> String {
    use Section::*;
    match (section, idx) {
        (Server, 0) => cfg.server.port.to_string(),
        (Server, 1) => cfg.server.bind_address.clone(),
        (Ticker, 0) => cfg.ticker.upstream_url.clone(),
        (Ticker, 1) => cfg.ticker.poll_interval_ms.to_string(),
        (Ticker, 2) => cfg.ticker.singer_count.to_string(),
        (Ticker, 3) => cfg.ticker.show_all_singers.to_string(),
        (Ticker, 4) => cfg.ticker.show_empty_singers.to_string(),
        (Ticker, 5) => cfg.ticker.empty_next_text.clone(),
        (Ticker, 6) => cfg.ticker.empty_then_text.join("; "),
        (Ticker, 7) => cfg.ticker.show_request_button.to_string(),
        (Ticker, 8) => cfg.ticker.request_button_text.clone(),
        (Ticker, 9) => cfg.ticker.request_button_url.clone(),
        (Ticker, 10) => cfg.ticker.show_tiktok_live.to_string(),
        (Ticker, 11) => cfg.ticker.tiktok_button_text.clone(),
        (Ticker, 12) => cfg.ticker.tiktok_button_url.clone(),
        (Ticker, 13) => cfg.ticker.hero_now_label.clone(),
        (Ticker, 14) => cfg.ticker.hero_next_label.clone(),
        (Ticker, 15) => cfg.ticker.badge_now_text.clone(),
        (Ticker, 16) => cfg.ticker.badge_next_text.clone(),
        (QueueInfo, 0) => cfg.ticker.queue_info.show_at_start.to_string(),
        (QueueInfo, 1) => cfg.ticker.queue_info.show_every_n_singers.to_string(),
        (QueueInfo, 2) => cfg.ticker.queue_info.precise_duration.to_string(),
        (QueueInfo, 3) => cfg.ticker.queue_info.count_legend.clone(),
        (QueueInfo, 4) => cfg.ticker.queue_info.time_legend.clone(),
        (SingTime, 0) => cfg.ticker.sing_time.enabled.to_string(),
        (SingTime, 1) => cfg.ticker.sing_time.show_per_singer.to_string(),
        (SingTime, 2) => cfg.ticker.sing_time.show_in_banner.to_string(),
        (SingTime, 3) => cfg.ticker.sing_time.format.clone(),
        (Scroll, 0) => cfg.scroll.height.to_string(),
        (Scroll, 1) => cfg.scroll.bg.clone(),
        (Scroll, 2) => cfg.scroll.font.clone(),
        (Scroll, 3) => cfg.scroll.size.to_string(),
        (Scroll, 4) => cfg.scroll.speed.to_string(),
        (Scroll, 5) => cfg.scroll.color_now.clone(),
        (Scroll, 6) => cfg.scroll.color_next.clone(),
        (Scroll, 7) => cfg.scroll.color_up.clone(),
        (Scroll, 8) => cfg.scroll.color_singer.clone(),
        (Scroll, 9) => cfg.scroll.color_song.clone(),
        (Scroll, 10) => cfg.scroll.color_artist.clone(),
        (Scroll, 11) => cfg.scroll.color_info.clone(),
        (Scroll, 12) => cfg.scroll.label_now.clone(),
        (Scroll, 13) => cfg.scroll.label_next.clone(),
        (Scroll, 14) => cfg.scroll.label_up.clone(),
        (Scroll, 15) => cfg.scroll.label_info.clone(),
        (Ngrok, 0) => cfg.ngrok.enabled.to_string(),
        (Ngrok, 1) => cfg.ngrok.authtoken.clone(),
        (Ngrok, 2) => cfg.ngrok.domain.clone(),
        _ => String::new(),
    }
}

fn display_value(cfg: &Config, section: Section, idx: usize) -> String {
    match fields(section)[idx].kind {
        FieldKind::Bool => {
            if value_str(cfg, section, idx) == "true" { "On".into() } else { "Off".into() }
        }
        FieldKind::List => {
            let n = list_ref(cfg, section, idx).map(|l| l.len()).unwrap_or(0);
            format!("{n} entries")
        }
        _ if section == Section::Ngrok && idx == 1 => {
            let v = value_str(cfg, section, idx);
            if v.is_empty() {
                "(unset — using NGROK_AUTHTOKEN env var)".to_string()
            } else {
                "•".repeat(v.chars().count().min(24))
            }
        }
        _ => value_str(cfg, section, idx),
    }
}

fn set_text(cfg: &mut Config, section: Section, idx: usize, text: &str) -> std::result::Result<(), String> {
    use Section::*;
    match (section, idx) {
        (Server, 0) => cfg.server.port = parse_num(text)?,
        (Server, 1) => cfg.server.bind_address = text.to_string(),
        (Ticker, 0) => cfg.ticker.upstream_url = text.to_string(),
        (Ticker, 1) => cfg.ticker.poll_interval_ms = parse_num(text)?,
        (Ticker, 2) => cfg.ticker.singer_count = parse_num(text)?,
        (Ticker, 5) => cfg.ticker.empty_next_text = text.to_string(),
        (Ticker, 8) => cfg.ticker.request_button_text = text.to_string(),
        (Ticker, 9) => cfg.ticker.request_button_url = text.to_string(),
        (Ticker, 11) => cfg.ticker.tiktok_button_text = text.to_string(),
        (Ticker, 12) => cfg.ticker.tiktok_button_url = text.to_string(),
        (Ticker, 13) => cfg.ticker.hero_now_label = text.to_string(),
        (Ticker, 14) => cfg.ticker.hero_next_label = text.to_string(),
        (Ticker, 15) => cfg.ticker.badge_now_text = text.to_string(),
        (Ticker, 16) => cfg.ticker.badge_next_text = text.to_string(),
        (QueueInfo, 1) => cfg.ticker.queue_info.show_every_n_singers = parse_num(text)?,
        (QueueInfo, 3) => cfg.ticker.queue_info.count_legend = text.to_string(),
        (QueueInfo, 4) => cfg.ticker.queue_info.time_legend = text.to_string(),
        (Scroll, 0) => cfg.scroll.height = parse_num(text)?,
        (Scroll, 1) => cfg.scroll.bg = text.to_string(),
        (Scroll, 2) => cfg.scroll.font = text.to_string(),
        (Scroll, 3) => cfg.scroll.size = parse_num(text)?,
        (Scroll, 4) => cfg.scroll.speed = parse_num(text)?,
        (Scroll, 5) => cfg.scroll.color_now = text.to_string(),
        (Scroll, 6) => cfg.scroll.color_next = text.to_string(),
        (Scroll, 7) => cfg.scroll.color_up = text.to_string(),
        (Scroll, 8) => cfg.scroll.color_singer = text.to_string(),
        (Scroll, 9) => cfg.scroll.color_song = text.to_string(),
        (Scroll, 10) => cfg.scroll.color_artist = text.to_string(),
        (Scroll, 11) => cfg.scroll.color_info = text.to_string(),
        (Scroll, 12) => cfg.scroll.label_now = text.to_string(),
        (Scroll, 13) => cfg.scroll.label_next = text.to_string(),
        (Scroll, 14) => cfg.scroll.label_up = text.to_string(),
        (Scroll, 15) => cfg.scroll.label_info = text.to_string(),
        (Ngrok, 1) => cfg.ngrok.authtoken = text.to_string(),
        (Ngrok, 2) => cfg.ngrok.domain = text.to_string(),
        _ => return Err("not a text field".to_string()),
    }
    Ok(())
}

fn parse_num<T: std::str::FromStr>(text: &str) -> std::result::Result<T, String> {
    text.trim().parse().map_err(|_| format!("\"{text}\" is not a valid number"))
}

fn toggle_bool(cfg: &mut Config, section: Section, idx: usize) {
    use Section::*;
    match (section, idx) {
        (Ticker, 3) => cfg.ticker.show_all_singers = !cfg.ticker.show_all_singers,
        (Ticker, 4) => cfg.ticker.show_empty_singers = !cfg.ticker.show_empty_singers,
        (Ticker, 7) => cfg.ticker.show_request_button = !cfg.ticker.show_request_button,
        (Ticker, 10) => cfg.ticker.show_tiktok_live = !cfg.ticker.show_tiktok_live,
        (QueueInfo, 0) => cfg.ticker.queue_info.show_at_start = !cfg.ticker.queue_info.show_at_start,
        (QueueInfo, 2) => cfg.ticker.queue_info.precise_duration = !cfg.ticker.queue_info.precise_duration,
        (SingTime, 0) => cfg.ticker.sing_time.enabled = !cfg.ticker.sing_time.enabled,
        (SingTime, 1) => cfg.ticker.sing_time.show_per_singer = !cfg.ticker.sing_time.show_per_singer,
        (SingTime, 2) => cfg.ticker.sing_time.show_in_banner = !cfg.ticker.sing_time.show_in_banner,
        (Ngrok, 0) => cfg.ngrok.enabled = !cfg.ngrok.enabled,
        _ => {}
    }
}

fn cycle_choice(cfg: &mut Config, section: Section, idx: usize, choices: &[&str], forward: bool) {
    if let (Section::SingTime, 3) = (section, idx) {
        let pos = choices.iter().position(|c| *c == cfg.ticker.sing_time.format).unwrap_or(0);
        let len = choices.len();
        let next = if forward { (pos + 1) % len } else { (pos + len - 1) % len };
        cfg.ticker.sing_time.format = choices[next].to_string();
    }
}

fn list_ref(cfg: &Config, section: Section, idx: usize) -> Option<&Vec<String>> {
    match (section, idx) {
        (Section::Ticker, 6) => Some(&cfg.ticker.empty_then_text),
        _ => None,
    }
}

fn list_mut(cfg: &mut Config, section: Section, idx: usize) -> Option<&mut Vec<String>> {
    match (section, idx) {
        (Section::Ticker, 6) => Some(&mut cfg.ticker.empty_then_text),
        _ => None,
    }
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
}

struct App {
    draft: Config,
    dirty: bool,
    section_idx: usize,
    cursor: [usize; 6],
    mode: Mode,
    status: String,
    startup_info: Option<StartupInfo>,
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
) -> Result<()> {
    let _guard = TerminalGuard::new()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    let initial = shared_cfg.read().unwrap().clone();
    let mut app = App {
        draft: initial,
        dirty: false,
        section_idx: 0,
        cursor: [0; 6],
        mode: Mode::Normal,
        status: "Loaded.".to_string(),
        startup_info: None,
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

fn step(
    app: &mut App,
    key: KeyEvent,
    mode: Mode,
    shared_cfg: &SharedConfig,
    config_path: &Path,
) -> Result<(Mode, bool)> {
    let is_text_input = matches!(mode, Mode::Editing { .. } | Mode::ListEditing { .. });
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
                let section = SECTIONS[app.section_idx];
                let idx = app.cursor[app.section_idx];
                match set_text(&mut app.draft, section, idx, &buffer) {
                    Ok(()) => {
                        app.dirty = true;
                        app.status = format!("Set {} (unsaved).", fields(section)[idx].label);
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
            let len = list_ref(&app.draft, Section::Ticker, 6).map(|l| l.len()).unwrap_or(0);
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => Ok((Mode::Normal, false)),
                KeyCode::Up | KeyCode::Char('k') => Ok((Mode::ListView { cursor: cursor.saturating_sub(1) }, false)),
                KeyCode::Down | KeyCode::Char('j') => {
                    let next = if cursor + 1 < len { cursor + 1 } else { cursor };
                    Ok((Mode::ListView { cursor: next }, false))
                }
                KeyCode::Enter if len > 0 => {
                    let buf = list_ref(&app.draft, Section::Ticker, 6)
                        .and_then(|l| l.get(cursor))
                        .cloned()
                        .unwrap_or_default();
                    Ok((Mode::ListEditing { cursor, buffer: buf, is_new: false }, false))
                }
                KeyCode::Char('a') => Ok((Mode::ListEditing { cursor: len, buffer: String::new(), is_new: true }, false)),
                KeyCode::Char('d') if len > 0 => {
                    if let Some(l) = list_mut(&mut app.draft, Section::Ticker, 6) {
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
                let len = list_ref(&app.draft, Section::Ticker, 6).map(|l| l.len()).unwrap_or(0);
                let clamped = if len == 0 { 0 } else { cursor.min(len - 1) };
                Ok((Mode::ListView { cursor: clamped }, false))
            }
            KeyCode::Enter => {
                if let Some(l) = list_mut(&mut app.draft, Section::Ticker, 6) {
                    if is_new {
                        l.push(buffer.clone());
                    } else if let Some(slot) = l.get_mut(cursor) {
                        *slot = buffer.clone();
                    }
                }
                app.dirty = true;
                app.status = "Entry saved (unsaved).".into();
                Ok((Mode::ListView { cursor }, false))
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

        Mode::Normal => {
            let section = SECTIONS[app.section_idx];
            let count = field_count(section);
            match key.code {
                KeyCode::Char('q') => Ok(request_quit(app)),
                KeyCode::Tab | KeyCode::Right => {
                    app.section_idx = (app.section_idx + 1) % SECTIONS.len();
                    Ok((Mode::Normal, false))
                }
                KeyCode::BackTab | KeyCode::Left => {
                    app.section_idx = (app.section_idx + SECTIONS.len() - 1) % SECTIONS.len();
                    Ok((Mode::Normal, false))
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    if app.cursor[app.section_idx] > 0 {
                        app.cursor[app.section_idx] -= 1;
                    }
                    Ok((Mode::Normal, false))
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if app.cursor[app.section_idx] + 1 < count {
                        app.cursor[app.section_idx] += 1;
                    }
                    Ok((Mode::Normal, false))
                }
                KeyCode::Enter | KeyCode::Char(' ') => {
                    let idx = app.cursor[app.section_idx];
                    match fields(section)[idx].kind {
                        FieldKind::Bool => {
                            toggle_bool(&mut app.draft, section, idx);
                            app.dirty = true;
                            app.status = "Toggled (unsaved).".into();
                            Ok((Mode::Normal, false))
                        }
                        FieldKind::Choice(choices) => {
                            cycle_choice(&mut app.draft, section, idx, choices, true);
                            app.dirty = true;
                            app.status = "Changed (unsaved).".into();
                            Ok((Mode::Normal, false))
                        }
                        FieldKind::List => Ok((Mode::ListView { cursor: 0 }, false)),
                        FieldKind::Text | FieldKind::Num => {
                            let seed = value_str(&app.draft, section, idx);
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
        .constraints([Constraint::Length(3), Constraint::Min(5), Constraint::Length(6), Constraint::Length(2)])
        .split(area);

    draw_tabs(f, chunks[0], app);
    draw_body(f, chunks[1], app);
    draw_status(f, chunks[2], app);
    draw_footer(f, chunks[3], app);

    match &app.mode {
        Mode::Editing { buffer, error } => draw_edit_popup(f, area, app, buffer, error.as_deref()),
        Mode::ListView { cursor } => draw_list_popup(f, area, app, *cursor, None),
        Mode::ListEditing { cursor, buffer, .. } => draw_list_popup(f, area, app, *cursor, Some(buffer)),
        Mode::ConfirmQuit => draw_confirm_popup(f, area),
        Mode::Normal => {}
    }
}

fn draw_status(f: &mut Frame, area: Rect, app: &App) {
    let lines: Vec<Line> = match &app.startup_info {
        None => vec![Line::from(Span::styled("Starting server…", Style::default().fg(Color::Gray)))],
        Some(info) => {
            let label = |s: &'static str| Span::styled(format!("{s:<11}"), Style::default().fg(Color::Gray));

            let mut lines = vec![
                Line::from(vec![
                    label("Dashboard"),
                    Span::raw(info.local_dashboard_url.clone()),
                    Span::raw("    "),
                    Span::styled("Scroll  ", Style::default().fg(Color::Gray)),
                    Span::raw(info.local_scroll_url.clone()),
                ]),
                Line::from(vec![
                    label("LAN"),
                    Span::raw(info.lan_dashboard_url.clone()),
                    Span::raw("    "),
                    Span::styled("Scroll  ", Style::default().fg(Color::Gray)),
                    Span::raw(info.lan_scroll_url.clone()),
                ]),
            ];

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

            lines
        }
    };

    let block = Block::default().borders(Borders::ALL).title("Status");
    f.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: true }), area);
}

fn draw_tabs(f: &mut Frame, area: Rect, app: &App) {
    let titles: Vec<Line> = SECTIONS.iter().map(|s| Line::from(s.title())).collect();
    let tabs = Tabs::new(titles)
        .select(app.section_idx)
        .block(Block::default().borders(Borders::ALL).title("kroak-time-ticker config"))
        .highlight_style(Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD));
    f.render_widget(tabs, area);
}

fn draw_body(f: &mut Frame, area: Rect, app: &App) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    let section = SECTIONS[app.section_idx];
    let cursor = app.cursor[app.section_idx];
    let metas = fields(section);

    let items: Vec<ListItem> = metas
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let selected = i == cursor;
            let label_style = if selected {
                Style::default().add_modifier(Modifier::BOLD).fg(Color::Yellow)
            } else {
                Style::default()
            };
            let label = format!("{}{}", m.label, if m.restart { " *" } else { "" });
            let mut spans = vec![Span::styled(format!("{label:<24}"), label_style)];

            if is_color_field(section, i) {
                if let Some(c) = parse_css_color(&value_str(&app.draft, section, i)) {
                    spans.push(Span::styled("■ ", Style::default().fg(c)));
                }
            }
            spans.push(Span::raw(display_value(&app.draft, section, i)));
            ListItem::new(Line::from(spans))
        })
        .collect();

    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(section.title()));
    f.render_widget(list, cols[0]);

    let meta = &metas[cursor];
    let restart_note = if meta.restart { "\n\n* takes effect after a restart of kroak-time-ticker." } else { "" };
    let help = Paragraph::new(format!("{}{restart_note}", meta.help))
        .wrap(Wrap { trim: true })
        .block(Block::default().borders(Borders::ALL).title("About this setting"));
    f.render_widget(help, cols[1]);
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let dirty_tag = if app.dirty { " [UNSAVED]" } else { "" };
    let text = format!(
        "{}{}\n←/→ tab  ↑/↓ field  Enter edit/toggle  s save  q quit",
        app.status, dirty_tag
    );
    f.render_widget(Paragraph::new(text), area);
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
    let section = SECTIONS[app.section_idx];
    let idx = app.cursor[app.section_idx];
    let label = fields(section)[idx].label;
    let mask = section == Section::Ngrok && idx == 1;
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

fn draw_list_popup(f: &mut Frame, area: Rect, app: &App, cursor: usize, editing: Option<&str>) {
    let popup = centered_rect(70, 60, area);
    f.render_widget(Clear, popup);

    let rows = if editing.is_some() {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(3)])
            .split(popup)
    } else {
        Layout::default().direction(Direction::Vertical).constraints([Constraint::Min(3)]).split(popup)
    };

    let list = list_ref(&app.draft, Section::Ticker, 6).cloned().unwrap_or_default();
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

    let title = "empty_then_text — ↑/↓ select · Enter edit · a add · d delete · Esc back";
    f.render_widget(List::new(items).block(Block::default().borders(Borders::ALL).title(title)), rows[0]);

    if let Some(buf) = editing {
        let edit_block = Block::default().borders(Borders::ALL).title("Entry text (Enter save · Esc cancel)");
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
