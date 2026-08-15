//! 可选媒体层: 播放来源描述 (source descriptor) 与下载/解密助手.
//!
//! 播放器侧的职责边界:
//! - 本模块只负责**描述**可播放来源 (`MediaSource`: URL + 元数据), 以及
//!   面向 CLI 下载器 / 非播放链路的**下载与解密**助手;
//! - 不会侵入播放器 pipeline (YAQMC 等宿主自行决定如何消费 `MediaSource`);
//! - 需要流式解密时, 宿主可基于 `MediaSource { url, ekey, encrypted }` 自行实现.

use crate::error::{QmError, Result};
use crate::models::song::GetSongUrlsResponse;
use crate::models::{Credential, Song};
use crate::modules::song::{SongApi, SongFileInfo, SongQuality};

/// 单个可播放来源的描述 (播放器直接消费, 不携带下载/解密逻辑).
#[derive(Debug, Clone)]
pub struct MediaSource {
    /// 歌曲 ID.
    pub song_id: i64,
    /// 歌曲 MID.
    pub song_mid: String,
    /// 音质档位.
    pub quality: SongQuality,
    /// 完整播放地址 (含 CDN 前缀, 可直接请求).
    pub url: String,
    /// 解密密钥 (`CgiGetEVkey` 返回的 ekey, 未加密音质为空).
    pub ekey: String,
    /// 是否加密音质 (`.mflac` / `.mgg` 等).
    pub encrypted: bool,
    /// 文件扩展名 (如 `mflac` / `flac` / `mp3`).
    pub file_ext: String,
    /// 播放链接结果码 (`0` 正常; `104003` 无权限等).
    pub result: i64,
}

impl MediaSource {
    /// 解析歌曲最高可用音质的来源描述.
    ///
    /// - `allow_encrypted`: 是否允许加密音质 (走 `CgiGetEVkey`, 返回 `.mflac` 等).
    /// - `credential`: VIP 账号凭证 (高音质通常需要绿钻).
    pub async fn best(
        api: &SongApi,
        song: &Song,
        credential: Option<&Credential>,
        allow_encrypted: bool,
    ) -> Result<Self> {
        let (quality, urls) = api.get_best_song_url(song, credential, allow_encrypted).await?;
        Self::from_urls(api, song, quality, urls)
    }

    /// 解析指定音质的来源描述.
    pub async fn resolve(
        api: &SongApi,
        song: &Song,
        quality: SongQuality,
        credential: Option<&Credential>,
        allow_encrypted: bool,
    ) -> Result<Self> {
        let file_type = quality.file_type(allow_encrypted);
        let urls = api
            .get_song_urls(
                &[SongFileInfo::new(&song.mid)
                    .with_song_type(song.r#type)
                    .with_media_mid(&song.file.media_mid)
                    .with_type_ref(file_type)],
                file_type,
                credential,
            )
            .await?;
        Self::from_urls(api, song, quality, urls)
    }

    fn from_urls(
        api: &SongApi,
        song: &Song,
        quality: SongQuality,
        urls: GetSongUrlsResponse,
    ) -> Result<Self> {
        let item = urls
            .data
            .first()
            .ok_or_else(|| QmError::ApiData("获取播放链接失败".into()))?;
        let url = format!("{}{}", api._song_url_fallback_domain, item.purl);
        let encrypted = quality.has_encrypted() && !item.ekey.is_empty();
        let file_ext = quality.file_type(true).e().trim_start_matches('.').to_string();
        Ok(MediaSource {
            song_id: song.id,
            song_mid: song.mid.clone(),
            quality,
            url,
            ekey: item.ekey.clone(),
            encrypted,
            file_ext,
            result: item.result,
        })
    }
}

/// 下载并解密指定音质的音频 (媒体层助手, 面向 CLI 下载器 / 非播放链路).
///
/// 获取加密音质链接 → 下载 `.mflac/.mgg` → QMC 解密 → 返回 (音频字节, 扩展名).
///
/// - 需要 `credential` 为有权限的 VIP 账号.
/// - 返回错误时查看 `MediaSource.result` (如 `104003` 无权限).
pub async fn download_quality(
    api: &SongApi,
    song: &Song,
    quality: SongQuality,
    credential: Option<&Credential>,
) -> Result<(Vec<u8>, String)> {
    let source = MediaSource::resolve(api, song, quality, credential, true).await?;
    if source.url.is_empty() {
        return Err(QmError::ApiData(format!(
            "无播放权限 (result={}), 需要对应 VIP 权益",
            source.result
        )));
    }
    let bytes = api
        .base
        .context
        .request_http_bytes(&source.url, credential)
        .await?;
    if source.encrypted && !source.ekey.is_empty() {
        crate::qmc::decrypt_qmc(&bytes, Some(&source.ekey))
    } else {
        // 未加密音质: 直接返回, 扩展名按实际内容嗅探.
        let ext = crate::qmc::detect_audio_extension(&bytes);
        let ext = if ext == "bin" {
            source.file_ext.clone()
        } else {
            ext
        };
        Ok((bytes, ext))
    }
}

/// 下载并解密歌曲最高可用音质 (媒体层助手).
pub async fn download_best(
    api: &SongApi,
    song: &Song,
    credential: Option<&Credential>,
) -> Result<(SongQuality, Vec<u8>, String)> {
    let available = api.available_qualities(song);
    let quality = *available
        .first()
        .ok_or_else(|| QmError::ApiData("歌曲无可用音质".into()))?;
    let (audio, ext) = download_quality(api, song, quality, credential).await?;
    Ok((quality, audio, ext))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_source_fields_exposed() {
        // MediaSource 是纯数据描述: 无隐藏网络/解密逻辑, 字段可直接构造.
        let src = MediaSource {
            song_id: 1,
            song_mid: "mid".into(),
            quality: SongQuality::Flac,
            url: "https://example.com/a.flac".into(),
            ekey: String::new(),
            encrypted: false,
            file_ext: "flac".into(),
            result: 0,
        };
        assert_eq!(src.quality, SongQuality::Flac);
        assert!(!src.encrypted);
    }
}
