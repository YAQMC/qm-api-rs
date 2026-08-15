//! 可选媒体层: 播放来源描述 (source descriptor) 与下载/解密助手.
//!
//! 播放器侧的职责边界:
//! - 本模块只负责**描述**可播放来源 (`MediaSource`: URL + 元数据), 以及
//!   面向 CLI 下载器 / 非播放链路的**下载与解密**助手;
//! - 不会侵入播放器 pipeline (YAQMC 等宿主自行决定如何消费 `MediaSource`);
//! - 需要流式解密时, 宿主可基于 `MediaSource { url, ekey, encrypted }` 自行实现.
//!
//! 语义约定:
//! - `MediaSource.url` 在**无播放权限** (如 `result = 104003`, `purl` 为空) 时
//!   保持为空字符串, 不会拼出一个假的 CDN 根地址; 消费方应先检查 `playable()`.
//! - `MediaSource.expires_in_secs` 是**有效期 (TTL)**, 不是 Unix 时间戳
//!   (QQ Music 返回形如 `80400`, 约 22.3 小时); 绝对 deadline 请用
//!   `resolved_at + expires_in_secs`.

use crate::error::{QmError, Result};
use crate::models::song::GetSongUrlsResponse;
use crate::models::{Credential, Song};
use crate::modules::song::{FileTypeLike, SongApi, SongFileInfo, SongQuality};

/// 单个可播放来源的描述 (播放器直接消费, 不携带下载/解密逻辑).
#[derive(Debug, Clone)]
pub struct MediaSource {
    /// 歌曲 ID.
    pub song_id: i64,
    /// 歌曲 MID.
    pub song_mid: String,
    /// 音质档位.
    pub quality: SongQuality,
    /// 完整播放地址 (含 CDN 前缀, 可直接请求); 无播放权限时为空字符串.
    pub url: String,
    /// 解密密钥 (`CgiGetEVkey` 返回的 ekey, 未加密音质为空).
    pub ekey: String,
    /// 是否加密音质 (`.mflac` / `.mgg` 等).
    pub encrypted: bool,
    /// 文件扩展名 (如 `mflac` / `flac` / `mp3`), 对应本次实际选中的 file_type.
    pub file_ext: String,
    /// 播放链接结果码 (`0` 正常; `104003` 无权限等).
    pub result: i64,
    /// 链接有效期 (秒, **TTL 而非时间戳**, 来自 `GetSongUrlsResponse.expiration`).
    pub expires_in_secs: u64,
    /// 解析时刻 (用于计算绝对 deadline = `resolved_at + expires_in_secs`).
    pub resolved_at: std::time::Instant,
}

impl MediaSource {
    /// 是否可获得实际播放地址 (无权限或 `result != 0` 时不可播).
    pub fn playable(&self) -> bool {
        self.result == 0 && !self.url.is_empty()
    }

    /// 绝对过期时刻 (解析时刻 + TTL), 用于 URL 刷新调度.
    pub fn deadline(&self) -> std::time::Instant {
        self.resolved_at + std::time::Duration::from_secs(self.expires_in_secs)
    }

    /// 解析歌曲最高可用音质的来源描述 (按文件能力选档, 不保证账号可播).
    ///
    /// - `allow_encrypted`: 是否允许加密音质 (走 `CgiGetEVkey`, 返回 `.mflac` 等).
    /// - `credential`: VIP 账号凭证 (高音质通常需要绿钻).
    ///
    /// 返回的 `MediaSource.url` 可能为空 (账号无对应权益); 消费方用
    /// `playable()` 判断后再做降级.
    pub async fn best(
        api: &SongApi,
        song: &Song,
        credential: Option<&Credential>,
        allow_encrypted: bool,
    ) -> Result<Self> {
        let (quality, urls) = api
            .get_best_song_url(song, credential, allow_encrypted)
            .await?;
        Self::from_urls(api, song, quality, quality.file_type(allow_encrypted), urls)
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
        Self::from_urls(api, song, quality, file_type, urls)
    }

