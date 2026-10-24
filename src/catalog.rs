//! Song catalog for the singer-facing request form.
//!
//! Loaded from `[requests] catalog_path`, which may be:
//!
//! - a `.csv` / `.tsv` / `.txt` file with an `artist` and a `title` column
//!   (header row required; extra columns are ignored — see CATALOG_FORMAT.md),
//! - a `.json` file: an array of `{"artist": "...", "title": "..."}` objects,
//! - kroak-time's own `catalog.redb`.
//!
//! redb takes an exclusive lock on the database while kroak-time has it open
//! (a mandatory whole-file lock on Windows, so it can't even be copied), so
//! the redb path is read from a temporary copy whenever that's possible, and
//! every successful load is also written to a cache file next to the config.
//! If kroak-time has the catalog locked when the ticker starts, the last
//! cached snapshot is served instead; the load is retried whenever the
//! catalog file's modification time changes.
//!
//! The catalog is served to browsers as one pre-serialized JSON blob:
//! `{"artists": [...], "songs": [[artist_index, "title"], ...]}` — artist
//! names are stored once, which roughly halves the payload for a typical
//! karaoke library where each artist has several songs.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::SystemTime;

/// Where a successful load is cached, so the catalog survives kroak-time
/// holding its database locked across a ticker restart.
pub const CACHE_FILE: &str = "catalog-cache.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct Song {
    pub artist: String,
    pub title: String,
}

/// The catalog as served: pre-serialized JSON plus an ETag over it.
pub struct Served {
    pub json: bytes::Bytes,
    pub etag: String,
    pub song_count: usize,
}

#[derive(Default)]
struct Inner {
    served: Option<Arc<Served>>,
    /// The catalog path + mtime of the last load *attempt* (successful or
    /// not), so a locked redb isn't re-tried on every request — only once
    /// the file has changed again.
    attempted: Option<(PathBuf, Option<SystemTime>)>,
    last_error: Option<String>,
}

#[derive(Clone, Default)]
pub struct CatalogStore {
    inner: Arc<RwLock<Inner>>,
    /// Guards against two concurrent requests both kicking off a reload.
    loading: Arc<std::sync::Mutex<()>>,
}

impl CatalogStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn current(&self) -> Option<Arc<Served>> {
        self.inner.read().unwrap().served.clone()
    }

    pub fn last_error(&self) -> Option<String> {
        self.inner.read().unwrap().last_error.clone()
    }

    /// Re-load the catalog if `path` (or its modification time) differs from
    /// the last attempt. Blocking — call from `spawn_blocking`. On the very
    /// first call, falls back to the on-disk cache if the source can't be
    /// read (typically: kroak-time has its redb open).
    pub fn refresh(&self, configured_path: &str) {
        let _guard = self.loading.lock().unwrap();

        let Some(path) = resolve_path(configured_path) else {
            let mut inner = self.inner.write().unwrap();
            if inner.served.is_none() {
                inner.served = load_cache().map(Arc::new);
            }
            inner.last_error = Some("No catalog_path set and kroak-time's catalog.redb wasn't found".into());
            return;
        };
        let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        let key = (path.clone(), mtime);
        if self.inner.read().unwrap().attempted.as_ref() == Some(&key) {
            return;
        }

        let result = load(&path);
        let mut inner = self.inner.write().unwrap();
        inner.attempted = Some(key);
        match result {
            Ok(songs) => {
                tracing::info!("Loaded {} catalog songs from {}", songs.len(), path.display());
                save_cache(&songs);
                inner.served = Some(Arc::new(serve(&songs)));
                inner.last_error = None;
            }
            Err(e) => {
                let msg = format!("{}: {e:#}", path.display());
                tracing::warn!("Catalog load failed: {msg}");
                if inner.served.is_none() {
                    if let Some(cached) = load_cache() {
                        tracing::info!("Serving cached catalog ({} songs) from {CACHE_FILE}", cached.song_count);
                        inner.served = Some(Arc::new(cached));
                    }
                }
                inner.last_error = Some(msg);
            }
        }
    }
}

/// `configured` if non-empty, else kroak-time's default catalog location
/// (only if it exists).
pub fn resolve_path(configured: &str) -> Option<PathBuf> {
    let configured = configured.trim();
    if !configured.is_empty() {
        return Some(crate::paths::resolve(configured));
    }
    default_kroak_time_catalog().filter(|p| p.exists())
}

/// kroak-time keeps `catalog.redb` in its confy config directory unless a
/// custom `data_dir` is set in its settings.
fn default_kroak_time_catalog() -> Option<PathBuf> {
    Some(crate::paths::app_config_dir("kroak-time")?.join("catalog.redb"))
}

