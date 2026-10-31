//! Song requests submitted from the singer page, waiting for kroak-time.
//!
//! This module is the request store. kroak-time pulls from it through the
//! contract-driven request server in `contract.rs`, the same way it pulls
//! from wcis-helper.
//!
//! Requests are persisted to `song-requests.json` in the data folder
//! so a ticker restart mid-show doesn't lose ones kroak-time hasn't taken yet.

use serde::{Deserialize, Serialize};
use crate::state::{KaraokeState, Singer};
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

/// Lowercase letters and digits only, so "Rein me in" / "Rein Me In!" and
/// "Eva & Summer" / "eva &summer" compare equal.
fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// How long a request stays on its singer's "Your requests" list and in the
/// host's TUI list — about a night.
pub const HISTORY_SECS: i64 = 12 * 3600;

/// Upper bound on remembered requests, whatever the windows.
const MAX_SENT: usize = 1000;

/// A note from the host to whoever sent a request, shown on their phone.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HostMessage {
    pub text: String,
    /// Unix seconds.
    pub time: i64,
}

/// A request that was accepted, kept after kroak-time has taken it (which
/// happens within seconds) so its singer can follow it, the host can reply
/// to it, and a resend of the same song can be recognised.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Sent {
    #[serde(default)]
    request_id: i64,
    device_id: String,
    singer_key: String,
    song_key: String,
    singer: String,
    #[serde(default)]
    artist: String,
    title: String,
    time: i64,
    /// kroak-time has taken it off the ticker.
    #[serde(default)]
    taken: bool,
    /// Seen on stage: kroak-time's current singer, playing this song.
    #[serde(default)]
    on_stage: bool,
    /// Was on stage and has since finished.
    #[serde(default)]
    sung: bool,
    #[serde(default)]
    message: Option<HostMessage>,
}

impl Sent {
    /// Whether `s` (a singer in kroak-time's state) is this request: same
    /// singer (one name may contain the other, as hosts often shorten
    /// "Eva And Summer" to "Eva") with this as their next song.
    fn is(&self, s: &Singer) -> bool {
        let name = norm(&s.name);
        let me = &self.singer_key;
        let name_ok = !name.is_empty() && !me.is_empty() && (name.contains(me.as_str()) || me.contains(name.as_str()));
        let title = norm(&self.title);
        name_ok && !title.is_empty() && s.next_song_title.as_deref().map(norm).is_some_and(|t| t == title)
    }

    fn stage(&self, state: Option<&KaraokeState>) -> (Stage, Option<i64>) {
        if self.sung {
            return (Stage::Sung, None);
        }
        if let Some(st) = state {
            if let Some(cur) = st.current_singer.as_ref().filter(|s| self.is(s)) {
                return if st.is_playing { (Stage::OnNow, None) } else { (Stage::UpNext, cur.sings_in_secs) };
            }
            if let Some(next) = st.next_up.as_ref().filter(|s| self.is(s)) {
                return (Stage::UpNext, next.sings_in_secs);
            }
            if let Some(s) = st.rotation.iter().find(|s| self.is(s)) {
                return (Stage::Queued, s.sings_in_secs);
            }
        }
        if self.taken {
            (Stage::WithHost, None)
        } else {
            (Stage::Sent, None)
        }
    }

    fn tracked(&self, state: Option<&KaraokeState>) -> Tracked {
        let (stage, sings_in_secs) = self.stage(state);
        Tracked {
            request_id: self.request_id,
            singer: self.singer.clone(),
            artist: self.artist.clone(),
            title: self.title.clone(),
            sent_at: self.time,
            stage,
            sings_in_secs,
            message: self.message.clone(),
            device_code: device_code(&self.device_id),
        }
    }
}

/// Where a request has got to, as its singer sees it.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// Waiting on the ticker for kroak-time to pick it up.
    Sent,
    /// kroak-time has it, but it isn't in the rotation as the singer's next
    /// song (not yet — or the host passed on it).
    WithHost,
    /// In the rotation as this singer's next song.
    Queued,
    UpNext,
    OnNow,
    Sung,
}

impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Stage::Sent => "sent",
            Stage::WithHost => "with host",
            Stage::Queued => "queued",
            Stage::UpNext => "up next",
            Stage::OnNow => "on now",
            Stage::Sung => "sung",
        }
    }
}

