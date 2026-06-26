# kroak-time `/api/state` endpoint spec

kroak-time-ticker polls this endpoint every 1500 ms to drive its OBS overlays. Implement it in kroak-time as described below.

---

## Endpoint

```
GET /api/state
Content-Type: application/json
```

No authentication. No query parameters. Always returns JSON.

---

## Response shape

```json
{
  "current_singer": <Singer | null>,
  "next_up":        <Singer | null>,
  "rotation":       <Singer[]>,
  "singer_count":   <integer>,
  "is_playing":     <boolean>,
  "status":         <string>
}
```

### Top-level fields

| Field | Type | Description |
|-------|------|-------------|
| `current_singer` | Singer \| null | The singer who is currently up at the microphone (their turn in rotation). `null` when the rotation is empty. |
| `next_up` | Singer \| null | The singer immediately after `current_singer` in rotation order. If `current_singer` is last, wrap to the first singer. `null` when there is only one singer or the rotation is empty. |
| `rotation` | Singer[] | All singers in rotation order, starting from position 1. Always the full list — the ticker decides how many to display. |
| `singer_count` | integer | A display hint passed through to the ticker unchanged. Set it to however many singers beyond NOW/NEXT you want shown in the overlay (default: 8). |
| `is_playing` | boolean | `true` when a song is actively being performed by `current_singer`. `false` between songs or when the rotation is idle. |
| `status` | string | Health string (see Status values below). |

---

### Singer object

```json
{
  "name":             <string>,
  "next_song_artist": <string | null>,
  "next_song_title":  <string | null>,
  "is_current":       <boolean>
}
```

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Display name of the singer. |
| `next_song_artist` | string \| null | Artist of the singer's next queued song, or `null` if their queue is empty. |
| `next_song_title` | string \| null | Title of the singer's next queued song, or `null` if their queue is empty. |
| `is_current` | boolean | `true` only for the singer who matches `current_singer`. Set to `false` for all others, including `next_up`. |

---

### Status values

| Value | Meaning |
|-------|---------|
| `"ok"` | Normal operation — rotation data is available. |
| `"not_ready"` | kroak-time has started but data is not yet available (e.g., still loading). |
| `"database_not_found"` | The underlying data source cannot be located. |

The ticker displays a "waiting" message for any status other than `"ok"`.

---

## Semantics

### `is_playing` vs `current_singer`

These are independent:

- `current_singer` is whoever is **up next / at the mic** — it changes when the host advances the rotation.
- `is_playing` reflects whether a song is **actively being performed** right now.

The ticker uses both together:

| `is_playing` | `current_singer` | Ticker shows |
|---|---|---|
| `true` | Alice | "NOW Alice · Song Title … NEXT Bob" |
| `false` | Alice | "NEXT Alice · Song Title …" (between songs) |
| `false` | null | Idle / no rotation |

### Rotation ordering

`rotation` must be sorted by position (slot order), not alphabetically. `current_singer` will typically be `rotation[0]` or wherever the host has set the current position. `next_up` is the singer immediately after `current_singer` in that ordered list.

### When the queue is empty

If a singer has no songs queued, set `next_song_artist` and `next_song_title` to `null`. The ticker will show the singer's name without a song title.

---

## Example responses

**Normal — song playing:**
```json
{
  "current_singer": {
    "name": "Alice",
    "next_song_artist": "The Beatles",
    "next_song_title": "Let It Be",
    "is_current": true
  },
  "next_up": {
    "name": "Bob",
    "next_song_artist": "Journey",
    "next_song_title": "Don't Stop Believin'",
    "is_current": false
  },
  "rotation": [
    { "name": "Alice", "next_song_artist": "The Beatles", "next_song_title": "Let It Be", "is_current": true },
    { "name": "Bob",   "next_song_artist": "Journey",     "next_song_title": "Don't Stop Believin'", "is_current": false },
    { "name": "Carol", "next_song_artist": null,           "next_song_title": null, "is_current": false }
  ],
  "singer_count": 8,
  "is_playing": true,
  "status": "ok"
}
```

**Between songs (rotation advanced, no song started yet):**
```json
{
  "current_singer": { "name": "Bob", "next_song_artist": "Journey", "next_song_title": "Don't Stop Believin'", "is_current": true },
  "next_up":        { "name": "Carol", "next_song_artist": null, "next_song_title": null, "is_current": false },
  "rotation": [ ... ],
  "singer_count": 8,
  "is_playing": false,
  "status": "ok"
}
```

**Not ready:**
```json
{
  "current_singer": null,
  "next_up": null,
  "rotation": [],
  "singer_count": 8,
  "is_playing": false,
  "status": "not_ready"
}
```