    /// 依据原始响应构造来源描述.
    ///
    /// `file_type` 必须是本次请求实际选中的文件类型 (而非按
    /// `allow_encrypted=true` 重新推导), 否则 `file_ext`/`encrypted` 会失真.
    fn from_urls(
        api: &SongApi,
        song: &Song,
        quality: SongQuality,
        file_type: &'static dyn FileTypeLike,
        urls: GetSongUrlsResponse,
    ) -> Result<Self> {
        let item = urls
            .data
            .first()
            .ok_or_else(|| QmError::ApiData("获取播放链接失败".into()))?;
        // purl 为空 (如 result=104003 无权限) 时, 不拼出假的 CDN 根地址.
        let url = if item.purl.is_empty() {
            String::new()
        } else {
            format!("{}{}", api._song_url_fallback_domain, item.purl)
        };
        let encrypted = file_type.is_encrypted() && !item.ekey.is_empty();
        let file_ext = file_type.e().trim_start_matches('.').to_string();
        Ok(MediaSource {
            song_id: song.id,
            song_mid: song.mid.clone(),
            quality,
            url,
            ekey: item.ekey.clone(),
            encrypted,
            file_ext,
            result: item.result,
            expires_in_secs: urls.expiration.max(0) as u64,
            resolved_at: std::time::Instant::now(),
        })
    }
}

