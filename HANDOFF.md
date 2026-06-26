# Session handoff

**Branch**: `main`
**Paused**: 2026-06-26

## Done this session

- Updated `src/static/ticker.html`: changed the "UP NEXT" label to "THEN" (line 169) so the idle ticker reads "NEXT … THEN …" instead of "NEXT … UP NEXT …"
- Updated `src/static/ticker.html`: replaced `.up-entry` class with `.up-singer` (white, weight 600) and `.up-song` (grey #888, italic, 0.85em) so singer names are prominent and song titles are subordinate in the queued-singers segment

## In progress / next steps

- Changes are correct in source; user needs to verify visually in browser after hard refresh (Cmd+Shift+R) — the old rendering was a browser cache issue, not a code issue

## Context to carry forward

- HTML is embedded at compile time via `include_str!` in `src/server.rs:20` — editing `ticker.html` requires a recompile (`cargo build` / `cargo run`) before changes appear; the server does not hot-reload static files
- Ticker server runs on port 8080 (configured in `kroak-time-ticker.toml`); upstream kroak-time API is at `http://localhost:7070/api/state`
- `kroak-time-ticker.toml` is untracked — it is the local config file and should stay out of git