pub fn load(path: &Path) -> Result<Vec<Song>> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let songs = match ext.as_str() {
        "redb" => load_redb(path)?,
        "json" => load_json(path)?,
        "tsv" => load_delimited(path, b'\t')?,
        "csv" | "txt" => {
            // .txt is often tab-separated (spreadsheet "save as text") —
            // sniff the header line rather than guessing from the extension.
            let head = std::fs::read_to_string(path)
                .with_context(|| format!("reading {}", path.display()))?;
            let first = head.lines().next().unwrap_or("");
            let delim = if first.contains('\t') { b'\t' } else { b',' };
            load_delimited(path, delim)?
        }
        _ => bail!("unsupported catalog file type {ext:?} (expected .csv, .tsv, .txt, .json or .redb)"),
    };
    Ok(normalize(songs))
}

/// Trim, drop blanks, merge artist spellings that differ only in case
/// ("Kings Of Leon" / "Kings of Leon" — different vendors) under the most
/// common one, drop case-insensitive duplicate songs, sort by artist then
/// title.
fn normalize(songs: Vec<Song>) -> Vec<Song> {
    let songs: Vec<Song> = songs
        .into_iter()
        .map(|s| Song { artist: s.artist.trim().to_string(), title: s.title.trim().to_string() })
        .filter(|s| !s.title.is_empty())
        .collect();

    // lowercase artist -> spelling -> count
    let mut spellings: BTreeMap<String, BTreeMap<&str, usize>> = BTreeMap::new();
    for s in &songs {
        *spellings.entry(s.artist.to_lowercase()).or_default().entry(&s.artist).or_default() += 1;
    }
    let canonical: BTreeMap<String, String> = spellings
        .iter()
        .map(|(lower, counts)| {
            // Ties go to the more capitalised spelling ("Kings of Leon" over
            // "kings of leon").
            let best = counts
                .iter()
                .max_by_key(|(s, n)| (**n, s.chars().filter(|c| c.is_uppercase()).count()))
                .map(|(s, _)| s.to_string())
                .unwrap_or_default();
            (lower.clone(), best)
        })
        .collect();

    let mut seen = HashSet::new();
    let mut out: Vec<Song> = songs
        .iter()
        .map(|s| Song { artist: canonical[&s.artist.to_lowercase()].clone(), title: s.title.clone() })
        .filter(|s| seen.insert((s.artist.to_lowercase(), s.title.to_lowercase())))
        .collect();
    out.sort_by(|a, b| {
        (a.artist.to_lowercase(), a.title.to_lowercase()).cmp(&(b.artist.to_lowercase(), b.title.to_lowercase()))
    });
    out
}

fn load_delimited(path: &Path, delimiter: u8) -> Result<Vec<Song>> {
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .flexible(true)
        .trim(csv::Trim::All)
        .from_path(path)
        .with_context(|| format!("opening {}", path.display()))?;
    let headers = rdr.headers()?.clone();
    let col = |name: &str| headers.iter().position(|h| h.trim_start_matches('\u{feff}').eq_ignore_ascii_case(name));
    let (Some(ai), Some(ti)) = (col("artist"), col("title")) else {
        bail!("header row must contain \"artist\" and \"title\" columns (found: {:?})", headers.iter().collect::<Vec<_>>());
    };
    let mut songs = Vec::new();
    for rec in rdr.records() {
        let rec = rec?;
        songs.push(Song {
            artist: rec.get(ai).unwrap_or("").to_string(),
            title: rec.get(ti).unwrap_or("").to_string(),
        });
    }
    Ok(songs)
}

fn load_json(path: &Path) -> Result<Vec<Song>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&text).context("expected a JSON array of {\"artist\": ..., \"title\": ...} objects")
}

/// Prefix of kroak-time's `KaraokeSong` (src/domain/song.rs), bincode-encoded
/// in its `songs` table. bincode is positional and `bincode::deserialize`
/// allows trailing bytes, so only the fields up to `dropped` are declared —
/// that also covers older records written before `vendor` was appended.
/// `last_play` is a chrono `DateTime<Utc>`, which serializes as a string.
#[derive(Deserialize)]
#[allow(dead_code)]
struct RedbSong {
    id: i32,
    artist: String,
    artist_lower: String,
    title: String,
    title_lower: String,
    song_id: String,
    song_id_lower: String,
    duration_ms: i32,
    filename: String,
    path: String,
    search_string: String,
    plays: i32,
    last_play: Option<String>,
    bad: bool,
    dropped: bool,
}

const REDB_SONGS: redb::TableDefinition<i32, &[u8]> = redb::TableDefinition::new("songs");

fn load_redb(path: &Path) -> Result<Vec<Song>> {
    // Read from a copy: opening the original would take redb's exclusive
    // lock away from kroak-time (or fail if kroak-time holds it). On
    // Windows the copy itself fails while kroak-time is running.
    let tmp = std::env::temp_dir().join(format!("kroak-time-ticker-catalog-{}.redb", std::process::id()));
    std::fs::copy(path, &tmp).map_err(|e| {
        anyhow::anyhow!("couldn't copy the catalog ({e}) — kroak-time probably has it open; close kroak-time once to refresh the cached copy")
    })?;
    let result = (|| -> Result<Vec<Song>> {
        use redb::{ReadableTable, ReadableTableMetadata};
        let db = redb::Database::open(&tmp).context("opening catalog copy")?;
        let txn = db.begin_read()?;
        let table = txn.open_table(REDB_SONGS)?;
        let mut songs = Vec::with_capacity(table.len()? as usize);
        for row in table.iter()? {
            let (_, bytes) = row?;
            match bincode::deserialize::<RedbSong>(bytes.value()) {
                Ok(s) if !s.bad && !s.dropped => songs.push(Song { artist: s.artist, title: s.title }),
                Ok(_) => {}
                Err(e) => tracing::debug!("skipping undecodable catalog record: {e}"),
            }
        }
        Ok(songs)
    })();
    let _ = std::fs::remove_file(&tmp);
    result
}

