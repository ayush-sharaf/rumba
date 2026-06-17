//! Shared data types mirroring the JSON the Python sidecar emits.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Track {
    pub video_id: Option<String>,
    pub title: String,
    #[serde(default)]
    pub artist: String,
    // album is carried through for a future album column.
    #[serde(default)]
    #[allow(dead_code)]
    pub album: String,
    #[serde(default)]
    pub duration: u64,
    #[serde(default)]
    #[allow(dead_code)]
    pub thumbnail: Option<String>,
}

impl Track {
    /// `m:ss` formatting for the duration, e.g. `3:07`. Blank when unknown
    /// (some feeds — e.g. home recommendations — don't include a duration).
    pub fn duration_str(&self) -> String {
        if self.duration == 0 {
            String::new()
        } else {
            fmt_secs(self.duration)
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlaylistMeta {
    pub playlist_id: Option<String>,
    pub title: String,
    #[serde(default)]
    pub count: Option<serde_json::Value>,
}

impl PlaylistMeta {
    pub fn count_str(&self) -> String {
        match &self.count {
            Some(v) => v.to_string().trim_matches('"').to_string(),
            None => String::new(),
        }
    }
}

/// Render seconds as `m:ss` (or `h:mm:ss` past an hour).
pub fn fmt_secs(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}
