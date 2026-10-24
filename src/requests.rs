//! Song requests submitted from the singer page, waiting for kroak-time.
//!
//! This module is the request store. kroak-time pulls from it through the
//! contract-driven request server in `contract.rs`, the same way it pulls
//! from wcis-helper.
//!
//! Requests are persisted to `song-requests.json` in the data folder
//! so a ticker restart mid-show doesn't lose ones kroak-time hasn't taken yet.

use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};

pub const STORE_FILE: &str = "song-requests.json";

/// Longest artist/title/name accepted from the public form.
pub const MAX_FIELD_LEN: usize = 120;
/// Longest free-text comment accepted from the public form.
pub const MAX_COMMENT_LEN: usize = 200;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SongRequest {
    pub request_id: i64,
    pub singer: String,
    pub artist: String,
    pub title: String,
    /// Semitones, -6..=6; 0 = original key.
    pub key_change: i32,
    /// Unix seconds.
    pub request_time: i64,
    /// Canonical per-device id from the singer page (persisted in the
    /// phone's localStorage). Identical across requests from one phone even
    /// when they're under different singer names.
    pub device_id: String,
    /// Browser fingerprint hash, recorded alongside as a secondary signal.
    #[serde(default)]
    pub fingerprint: String,
    /// Whether the song was picked from the catalog (vs. typed freehand).
    #[serde(default)]
    pub in_catalog: bool,
    /// Optional free-text note from the requester; empty if none.
    #[serde(default)]
    pub comment: String,
}

impl SongRequest {
    /// Short, human-checkable device code for the host, e.g. "K3F9".
    /// Derived from the device id so it's stable across requests.
    pub fn device_code(&self) -> String {
        device_code(&self.device_id)
    }
}

pub fn device_code(device_id: &str) -> String {
    // FNV-1a, then 4 chars of an unambiguous base-32 alphabet (no 0/O/1/I).
    const ALPHABET: &[u8] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";
    let mut h: u64 = 0xcbf29ce484222325;
    for b in device_id.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    (0..4).map(|i| ALPHABET[((h >> (i * 5)) & 31) as usize] as char).collect()
}

#[derive(Debug, Serialize, Deserialize)]
struct Persisted {
    next_id: i64,
    serial: i32,
    accepting: bool,
    requests: Vec<SongRequest>,
}

impl Default for Persisted {
    fn default() -> Self {
        Self { next_id: 1, serial: 1, accepting: true, requests: Vec::new() }
    }
}

#[derive(Clone)]
pub struct RequestStore {
    inner: Arc<RwLock<Persisted>>,
    /// Tests run in memory and never touch STORE_FILE.
    persistent: bool,
}

#[derive(Debug, PartialEq)]
pub enum SubmitError {
    NotAccepting,
    TooManyPending(usize),
}

pub struct NewRequest {
    pub singer: String,
    pub artist: String,
    pub title: String,
    pub key_change: i32,
    pub device_id: String,
    pub fingerprint: String,
    pub in_catalog: bool,
    pub comment: String,
}

impl RequestStore {
    pub fn load() -> Self {
        let persisted = std::fs::read_to_string(crate::paths::file(STORE_FILE))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        Self { inner: Arc::new(RwLock::new(persisted)), persistent: true }
    }

    #[cfg(test)]
    pub(crate) fn in_memory() -> Self {
        Self { inner: Arc::new(RwLock::new(Persisted::default())), persistent: false }
    }

    #[cfg(test)]
    pub fn accepting(&self) -> bool {
        self.inner.read().unwrap().accepting
    }

    pub fn set_accepting(&self, accepting: bool) {
        let mut s = self.inner.write().unwrap();
        if s.accepting != accepting {
            s.accepting = accepting;
            self.persist(&s);
        }
    }

    pub fn serial(&self) -> i32 {
        self.inner.read().unwrap().serial
    }

    pub fn list(&self) -> Vec<SongRequest> {
        self.inner.read().unwrap().requests.clone()
    }

    pub fn delete(&self, request_id: i64) {
        let mut s = self.inner.write().unwrap();
        if let Some(pos) = s.requests.iter().position(|r| r.request_id == request_id) {
            s.requests.remove(pos);
            bump_serial(&mut s);
            self.persist(&s);
        }
    }

    /// Adds a request; `max_pending_per_device` = 0 means unlimited.
    pub fn submit(&self, req: NewRequest, max_pending_per_device: usize) -> Result<SongRequest, SubmitError> {
        let mut s = self.inner.write().unwrap();
        if !s.accepting {
            return Err(SubmitError::NotAccepting);
        }
        if max_pending_per_device > 0 {
            let pending = s.requests.iter().filter(|r| r.device_id == req.device_id).count();
            if pending >= max_pending_per_device {
                return Err(SubmitError::TooManyPending(pending));
            }
        }
        let r = SongRequest {
            request_id: s.next_id,
            singer: req.singer,
            artist: req.artist,
            title: req.title,
            key_change: req.key_change,
            request_time: unix_now(),
            device_id: req.device_id,
            fingerprint: req.fingerprint,
            in_catalog: req.in_catalog,
            comment: req.comment,
        };
        s.next_id += 1;
        s.requests.push(r.clone());
        bump_serial(&mut s);
        self.persist(&s);
        Ok(r)
    }

    fn persist(&self, s: &Persisted) {
        if !self.persistent {
            return;
        }
        match serde_json::to_vec_pretty(s) {
            Ok(json) => {
                if let Err(e) = std::fs::write(crate::paths::file(STORE_FILE), json) {
                    tracing::warn!("Couldn't write {STORE_FILE}: {e}");
                }
            }
            Err(e) => tracing::warn!("Couldn't serialize song requests: {e}"),
        }
    }
}

fn bump_serial(s: &mut Persisted) {
    s.serial = if s.serial >= i32::MAX { 2 } else { s.serial + 1 };
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_req(device: &str) -> NewRequest {
        NewRequest {
            singer: "Alice".into(),
            artist: "Kings of Leon".into(),
            title: "Sex on Fire".into(),
            key_change: -2,
            device_id: device.into(),
            fingerprint: "fp".into(),
            in_catalog: true,
            comment: String::new(),
        }
    }

    #[test]
    fn per_device_limit_and_accepting() {
        let store = RequestStore::in_memory();
        store.submit(new_req("dev-a"), 2).unwrap();
        store.submit(new_req("dev-a"), 2).unwrap();
        assert_eq!(store.submit(new_req("dev-a"), 2).unwrap_err(), SubmitError::TooManyPending(2));
        assert!(store.submit(new_req("dev-b"), 2).is_ok());

        store.set_accepting(false);
        assert_eq!(store.submit(new_req("dev-c"), 0).unwrap_err(), SubmitError::NotAccepting);
    }

    #[test]
    fn delete_bumps_serial() {
        let store = RequestStore::in_memory();
        let r = store.submit(new_req("dev-a"), 0).unwrap();
        let before = store.serial();
        store.delete(r.request_id);
        assert_ne!(store.serial(), before);
        assert!(store.list().is_empty());
    }

    #[test]
    fn device_code_is_stable_and_short() {
        assert_eq!(device_code("abc"), device_code("abc"));
        assert_ne!(device_code("abc"), device_code("abd"));
        assert_eq!(device_code("abc").len(), 4);
    }
}
