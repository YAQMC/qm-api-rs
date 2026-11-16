//! Cross-device playback snapshot models.

use serde::{Deserialize, Serialize};

/// Current-playlist snapshot returned by QQ Music's cross-platform home card.
///
/// This is a snapshot of the active remote player, not play history. The
/// observed response does not contain a seek position or playback mode.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct OtherPlatformCurrentPlaylist {
    /// Response platform/type marker from the upstream `ct` field.
    #[serde(rename = "ct")]
    pub ct: i32,
    /// Response client/version marker from the upstream `cv` field.
    #[serde(rename = "cv")]
    pub cv: i32,
    /// Server snapshot update time, in Unix seconds.
    #[serde(rename = "updateTimestampSec")]
    pub update_timestamp_sec: i64,
    /// Song IDs surrounding the active song, in upstream playlist order.
    /// Duplicates are preserved.
    #[serde(rename = "playList")]
    pub play_list: Vec<i64>,
    /// Active song ID (`0` means that the response did not provide one).
    #[serde(rename = "listeningSongID")]
    pub listening_song_id: i64,
    /// Opaque server-provided display string. When it contains the observed
    /// nested JSON shape, use [`Self::display_text`] to decode it.
    pub text: Option<String>,
    /// Card lifetime, in seconds.
    #[serde(rename = "keepSec")]
    pub keep_sec: i32,
}

impl OtherPlatformCurrentPlaylist {
    /// Decode the observed optional `text` payload without making an otherwise
    /// usable snapshot fail when the server changes its presentation shape.
    pub fn display_text(&self) -> Option<OtherPlatformPlaylistText> {
        serde_json::from_str(self.text.as_deref()?).ok()
    }

    /// Whether the snapshot identifies an active song.
    pub fn has_current_song(&self) -> bool {
        self.listening_song_id > 0
    }
}

/// Nested display fields observed inside `OtherPlatformCurrentPlaylist.text`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct OtherPlatformPlaylistText {
    #[serde(rename = "txt")]
    pub text: String,
    #[serde(rename = "songName")]
    pub song_name: String,
}

/// One explicit update to the current account's listening state.
///
/// The service field names reflect the observed mobile contract. In particular,
/// `song_play_time_ms` is the track duration and `remaining_time_ms` is duration
/// minus current position; neither is the current position itself.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ListeningMusicReportRequest {
    #[serde(rename = "songmid")]
    pub song_mid: String,
    #[serde(rename = "songtype")]
    pub song_type: i32,
    #[serde(rename = "pauseFlag")]
    pub pause_flag: bool,
    #[serde(rename = "remainingTime")]
    pub remaining_time_ms: u64,
    #[serde(rename = "songPlayTime")]
    pub song_play_time_ms: u64,
    #[serde(rename = "songid")]
    pub song_id: i64,
    pub speed: f32,
    #[serde(rename = "icon2")]
    pub icon2: i64,
    #[serde(rename = "playList")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub play_list: Option<Vec<i64>>,
}

/// Business result returned by `ListeningMusicReport`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct ListeningMusicReportResponse {
    #[serde(rename = "code")]
    pub code: i32,
    #[serde(rename = "errmsg")]
    pub err_msg: String,
    #[serde(rename = "needPlayList")]
    pub need_play_list: i32,
}
