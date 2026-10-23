# Song requests: catalog format and kroak-time setup

Turning on `[requests] enabled = true` changes the dashboard's (`/`) request button. It no
longer links out; it opens a request form with Your name / Artist / Title / Key, each with
search-as-you-type over your catalog. Anything not in the catalog can be typed in by hand.

## 1. The catalog

Set `[requests] catalog_path` to one of these:

| Format | What it looks like |
|---|---|
| **CSV** (`.csv`) | Header row with `artist` and `title` columns (any order, any case). Other columns are ignored, so an export with `song_id`, `vendor` and so on is fine. Use standard CSV quoting for commas: `"Earth, Wind & Fire",September`. |
| **TSV** (`.tsv`, or `.txt` whose header contains a tab) | The same, tab-separated. This is what "Save as → Text (tab delimited)" gives you from a spreadsheet. |
| **JSON** (`.json`) | `[{"artist": "Kings of Leon", "title": "Sex on Fire"}, ...]` |
| **kroak-time's database** (`.redb`) | Point it at kroak-time's `catalog.redb`. Songs marked bad or dropped are skipped. |

Minimal CSV:

```csv
artist,title
Kings of Leon,Sex on Fire
"Earth, Wind & Fire",September
Oasis,Wonderwall
```

Leaving `catalog_path` blank uses kroak-time's default `catalog.redb`
(Windows: `%APPDATA%\kroak-time\config\catalog.redb`) if it exists. You only need to set it
if kroak-time uses a custom `data_dir`, or if you'd rather maintain a CSV.

On load, the catalog is cleaned up:
- Surrounding whitespace is trimmed.
- Artist spellings that differ only in case ("Kings Of Leon" / "Kings of Leon") are merged
  under the most common spelling.
- Case-insensitive duplicate songs (the same track from two vendors) appear once.

**Reloading:** the file is re-read automatically when its modification time changes. You
don't need to restart.

**When kroak-time has the redb open:** the ticker reads from a temporary copy, so it never
takes kroak-time's lock. If even the copy fails (redb's Windows lock blocks all reads while
the database is held open), the ticker serves the last good copy from `catalog-cache.json`.
It tries again the next time the file changes.

The page never downloads the catalog during page load. It fetches `/api/catalog` (gzipped
compact JSON, about 215 KB for about 19k songs) only after the page has loaded and the
browser is idle, or straight away if the form is opened first. Repeat visits revalidate
with an ETag and get a 304.

## 2. Sending requests to kroak-time

kroak-time pulls requests from request sources, the same way it pulls from wcis-helper. The
API the ticker serves is **not hard-coded**: it's defined by `[requests] contract_path`
(default `request-source.toml`, created on first run). That file is a kroak-time
request-source definition. **The same file configures both sides**:

1. Start the ticker once with requests enabled, so it writes `request-source.toml`.
2. Copy that file into kroak-time's `request-sources` folder
   (Windows: `%APPDATA%\kroak-time\config\request-sources\`) and restart kroak-time.
3. Requests now show up in kroak-time's Incoming panel. Adding or dismissing one removes it
   from the ticker.

If you edit the contract (port, paths, command names, field names), copy it to kroak-time
again so both sides agree. The ticker reads it like this:
- `[config].base_url` is where the ticker listens. Keep it on `127.0.0.1`: the protocol has
  no authentication.
- An operation is recognised by the literal (non-placeholder) values in its `body`.
- `{request_id}` and `{accepting}` are bound from the incoming body.
- Other placeholders such as `"{api_key}"` accept any value.
- Responses are built at the `extract` paths.
- `[config] requested_by_first_name` / `requested_by_last_name` set what kroak-time shows
  as "requested by". Placeholders: `{device_code}`, `{device_id}`, `{fingerprint}`,
  `{singer}`. The default is `Phone K3F9`.

## 3. Identifying phones

Each phone gets a random device id the first time it sends a request. The id is kept in
localStorage plus a cookie as a fallback. Every request also carries a browser fingerprint
(canvas, WebGL renderer, screen, timezone, languages, and so on).

- **kroak-time** shows a 4-character code derived from the device id, so two requests under
  different names from the same phone carry the same code.
- **`song-requests.json`** keeps the full `device_id` and `fingerprint` for each request
  still waiting.

The fingerprint is recorded but deliberately **not** used as the id: phones of the same
model on the same OS version (two iPhones, say) often fingerprint identically, which would
merge different people. Use it as a cross-check if someone clears their browser data.

The name a singer enters is remembered on their phone the same way and pre-filled next
time. They can still change it to request for a friend.

`[requests] max_pending_per_device` (default 5) limits how many waiting requests one phone
can have. kroak-time's `setAccepting` closes requests from the page entirely.