/// One of tonight's requests, as shown on its singer's phone and in the TUI.
#[derive(Debug, Clone, Serialize)]
pub struct Tracked {
    pub request_id: i64,
    pub singer: String,
    pub artist: String,
    pub title: String,
    /// Unix seconds.
    pub sent_at: i64,
    pub stage: Stage,
    /// kroak-time's estimate, while queued / up next.
    pub sings_in_secs: Option<i64>,
    pub message: Option<HostMessage>,
    #[serde(skip)]
    pub device_code: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Persisted {
    next_id: i64,
    serial: i32,
    accepting: bool,
    requests: Vec<SongRequest>,
    #[serde(default)]
    sent: Vec<Sent>,
}

impl Default for Persisted {
    fn default() -> Self {
        Self { next_id: 1, serial: 1, accepting: true, requests: Vec::new(), sent: Vec::new() }
    }
}

#[derive(Clone)]
pub struct RequestStore {
    inner: Arc<RwLock<Persisted>>,
    /// kroak-time's latest state, for working out each request's stage.
    live: Arc<RwLock<Option<KaraokeState>>>,
    /// Tests run in memory and never touch STORE_FILE.
    persistent: bool,
}

#[derive(Debug, PartialEq)]
pub enum SubmitError {
    NotAccepting,
    TooManyPending(usize),
    /// Same song already sent from this phone or under this singer name.
    Duplicate { singer: String, title: String, secs_ago: i64 },
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
        Self { inner: Arc::new(RwLock::new(persisted)), live: Arc::default(), persistent: true }
    }

