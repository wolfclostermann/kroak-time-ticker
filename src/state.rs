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
}

pub async fn fetch_state(upstream_url: &str) -> Result<KaraokeState> {
    let state = reqwest::get(upstream_url)
        .await?
        .json::<KaraokeState>()
        .await?;
    Ok(state)
}