/// 下载并解密指定音质的音频 (媒体层助手, 面向 CLI 下载器 / 非播放链路).
///
/// 获取加密音质链接 → 下载 `.mflac/.mgg` → QMC 解密 → 返回 (音频字节, 扩展名).
///
/// - 需要 `credential` 为有权限的 VIP 账号.
/// - 无播放权限时返回错误 (见 `MediaSource.result`, 如 `104003`).
pub async fn download_quality(
    api: &SongApi,
    song: &Song,
    quality: SongQuality,
    credential: Option<&Credential>,
) -> Result<(Vec<u8>, String)> {
    let source = MediaSource::resolve(api, song, quality, credential, true).await?;
    if !source.playable() {
        // 保留真实业务码 (如 104003), 供 category()/is_retryable() 正确分类.
        return Err(QmError::CgiApi {
            code: source.result,
            data: "无播放权限, 需要对应 VIP 权益".into(),
        });
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

/// 下载并解密歌曲**实际可播放**的最高音质 (媒体层助手).
///
/// 按可用音质从高到低尝试; 账号无权限 (如 `result=104003`) 的音质会跳过,
/// 直到找到第一个可播放的音质. 全部不可播时返回最后一个权限错误.
pub async fn download_best(
    api: &SongApi,
    song: &Song,
    credential: Option<&Credential>,
) -> Result<(SongQuality, Vec<u8>, String)> {
    let available = api.available_qualities(song);
    if available.is_empty() {
        return Err(QmError::ApiData("歌曲无可用音质".into()));
    }
    let mut last_permission_err: Option<QmError> = None;
    for quality in available {
        match download_quality(api, song, quality, credential).await {
            Ok((audio, ext)) => return Ok((quality, audio, ext)),
            Err(e) if e.category() == crate::error::ErrorCategory::Permission => {
                // 该音质无播放权限, 降级到下一档.
                last_permission_err = Some(e);
                continue;
            }
            Err(e) => return Err(e),
        }
    }
    Err(last_permission_err.unwrap_or_else(|| QmError::ApiData("歌曲无可用音质".into())))
}

/// 解析歌曲**最高实际可播放**音质的来源描述 (只 resolve, 不下载).
///
/// 按可用音质从高到低逐个 resolve, 返回第一个 `playable()` 的来源
/// (`result == 0` 且 URL 非空); 全部不可播时返回最后一个权限错误.
/// 这是 YAQMC Provider 获取播放来源的推荐入口.
pub async fn best_playable(
    api: &SongApi,
    song: &Song,
    credential: Option<&Credential>,
    allow_encrypted: bool,
) -> Result<MediaSource> {
    let available = api.available_qualities(song);
    if available.is_empty() {
        return Err(QmError::ApiData("歌曲无可用音质".into()));
    }
    let mut last_permission_err: Option<QmError> = None;
    for quality in available {
        let source = MediaSource::resolve(api, song, quality, credential, allow_encrypted).await?;
        if source.playable() {
            return Ok(source);
        }
        last_permission_err = Some(QmError::CgiApi {
            code: source.result,
            data: "无播放权限, 需要对应 VIP 权益".into(),
        });
    }
    Err(last_permission_err.unwrap_or_else(|| QmError::ApiData("歌曲无可用音质".into())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::song::FileTypeLike;

    fn domain() -> &'static str {
        "https://isure.stream.qqmusic.qq.com/"
    }

    fn build_source(purl: &str, result: i64, file_type: &'static dyn FileTypeLike) -> MediaSource {
        use std::sync::Arc;
        let urls = GetSongUrlsResponse {
            expiration: 80_400,
            data: vec![crate::models::song::UrlinfoItem {
                mid: "mid".into(),
                filename: "f.mflac".into(),
                purl: purl.into(),
                vkey: String::new(),
                ekey: if file_type.is_encrypted() {
                    "ekey-1".into()
                } else {
                    String::new()
                },
                result,
            }],
        };
        let song = Song {
            id: 1,
            mid: "mid".into(),
            ..Default::default()
        };
        let ctx = crate::context::ApiContext::new(None, None).unwrap();
        let api_owned = SongApi::new(Arc::new(ctx));
        MediaSource::from_urls(&api_owned, &song, SongQuality::Flac, file_type, urls).unwrap()
    }

    #[test]
    fn empty_purl_produces_empty_url() {
        // 无权限: purl 为空 → url 必须为空, 不能变成 CDN 根地址.
        let src = build_source("", 104003, SongQuality::Flac.file_type(true));
        assert_eq!(src.url, "");
        assert!(!src.playable());
        assert_eq!(src.result, 104003);
    }

    #[test]
    fn non_empty_purl_with_error_result_is_not_playable() {
        // result != 0 时即使 purl 非空也视为不可播.
        let src = build_source("/a.mflac?k=1", 104003, SongQuality::Flac.file_type(true));
        assert!(!src.url.is_empty());
        assert!(!src.playable());
    }

    #[test]
    fn non_empty_purl_produces_full_url() {
        let src = build_source("/a.mflac?k=1", 0, SongQuality::Flac.file_type(true));
        assert_eq!(src.url, format!("{}{}", domain(), "/a.mflac?k=1"));
        assert!(src.playable());
    }

    #[test]
    fn file_type_reflects_actual_request() {
        // allow_encrypted=false 时使用普通文件类型: 扩展名/加密标志一致.
        let plain = build_source("/a.flac", 0, SongQuality::Flac.file_type(false));
        assert_eq!(plain.file_ext, "flac");
        assert!(!plain.encrypted);
        assert!(plain.ekey.is_empty());

        let encrypted = build_source("/a.mflac", 0, SongQuality::Flac.file_type(true));
        assert_eq!(encrypted.file_ext, "mflac");
        assert!(encrypted.encrypted);
        assert!(!encrypted.ekey.is_empty());
    }

    #[test]
    fn expiration_is_ttl_not_timestamp() {
        // QQ Music 返回 80400 ≈ 22.3h 的有效期 (TTL), 不是 Unix 时间戳.
        let src = build_source("/a.flac", 0, SongQuality::Flac.file_type(false));
        assert_eq!(src.expires_in_secs, 80_400);
        assert!(src.deadline() > std::time::Instant::now());
    }

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
            expires_in_secs: 3600,
            resolved_at: std::time::Instant::now(),
        };
        assert_eq!(src.quality, SongQuality::Flac);
        assert!(!src.encrypted);
        assert!(src.playable());
    }
}
