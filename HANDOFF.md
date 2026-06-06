# Session handoff

**Branch**: `main`
**Paused**: 2026-06-07

## Done this session
- Added `is_playing: bool` to `KaraokeState` in `src/db.rs` — detects whether the current singer is actively singing
- Updated `src/static/ticker.html` and `src/static/list.html` to use `is_playing`: "Now Singing" only shows when a song is actively playing; "Up Next" shows the top-of-rotation singer between songs
- Fixed INI file lookup in `read_current_singer_id` (`src/db.rs`) — now checks `openkj.ini` first (correct filename on Windows), with `openkj2.ini` and `openkj2-unstable.ini` as fallbacks
- Fixed `discover_data_dir` in `src/config.rs` to also match on `openkj.ini`
- Fixed `is_playing` detection query — original approach tried to match `historySongs.historySinger` (an INT FK) against a singer name string, and used `historySongs.songid` (TEXT disc ID) to join against `queueSongs.song` (INT). Fixed join chain was: `historySingers(name→id) → historySongs(filepath) → dbSongs(path→songid) → queueSongs`
- Final fix: dropped the singer-name lookup entirely. Now uses the globally most-recent `historySongs` entry (by `lastplay`) and checks if that filepath is still unplayed in the current singer's queue — works for first-time singers with no history
- Released alpha.1 through alpha.8 to GitHub; binary is Windows x86_64 cross-compiled via `x86_64-pc-windows-gnu` target

## In progress / next steps
- alpha.8 has not been confirmed working yet — user needs to test it
- If `is_playing` still doesn't work, the next debug step is to check the `is_playing: most recent song globally` log line to verify the filepath being found, and the `unplayed_count` log line
- Debug logging (default level `debug`) and detailed INI/history logs are still in the code — once confirmed working, strip the debug logging back to `info` and do a proper release

## Context to carry forward
- OpenKJ on Windows stores settings in `%APPDATA%\OpenKJ2\openkj.ini` (not `openkj2.ini` as originally assumed)
- `historySongs.historySinger` is an INT FK to `historySingers.id`, not the singer name
- `historySongs.songid` is a TEXT disc ID (e.g. "SF-BW0001"), NOT the integer `dbSongs.songid`
- The link between history and queue runs through `dbSongs.path` (filepath): `historySongs.filepath = dbSongs.path`, then `dbSongs.songid = queueSongs.song`
- OpenKJ source is at `/Users/wolf/Projects/openkj2/` — useful for schema questions
- Cross-compilation config for Windows is in `.cargo/config.toml` (mingw linker); target already installed
- `currentRotationPosition` in the INI stores the singer's `singerid` integer from `rotationSingers`
