//! QQ playback filename and CDN response contracts. Player quality selection,
//! account policy, caching and decoder ownership remain with the consumer.
use crate::models::song::UrlinfoItem;
use crate::modules::song::{
    EncryptedSongFileType, FileTypeLike, SongApi, SongFileInfo, SongFileType, SpecialSongFileType,
};
use crate::{Credential, QmError, Result};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackFile {
    Master,
    HiRes,
    EncryptedLossless,
    Lossless,
    High,
    Standard,
    Aac,
    Preview,
}

#[derive(Debug)]
struct HiResFlac;
impl FileTypeLike for HiResFlac {
    fn s(&self) -> &'static str {
        "RS01"
    }
    fn e(&self) -> &'static str {
        ".flac"
    }
}

impl PlaybackFile {
    pub fn file_type(self) -> &'static (dyn FileTypeLike + Send + Sync) {
        match self {
            Self::Master => &EncryptedSongFileType::Master,
            Self::HiRes => &HiResFlac,
            Self::EncryptedLossless => &EncryptedSongFileType::Flac,
            Self::Lossless => &SongFileType::Flac,
            Self::High => &SongFileType::Mp3_320,
            Self::Standard => &SongFileType::Mp3_128,
            Self::Aac => &SongFileType::Acc96,
            Self::Preview => &SpecialSongFileType::Try,
        }
    }
    pub fn filename(self, media_mid: &str) -> String {
        let kind = self.file_type();
        format!("{}{}{}", kind.s(), media_mid, kind.e())
    }
    pub fn from_filename(filename: &str) -> Option<Self> {
        [
            Self::Master,
            Self::HiRes,
            Self::EncryptedLossless,
            Self::Lossless,
            Self::High,
            Self::Standard,
            Self::Aac,
            Self::Preview,
        ]
        .into_iter()
        .find(|kind| {
            filename.starts_with(kind.file_type().s()) && filename.ends_with(kind.file_type().e())
        })
    }
}

/// URLs and keys are deliberately excluded from Debug output.
pub struct PlaybackFileSource {
    pub url: Option<String>,
    pub ekey: Option<String>,
}
impl std::fmt::Debug for PlaybackFileSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlaybackFileSource")
            .field("playable", &self.url.is_some())
            .field("encrypted", &self.ekey.is_some())
            .finish()
    }
}

/// Resolve a homogeneous clear/encrypted batch and validate returned identities.
pub async fn resolve_files(
    api: &SongApi,
    track_mid: &str,
    media_mid: &str,
    filenames: &[String],
    credential: Option<&Credential>,
    encrypted: bool,
) -> Result<HashMap<String, PlaybackFileSource>> {
    if filenames.is_empty() {
        return Ok(HashMap::new());
    }
    let mut seen = HashSet::new();
    let mut infos = Vec::with_capacity(filenames.len());
    for filename in filenames {
        let kind = PlaybackFile::from_filename(filename)
            .ok_or_else(|| QmError::ValueError("unsupported playback file".into()))?;
        let file_type = kind.file_type();
        if filename != &kind.filename(media_mid)
            || file_type.is_encrypted() != encrypted
            || !seen.insert(filename)
        {
            return Err(QmError::ValueError(
                "inconsistent playback file request".into(),
            ));
        }
        infos.push(
            SongFileInfo::new(track_mid)
                .with_media_mid(media_mid)
                .with_song_type(0)
                .with_type_ref(file_type),
        );
    }
    let default_type = if encrypted {
        PlaybackFile::Master
    } else {
        PlaybackFile::Standard
    };
    let response = api
        .get_song_urls(&infos, default_type.file_type(), credential)
        .await?;
    sources_from_items(filenames, response.data, encrypted)
}

/// Decode a vkey response without manufacturing URLs for unavailable rows.
pub fn sources_from_items(
    filenames: &[String],
    items: Vec<UrlinfoItem>,
    encrypted: bool,
) -> Result<HashMap<String, PlaybackFileSource>> {
    let requested: HashSet<_> = filenames.iter().map(String::as_str).collect();
    if requested.len() != filenames.len() {
        return Err(QmError::ApiData("duplicate playback file request".into()));
    }
    let mut seen = HashSet::new();
    let mut result: HashMap<_, _> = filenames
        .iter()
        .map(|name| {
            (
                name.clone(),
                PlaybackFileSource {
                    url: None,
                    ekey: None,
                },
            )
        })
        .collect();
    for item in items {
        if item.filename.is_empty()
            || !requested.contains(item.filename.as_str())
            || !seen.insert(item.filename.clone())
        {
            return Err(QmError::ApiData(
                "invalid playback response identity".into(),
            ));
        }
        if item.result != 0
            || item.purl.trim().is_empty()
            || (encrypted && item.ekey.trim().is_empty())
        {
            continue;
        }
        let url = normalize_cdn_url("https://isure.stream.qqmusic.qq.com/", &item.purl)?;
        result.insert(
            item.filename,
            PlaybackFileSource {
                url: Some(url),
                ekey: encrypted.then_some(item.ekey),
            },
        );
    }
    Ok(result)
}

/// Only default-port HTTPS QQ Music CDNs without userinfo are playback URLs.
pub fn normalize_cdn_url(base: &str, path: &str) -> Result<String> {
    let invalid = || QmError::ApiData("invalid QQ Music playback URL".into());
    let url = url::Url::parse(base)
        .map_err(|_| invalid())?
        .join(path)
        .map_err(|_| invalid())?;
    let host = url.host_str().ok_or_else(invalid)?;
    if url.scheme() != "https"
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || !(host.ends_with(".qqmusic.qq.com") || host.ends_with(".tc.qq.com"))
    {
        return Err(invalid());
    }
    Ok(url.to_string())
}
