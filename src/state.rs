use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Singer {
    pub name: String,
    pub next_song_artist: Option<String>,
    pub next_song_title: Option<String>,
    pub is_current: bool,
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