    #[cfg(test)]
    pub(crate) fn in_memory() -> Self {
        Self { inner: Arc::new(RwLock::new(Persisted::default())), live: Arc::default(), persistent: false }
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

    /// kroak-time taking a request off the ticker.
    pub fn delete(&self, request_id: i64) {
        let mut s = self.inner.write().unwrap();
        if let Some(pos) = s.requests.iter().position(|r| r.request_id == request_id) {
            s.requests.remove(pos);
            if let Some(x) = s.sent.iter_mut().find(|x| x.request_id == request_id) {
                x.taken = true;
            }
            bump_serial(&mut s);
            self.persist(&s);
        }
    }

    /// Feeds in kroak-time's latest state: notes which requests have been on
    /// stage and which have since been sung.
    pub fn observe(&self, state: KaraokeState) {
        let mut s = self.inner.write().unwrap();
        let mut changed = false;
        for x in s.sent.iter_mut().filter(|x| !x.sung) {
            let current = state.current_singer.as_ref().is_some_and(|c| x.is(c));
            if current && state.is_playing && !x.on_stage {
                x.on_stage = true;
                x.taken = true;
                changed = true;
            } else if x.on_stage && !current {
                x.sung = true;
                changed = true;
            }
        }
        if changed {
            self.persist(&s);
        }
        *self.live.write().unwrap() = Some(state);
    }

    /// The requests sent from one phone in the last `HISTORY_SECS`, newest first.
    pub fn for_device(&self, device_id: &str) -> Vec<Tracked> {
        self.tracked(unix_now(), |x| x.device_id == device_id)
    }

    /// All requests from the last `HISTORY_SECS`, newest first.
    pub fn recent(&self) -> Vec<Tracked> {
        self.tracked(unix_now(), |_| true)
    }

    fn tracked(&self, now: i64, keep: impl Fn(&Sent) -> bool) -> Vec<Tracked> {
        let s = self.inner.read().unwrap();
        let live = self.live.read().unwrap();
        s.sent
            .iter()
            .rev()
            .filter(|x| now - x.time < HISTORY_SECS && keep(x))
            .map(|x| x.tracked(live.as_ref()))
            .collect()
    }

    /// Sets (or, with empty `text`, clears) the host's note on a request.
    /// False if the request is no longer remembered.
    pub fn set_message(&self, request_id: i64, text: &str) -> bool {
        let mut s = self.inner.write().unwrap();
        let Some(x) = s.sent.iter_mut().find(|x| x.request_id == request_id) else {
            return false;
        };
        let text: String = text.trim().chars().take(MAX_COMMENT_LEN).collect();
        x.message = (!text.is_empty()).then(|| HostMessage { text, time: unix_now() });
        self.persist(&s);
        true
    }

    /// Adds a request; `max_pending_per_device` = 0 means unlimited.
    /// `duplicate_window_secs` = 0 turns off the repeat-song check.
    pub fn submit(
        &self,
        req: NewRequest,
        max_pending_per_device: usize,
        duplicate_window_secs: i64,
    ) -> Result<SongRequest, SubmitError> {
        self.submit_at(req, max_pending_per_device, duplicate_window_secs, unix_now())
    }

    fn submit_at(
        &self,
        req: NewRequest,
        max_pending_per_device: usize,
        duplicate_window_secs: i64,
        now: i64,
    ) -> Result<SongRequest, SubmitError> {
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

        let singer_key = norm(&req.singer);
        let song_key = format!("{}|{}", norm(&req.artist), norm(&req.title));
        s.sent.retain(|x| now - x.time < HISTORY_SECS.max(duplicate_window_secs));
        // Once sung, asking for it again is a real new request.
        if let Some(prev) = s.sent.iter().find(|x| {
            now - x.time < duplicate_window_secs
                && !x.sung
                && x.song_key == song_key
                && (x.device_id == req.device_id || x.singer_key == singer_key)
        }) {
            return Err(SubmitError::Duplicate {
                singer: prev.singer.clone(),
                title: prev.title.clone(),
                secs_ago: now - prev.time,
            });
        }

        let r = SongRequest {
            request_id: s.next_id,
            singer: req.singer,
            artist: req.artist,
            title: req.title,
            key_change: req.key_change,
            request_time: now,
            device_id: req.device_id,
            fingerprint: req.fingerprint,
            in_catalog: req.in_catalog,
            comment: req.comment,
        };
        s.next_id += 1;
        if s.sent.len() >= MAX_SENT {
            s.sent.remove(0);
        }
        s.sent.push(Sent {
            request_id: r.request_id,
            device_id: r.device_id.clone(),
            singer_key,
            song_key,
            singer: r.singer.clone(),
            artist: r.artist.clone(),
            title: r.title.clone(),
            time: now,
            taken: false,
            on_stage: false,
            sung: false,
            message: None,
        });
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
        store.submit(new_req("dev-a"), 2, 0).unwrap();
        store.submit(new_req("dev-a"), 2, 0).unwrap();
        assert_eq!(store.submit(new_req("dev-a"), 2, 0).unwrap_err(), SubmitError::TooManyPending(2));
        assert!(store.submit(new_req("dev-b"), 2, 0).is_ok());

        store.set_accepting(false);
        assert_eq!(store.submit(new_req("dev-c"), 0, 0).unwrap_err(), SubmitError::NotAccepting);
    }

    #[test]
    fn delete_bumps_serial() {
        let store = RequestStore::in_memory();
        let r = store.submit(new_req("dev-a"), 0, 0).unwrap();
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

    const HOUR: i64 = 3600;

    #[test]
    fn resend_after_kroak_time_took_it_is_refused() {
        let store = RequestStore::in_memory();
        let r = store.submit_at(new_req("dev-a"), 0, 6 * HOUR, 1000).unwrap();
        store.delete(r.request_id); // kroak-time took it
        let mut again = new_req("dev-a");
        again.title = "sex on fire!".into(); // retyped, different case/punctuation
        assert_eq!(
            store.submit_at(again, 0, 6 * HOUR, 1040).unwrap_err(),
            SubmitError::Duplicate { singer: "Alice".into(), title: "Sex on Fire".into(), secs_ago: 40 }
        );
    }

    #[test]
    fn same_name_on_another_phone_is_refused_but_other_people_are_not() {
        let store = RequestStore::in_memory();
        store.submit_at(new_req("dev-a"), 0, 6 * HOUR, 1000).unwrap();
        let mut friend = new_req("dev-b");
        friend.singer = "alice ".into();
        assert!(matches!(store.submit_at(friend, 0, 6 * HOUR, 1010), Err(SubmitError::Duplicate { .. })));
        let mut bob = new_req("dev-c");
        bob.singer = "Bob".into();
        assert!(store.submit_at(bob, 0, 6 * HOUR, 1020).is_ok());
    }

    #[test]
    fn different_song_and_expired_window_are_allowed() {
        let store = RequestStore::in_memory();
        store.submit_at(new_req("dev-a"), 0, 6 * HOUR, 1000).unwrap();
        let mut other = new_req("dev-a");
        other.title = "Use Somebody".into();
        assert!(store.submit_at(other, 0, 6 * HOUR, 1010).is_ok());
        assert!(store.submit_at(new_req("dev-a"), 0, 6 * HOUR, 1000 + 6 * HOUR).is_ok());
    }

    #[test]
    fn window_zero_allows_repeats() {
        let store = RequestStore::in_memory();
        store.submit_at(new_req("dev-a"), 0, 0, 1000).unwrap();
        assert!(store.submit_at(new_req("dev-a"), 0, 0, 1001).is_ok());
    }

    #[test]
    fn old_store_file_without_sent_still_loads() {
        let p: Persisted = serde_json::from_str(r#"{"next_id":81,"serial":161,"accepting":true,"requests":[]}"#).unwrap();
        assert!(p.sent.is_empty());
    }

    fn singer(name: &str, title: &str) -> Singer {
        Singer {
            name: name.into(),
            next_song_artist: Some("Kings of Leon".into()),
            next_song_title: Some(title.into()),
            next_song_duration_ms: None,
            is_current: false,
            sings_in_secs: Some(600),
            sings_at: None,
        }
    }

    fn state(current: Option<Singer>, next: Option<Singer>, rotation: Vec<Singer>, playing: bool) -> KaraokeState {
        KaraokeState {
            current_singer: current,
            next_up: next,
            rotation,
            singer_count: 0,
            is_playing: playing,
            status: String::new(),
            queue_duration_secs: 0,
            song_position_ms: None,
            song_duration_ms: None,
            playback_rate: None,
        }
    }

    fn stage_of(store: &RequestStore, device: &str) -> Stage {
        store.tracked(1000, |x| x.device_id == device)[0].stage
    }

    #[test]
    fn a_request_follows_kroak_time_from_sent_to_sung() {
        let store = RequestStore::in_memory();
        let r = store.submit_at(new_req("dev-a"), 0, 6 * HOUR, 1000).unwrap();
        assert_eq!(stage_of(&store, "dev-a"), Stage::Sent);

        store.delete(r.request_id);
        assert_eq!(stage_of(&store, "dev-a"), Stage::WithHost);

        // The host shortened the name and the title's case differs.
        let me = || singer("ALICE", "sex on fire");
        store.observe(state(None, None, vec![me()], false));
        assert_eq!(stage_of(&store, "dev-a"), Stage::Queued);
        assert_eq!(store.tracked(1000, |_| true)[0].sings_in_secs, Some(600));

        store.observe(state(None, Some(me()), vec![me()], false));
        assert_eq!(stage_of(&store, "dev-a"), Stage::UpNext);

        store.observe(state(Some(me()), None, vec![me()], true));
        assert_eq!(stage_of(&store, "dev-a"), Stage::OnNow);

        // Paused mid-song: still on stage, not sung.
        store.observe(state(Some(me()), None, vec![me()], false));
        assert_ne!(stage_of(&store, "dev-a"), Stage::Sung);

        store.observe(state(Some(singer("Bob", "Mr Brightside")), None, vec![], true));
        assert_eq!(stage_of(&store, "dev-a"), Stage::Sung);

        // Sung, so asking for it again is a new request, not a duplicate.
        assert!(store.submit_at(new_req("dev-a"), 0, 6 * HOUR, 1100).is_ok());
    }

    #[test]
    fn someone_else_with_the_song_is_not_a_match() {
        let store = RequestStore::in_memory();
        store.submit_at(new_req("dev-a"), 0, 6 * HOUR, 1000).unwrap();
        store.observe(state(Some(singer("Bob", "Sex on Fire")), None, vec![], true));
        assert_eq!(stage_of(&store, "dev-a"), Stage::Sent);
    }

    #[test]
    fn host_messages_reach_only_that_phone() {
        let store = RequestStore::in_memory();
        let r = store.submit_at(new_req("dev-a"), 0, 6 * HOUR, 1000).unwrap();
        store.submit_at(new_req("dev-b"), 0, 0, 1000).unwrap();
        assert!(store.set_message(r.request_id, "  Not in the catalog  "));
        assert_eq!(store.tracked(1000, |x| x.device_id == "dev-a")[0].message.as_ref().unwrap().text, "Not in the catalog");
        assert!(store.tracked(1000, |x| x.device_id == "dev-b")[0].message.is_none());
        assert!(store.set_message(r.request_id, ""));
        assert!(store.tracked(1000, |x| x.device_id == "dev-a")[0].message.is_none());
        assert!(!store.set_message(9999, "hello"));
    }
}
