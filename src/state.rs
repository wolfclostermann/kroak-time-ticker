use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Singer {
    pub name: String,
    pub next_song_artist: Option<String>,
    pub next_song_title: Option<String>,
    pub is_current: bool,
    /// Estimated seconds from now until this singer next starts singing (0 =
    /// on stage now). `null`/absent when kroak-time has rotation timing off,
    /// the singer was skipped as empty, or an older kroak-time doesn't send
    /// the field yet. `#[serde(default)]` keeps this ticker working against
    /// that older kroak-time.
    #[serde(default)]
    pub sings_in_secs: Option<i64>,
    /// The same estimate as a Unix timestamp (seconds), for clock-time display.
    #[serde(default)]
    pub sings_at: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct KaraokeState {
    pub current_singer: Option<Singer>,
    pub next_up: Option<Singer>,
    pub rotation: Vec<Singer>,
    pub singer_count: usize,
    pub is_playing: bool,
    pub status: String,
    /// Raw estimated seconds for the full rotation to complete, as reported by
    /// kroak-time. `#[serde(default)]` so this ticker keeps working against an
    /// older kroak-time that doesn't send the field yet.
    #[serde(default)]
    pub queue_duration_secs: i64,
}

pub async fn fetch_state(upstream_url: &str) -> Result<KaraokeState> {
    let state = reqwest::get(upstream_url)
        .await?
        .json::<KaraokeState>()
        .await?;
    Ok(state)
}