fn serve(songs: &[Song]) -> Served {
    let mut artist_idx: BTreeMap<&str, usize> = BTreeMap::new();
    let mut artists: Vec<&str> = Vec::new();
    let mut rows: Vec<(usize, &str)> = Vec::with_capacity(songs.len());
    for s in songs {
        let i = *artist_idx.entry(&s.artist).or_insert_with(|| {
            artists.push(&s.artist);
            artists.len() - 1
        });
        rows.push((i, &s.title));
    }
    let json = serde_json::to_vec(&serde_json::json!({ "artists": artists, "songs": rows }))
        .expect("catalog is always serializable");
    let mut h = DefaultHasher::new();
    json.hash(&mut h);
    Served {
        etag: format!("\"{:016x}\"", h.finish()),
        json: bytes::Bytes::from(json),
        song_count: songs.len(),
    }
}

fn save_cache(songs: &[Song]) {
    match serde_json::to_vec(songs) {
        Ok(json) => {
            if let Err(e) = std::fs::write(crate::paths::file(CACHE_FILE), json) {
                tracing::warn!("Couldn't write {CACHE_FILE}: {e}");
            }
        }
        Err(e) => tracing::warn!("Couldn't serialize catalog cache: {e}"),
    }
}

fn load_cache() -> Option<Served> {
    let text = std::fs::read_to_string(crate::paths::file(CACHE_FILE)).ok()?;
    let songs: Vec<Song> = serde_json::from_str(&text).ok()?;
    Some(serve(&songs))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_tmp(name: &str, body: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("ktt-test-{}-{name}", std::process::id()));
        std::fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn csv_with_quotes_extra_columns_and_dupes() {
        let p = write_tmp(
            "cat.csv",
            "\u{feff}Song ID,Artist,Title\nDK1,Kings of Leon,Sex on Fire\nDK2,\"Earth, Wind & Fire\",September\nZ9,kings of leon,sex on fire\n",
        );
        let songs = load(&p).unwrap();
        assert_eq!(
            songs,
            vec![
                Song { artist: "Earth, Wind & Fire".into(), title: "September".into() },
                Song { artist: "Kings of Leon".into(), title: "Sex on Fire".into() },
            ]
        );
    }

    #[test]
    fn artist_case_variants_merge_under_most_common_spelling() {
        let songs = normalize(vec![
            Song { artist: "Kings Of Leon".into(), title: "Closer".into() },
            Song { artist: "Kings of Leon".into(), title: "Walls".into() },
            Song { artist: "Kings of Leon".into(), title: "Use Somebody".into() },
        ]);
        assert!(songs.iter().all(|s| s.artist == "Kings of Leon"), "{songs:?}");
        assert_eq!(songs.len(), 3);
    }

    #[test]
    fn tab_separated_txt() {
        let p = write_tmp("cat.txt", "title\tartist\nWonderwall\tOasis\n");
        assert_eq!(load(&p).unwrap(), vec![Song { artist: "Oasis".into(), title: "Wonderwall".into() }]);
    }

    #[test]
    fn json_array() {
        let p = write_tmp("cat.json", r#"[{"artist":"Oasis","title":"Wonderwall"}]"#);
        assert_eq!(load(&p).unwrap().len(), 1);
    }

    #[test]
    fn missing_columns_is_an_error() {
        let p = write_tmp("bad.csv", "name,song\na,b\n");
        assert!(load(&p).is_err());
    }

    /// Against a real kroak-time catalog: `KTT_REDB=path cargo test -- --ignored redb`.
    #[test]
    #[ignore]
    fn reads_real_redb() {
        let path = std::env::var("KTT_REDB").expect("set KTT_REDB to a catalog.redb");
        let songs = load(Path::new(&path)).unwrap();
        assert!(!songs.is_empty());
        eprintln!("{} songs; first: {:?}; last: {:?}", songs.len(), songs.first(), songs.last());
    }

    #[test]
    fn served_shape_dedupes_artists() {
        let s = serve(&[
            Song { artist: "A".into(), title: "x".into() },
            Song { artist: "A".into(), title: "y".into() },
            Song { artist: "B".into(), title: "z".into() },
        ]);
        let v: serde_json::Value = serde_json::from_slice(&s.json).unwrap();
        assert_eq!(v["artists"], serde_json::json!(["A", "B"]));
        assert_eq!(v["songs"], serde_json::json!([[0, "x"], [0, "y"], [1, "z"]]));
    }
}
