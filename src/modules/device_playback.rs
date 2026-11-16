//! Cross-device playback snapshots and explicit listening-state reports.

use serde_json::json;
use tokio_util::sync::CancellationToken;

use super::ApiModule;
use crate::context::RequestOptions;
use crate::error::{QmError, Result};
use crate::models::device_playback::{
    ListeningMusicReportRequest, ListeningMusicReportResponse, OtherPlatformCurrentPlaylist,
};
use crate::models::Credential;

/// Maximum decoded response size for the current-playlist snapshot.
pub const MAX_OTHER_PLATFORM_PLAYLIST_RESPONSE_BYTES: usize = 256 * 1024;
/// Maximum decoded response size for a listening-state report acknowledgment.
pub const MAX_LISTENING_REPORT_RESPONSE_BYTES: usize = 64 * 1024;
/// The observed mobile report caps the submitted play-list slice at 100 IDs.
pub const MAX_LISTENING_REPORT_PLAYLIST_ITEMS: usize = 100;

const MODULE: &str = "music.richFlag.listening";
const METHOD: &str = "GetOtherPlatCurrentPlaylist";
const REPORT_METHOD: &str = "ListeningMusicReport";

/// API for reading another platform's playback snapshot and explicitly reporting
/// this client's listening state.
///
/// Snapshot reads do not publish state. Listening reports are remote state writes;
/// neither operation is a dedicated takeover or remote playback control command.
#[derive(Clone, Debug)]
pub struct DevicePlaybackApi {
    pub(crate) base: ApiModule,
}

impl DevicePlaybackApi {
    /// Read the latest other-platform playback snapshot using the client's
    /// current credential.
    pub async fn get_other_platform_current_playlist(
        &self,
    ) -> Result<OtherPlatformCurrentPlaylist> {
        self.get_other_platform_current_playlist_with_cancellation(None, CancellationToken::new())
            .await
    }

    /// Read the snapshot for an explicit account credential.
    pub async fn get_other_platform_current_playlist_for(
        &self,
        credential: &Credential,
    ) -> Result<OtherPlatformCurrentPlaylist> {
        self.get_other_platform_current_playlist_with_cancellation(
            Some(credential),
            CancellationToken::new(),
        )
        .await
    }

    /// Read with explicit credential selection and cancellation.
    pub async fn get_other_platform_current_playlist_with_cancellation(
        &self,
        credential: Option<&Credential>,
        cancellation: CancellationToken,
    ) -> Result<OtherPlatformCurrentPlaylist> {
        let mut options = RequestOptions::default();
        options.credential = credential.cloned();
        options.require_login = true;
        options.sign = true;
        options.cancellation = cancellation.clone();

        let data = self
            .base
            .context
            .request_cgi_with_response_limit(
                MODULE,
                METHOD,
                json!({}),
                &options,
                Some(MAX_OTHER_PLATFORM_PLAYLIST_RESPONSE_BYTES),
            )
            .await?
            .require_success()?;

        let parse = tokio::task::spawn_blocking(move || {
            serde_json::from_value(data).map_err(|_| QmError::Protocol {
                stage: "other-platform-playlist",
                message: "invalid current-playlist response shape".into(),
            })
        });
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(QmError::cancelled()),
            result = parse => result.map_err(|_| QmError::Protocol {
                stage: "other-platform-playlist",
                message: "response parser task failed".into(),
            })?,
        }
    }

    /// Explicitly publish this client's active track state for the current
    /// account. This is a remote state write, not a dedicated takeover command.
    ///
    /// The caller must invoke it only for a deliberate player state transition.
    /// It uses `RetryClass::Write`, so the transport does not automatically
    /// replay it after an ambiguous network failure.
    pub async fn report_listening_music(
        &self,
        request: &ListeningMusicReportRequest,
        credential: &Credential,
        cancellation: CancellationToken,
    ) -> Result<ListeningMusicReportResponse> {
        if request.song_mid.trim().is_empty()
            || request.song_mid.len() > 128
            || request.song_id < 0
            || !request.speed.is_finite()
            || request.speed <= 0.0
            || request
                .play_list
                .as_ref()
                .is_some_and(|songs| songs.len() > MAX_LISTENING_REPORT_PLAYLIST_ITEMS)
            || (request.song_play_time_ms > 0
                && request.remaining_time_ms > request.song_play_time_ms)
        {
            return Err(QmError::ValueError(
                "invalid listening report fields".into(),
            ));
        }
        if cancellation.is_cancelled() {
            return Err(QmError::cancelled());
        }

        let mut options = RequestOptions::default();
        options.credential = Some(credential.clone());
        options.require_login = true;
        options.preserve_bool = true;
        options.sign = true;
        options.retry = crate::RetryClass::Write;
        options.cancellation = cancellation.clone();

        let param = serde_json::to_value(request)?;
        let response = self
            .base
            .context
            .request_cgi_with_response_limit(
                MODULE,
                REPORT_METHOD,
                param,
                &options,
                Some(MAX_LISTENING_REPORT_RESPONSE_BYTES),
            )
            .await?
            .require_success()?;
        let parse = tokio::task::spawn_blocking(move || {
            serde_json::from_value(response).map_err(|_| QmError::Protocol {
                stage: "listening-report",
                message: "invalid listening report response shape".into(),
            })
        });
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(QmError::cancelled()),
            result = parse => result.map_err(|_| QmError::Protocol {
                stage: "listening-report",
                message: "response parser task failed".into(),
            })?,
        }
    }

    pub(crate) fn new(context: std::sync::Arc<crate::context::ApiContext>) -> Self {
        Self {
            base: ApiModule::new(context),
        }
    }
}

#[cfg(test)]
#[path = "device_playback_tests.rs"]
mod tests;
