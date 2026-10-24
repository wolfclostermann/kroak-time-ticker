# Todo

## kroak-time `/api/state` additions (reference for the items below)

From kroak-time branch `trunk/ticker-api-additions` (full spec in kroak-time's `docs/API.md`).
All are additive; an older kroak-time simply omits them, so read them with `#[serde(default)]`.

Top-level fields:

| Field | Type | Meaning |
|-------|------|---------|
| `song_position_ms` | integer \| null | How far into the song on stage playback is, in ms of *song time* (same value as kroak-time's progress bar). Sent while playing **and while paused** (`is_playing` tells them apart). `null` while idle, and for the first moment of a song before the player reports a position. Can be up to ~1 s old (sampled every 500 ms, snapshot rebuilt every 500 ms on a separate timer). |
| `song_duration_ms` | integer \| null | Length of the song on stage, in ms of song time, as reported by the player. Always present or `null` **together with** `song_position_ms`. |
| `playback_rate` | number \| null | Speed the song on stage plays at, from the tempo control: `1.0` normal, `0.9` = 90 %, range `0.5`–`1.5`. `null` while idle. Can change mid-song. Song time runs `playback_rate` times faster than real time. |

New field on every Singer object (`current_singer`, `next_up`, `rotation[]`, `waiting[]`):

| Field | Type | Meaning |
|-------|------|---------|
| `next_song_duration_ms` | integer \| null | Catalog length of the singer's next unsung song, in ms at normal speed (not adjusted for the tempo it's queued at). `null` with nothing queued or when the catalog has no length. For `current_singer` during a song this *is* the song on stage, so it's available straight away, before the player reports `song_duration_ms`. |

Useful formulas:
- Local clock between polls (while `is_playing`): `position = song_position_ms + elapsed_real_ms * playback_rate`.
- Real time left: `(song_duration_ms - position) / playback_rate`.
- Before the player reports anything (`is_playing` true, `song_position_ms` null): position 0, length from
  `current_singer.next_song_duration_ms`.

- [x] **Use kroak-time's new `playback_rate` field so the song clock stays right when the tempo is changed:**
  **Done:** steps 1–3 as described, plus the start-of-song fix below: a rate with no position shows
  0:00 against `current_singer.next_song_duration_ms`, and the player's real length replaces that
  estimate without moving the clock. Two deviations:
  - **Step 4 (drift check) kept at +0 / −3000 ms, not ±1500.** kroak-time's value is up to ~1 s
    old, but the ticker re-polls it only every 1.5 s, so what the page sees can be ~2.5 s old. A
    −1500 limit would make the clock tick backwards whenever a staler-than-usual sample arrived.
  - **"Clock keeps running while paused" didn't apply.** A paused song has `is_playing: false`,
    which the page treats as between songs: there's no "Now Singing" singer, so the progress row is
    hidden and the clock stopped. Only relevant if the row should later stay visible, frozen, while
    paused.

  Original note — kroak-time's `/api/state` now sends `playback_rate` (number | null) next to `song_position_ms` /
  `song_duration_ms` (kroak-time branch `trunk/ticker-api-additions`, documented in its `docs/API.md`).
  It's the tempo control as a rate: `1.0` = normal, `0.9` = 90 %, range `0.5`–`1.5`, `null` while idle,
  and it can change mid-song. Position and duration are in *song* time, so at any rate other than 1.0
  the singer page's clock is wrong today:
  - `progressNow()` in `src/static/list.html` counts forward one song-ms per real ms. At 0.8 it runs
    ahead and snaps back every ~3 s (the `drift < -3000` resync); at 1.2 it falls behind and jumps
    forward on every poll (`drift > 0`).
  - The "−m:ss" remaining time shows song time, not real time — at 0.8 a song with 2:00 of song time
    left actually has 2:30 to go.

  What to change:
  1. **Pass the field through.** The ticker re-serves kroak-time's state through the typed
     `KaraokeState` struct (`src/state.rs`), so a field missing from that struct gets dropped
     without any error. Add `#[serde(default)] pub playback_rate: Option<f64>` there (an older
     kroak-time that doesn't send it should be read as 1.0), and
     `#[serde(default)] pub next_song_duration_ms: Option<u64>` to `Singer` (same file).
  2. **Count at the rate.** In `progressNow()`, advance by `(performance.now() - progress.at) * rate`,
     storing `rate` in `progress` on each sync (`s.playback_rate ?? 1`). Re-anchor (`posMs`/`at`)
     whenever the rate changes, so the part counted at the old rate isn't recalculated at the new one.
  3. **Show remaining time in real time:** `(durMs - p) / rate`. Elapsed and the progress bar can stay
     in song time (they then match kroak-time's own progress bar). Note the existing comment
     "elapsed + remaining always adds up to the length" stops being true at rate ≠ 1.0, so either
     update it or decide whether elapsed should be real time too.
  4. **Tighten the drift check.** With the rate applied, local counting should track kroak-time
     closely. kroak-time's value can be up to ~1 s old (it samples every 500 ms and rebuilds the
     snapshot every 500 ms on a separate timer), so allow about −1500 / +1500 ms before resyncing
     rather than the current asymmetric 0 / −3000.

  Related, spotted while reading `syncProgress()`:
  - **The clock keeps running while paused.** `progress.running` is set to true whenever
    `song_position_ms` is a number, but kroak-time also sends a position while paused (with
    `is_playing: false`). The page's clock creeps forward during a pause and snaps back every ~3 s.
    Only count forward while `s.is_playing`; while paused, show the reported position frozen.
  - **"Playing, no position yet" is the start of a song.** For the first moment of a song kroak-time
    sends both `song_position_ms` and `song_duration_ms` as `null` (they're always both present or
    both null). The page currently hides the progress row then, so it flickers in a second after
    each song starts. Showing 0:00 instead, with the length from
    `current_singer.next_song_duration_ms`, would avoid that.
