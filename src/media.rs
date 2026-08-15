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

use crate::error::{ErrorCategory, QmError, Result};
use crate::models::song::GetSongUrlsResponse;
use crate::models::{Credential, Song};
use crate::modules::song::{FileTypeLike, SongApi, SongFileInfo, SongQuality};

fn redacted(s: &str) -> &'static str {
    if s.is_empty() {
        ""
    } else {
        "[redacted]"
    }
}

/// 单个可播放来源的描述 (播放器直接消费, 不携带下载/解密逻辑).
#[derive(Clone)]
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

impl std::fmt::Debug for MediaSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaSource")
            .field("song_id", &self.song_id)
            .field("song_mid", &self.song_mid)
            .field("quality", &self.quality)
            .field("url", &redacted(&self.url))
            .field("ekey", &redacted(&self.ekey))
            .field("encrypted", &self.encrypted)
            .field("file_ext", &self.file_ext)
            .field("result", &self.result)
            .field("expires_in_secs", &self.expires_in_secs)
            .finish_non_exhaustive()
    }
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
        return Err(unplayable_error(&source));
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
/// (`result == 0` 且 URL 非空). 仅在当前档位表示 **权限不足 / 该音质不可播**
/// 时降级; 限流、鉴权失效、协议错误、传输错误立即返回, 不会连打下一档.
/// `result == 0` 但 `purl` 为空视为协议数据不一致, 不伪装成 VIP 权限错误.
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
        let err = unplayable_error(&source);
        if is_quality_unavailable(&err) {
            last_permission_err = Some(err);
            continue;
        }
        return Err(err);
    }
    Err(last_permission_err.unwrap_or_else(|| QmError::ApiData("歌曲无可用音质".into())))
}

/// `result == 0 && url empty` 是数据不一致, 不能构造成 `CgiApi(code=0)` 再声称需要 VIP.
pub(crate) fn unplayable_error(source: &MediaSource) -> QmError {
    if source.result == 0 {
        QmError::Protocol {
            stage: "media-url",
            message: "result is 0 but playback URL is empty".into(),
        }
    } else if source.result == 104003 {
        QmError::CgiApi {
            code: source.result,
            data: "无播放权限, 需要对应 VIP 权益".into(),
        }
    } else {
        QmError::CgiApi {
            code: source.result,
            data: format!("播放链接不可用 (result={})", source.result),
        }
    }
}

fn is_quality_unavailable(err: &QmError) -> bool {
    err.category() == ErrorCategory::Permission
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

    #[test]
    fn debug_redacts_playback_secrets() {
        let src = build_source(
            "/C400001X3HEN1oK0Jr.mflac?vkey=SUPERSECRET&ekey=REAL",
            0,
            SongQuality::Flac.file_type(true),
        );
        let dbg = format!("{src:?}");
        assert!(dbg.contains("song_mid"));
        assert!(dbg.contains("[redacted]"));
        assert!(!dbg.contains("SUPERSECRET"));
        assert!(!dbg.contains("ekey-1"));
        assert!(!dbg.contains("/C400001X3HEN1oK0Jr"));
        assert!(!dbg.contains("isure.stream.qqmusic.qq.com"));
        let item = crate::models::song::UrlinfoItem {
            mid: "m".into(),
            filename: "f".into(),
            purl: "/secret.mflac?vkey=PLAYSECRET".into(),
            vkey: "VKEYSECRET".into(),
            ekey: "EKEYSECRET".into(),
            result: 0,
        };
        let item_dbg = format!("{item:?}");
        assert!(item_dbg.contains("[redacted]"));
        assert!(!item_dbg.contains("PLAYSECRET"));
        assert!(!item_dbg.contains("VKEYSECRET"));
        assert!(!item_dbg.contains("EKEYSECRET"));
    }

    #[test]
    fn result_zero_empty_purl_is_protocol_not_vip() {
        let src = build_source("", 0, SongQuality::Flac.file_type(false));
        let err = unplayable_error(&src);
        assert!(matches!(
            err,
            QmError::Protocol {
                stage: "media-url",
                ..
            }
        ));
        assert_ne!(err.category(), ErrorCategory::Permission);
        assert!(!is_quality_unavailable(&err));
    }

    #[test]
    fn permission_result_is_degradable() {
        let src = build_source("", 104003, SongQuality::Flac.file_type(true));
        let err = unplayable_error(&src);
        assert_eq!(err.category(), ErrorCategory::Permission);
        assert!(is_quality_unavailable(&err));
    }

    #[test]
    fn rate_limit_result_is_not_degradable() {
        let src = build_source("", 2001, SongQuality::Flac.file_type(false));
        let err = unplayable_error(&src);
        assert_eq!(err.category(), ErrorCategory::RateLimit);
        assert!(!is_quality_unavailable(&err));
    }

    fn song_with_master_and_flac() -> Song {
        let mut song = Song {
            id: 1,
            mid: "001X3HEN1oK0Jr".into(),
            ..Default::default()
        };
        song.file.media_mid = "001X3HEN1oK0Jr".into();
        song.file.size_new = vec![100];
        song.file.size_flac = 50;
        song
    }

    async fn spawn_vkey_mock(
        handler: impl Fn(String) -> (i64, String) + Send + Sync + 'static,
    ) -> String {
        use axum::extract::Json;
        use axum::routing::post;
        use axum::Router;
        use serde_json::Value;
        use std::sync::Arc;
        let handler = Arc::new(handler);
        let app = Router::new().route(
            "/cgi-bin/musicu.fcg",
            post(move |Json(payload): Json<Value>| {
                let handler = handler.clone();
                async move {
                    let method = payload["req_0"]["method"].as_str().unwrap_or("");
                    assert!(
                        method == "CgiGetEVkey" || method == "UrlGetVkey",
                        "unexpected method {method}"
                    );
                    let filename = payload["req_0"]["param"]["filename"][0]
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                    let (result, purl) = handler(filename);
                    format!(
                        r#"{{"code":0,"req_0":{{"code":0,"data":{{"expiration":80400,"midurlinfo":[{{"songmid":"001X3HEN1oK0Jr","filename":"f","purl":"{purl}","vkey":"vk","ekey":"ek","result":{result}}}]}}}}}}"#
                    )
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn best_playable_degrades_permission_then_returns_flac() {
        use std::sync::Arc;
        let base = spawn_vkey_mock(|filename| {
            if filename.contains("AIM0") || filename.contains("AI00") {
                (104003, String::new())
            } else {
                (0, "/C400ok.flac".into())
            }
        })
        .await;
        let mut ctx =
            crate::context::ApiContext::new(None, Some(crate::versioning::Platform::Web)).unwrap();
        ctx.cgi_base_url = format!("{base}/cgi-bin");
        let api = SongApi::new(Arc::new(ctx));
        let src = best_playable(&api, &song_with_master_and_flac(), None, true)
            .await
            .unwrap();
        assert_eq!(src.quality, SongQuality::Flac);
        assert!(src.playable());
        assert!(src.url.contains("/C400ok.flac"));
    }

    #[tokio::test]
    async fn best_playable_does_not_degrade_on_rate_limit() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;
        let hits = Arc::new(AtomicU32::new(0));
        let hits2 = hits.clone();
        let base = spawn_vkey_mock(move |_filename| {
            hits2.fetch_add(1, Ordering::SeqCst);
            (2001, String::new())
        })
        .await;
        let mut ctx =
            crate::context::ApiContext::new(None, Some(crate::versioning::Platform::Web)).unwrap();
        ctx.cgi_base_url = format!("{base}/cgi-bin");
        let api = SongApi::new(Arc::new(ctx));
        let err = best_playable(&api, &song_with_master_and_flac(), None, true)
            .await
            .unwrap_err();
        assert_eq!(err.category(), ErrorCategory::RateLimit);
        assert_eq!(
            hits.load(Ordering::SeqCst),
            1,
            "must not try the next quality"
        );
    }

    #[tokio::test]
    async fn best_playable_does_not_degrade_on_result_zero_empty_purl() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;
        let hits = Arc::new(AtomicU32::new(0));
        let hits2 = hits.clone();
        let base = spawn_vkey_mock(move |_filename| {
            hits2.fetch_add(1, Ordering::SeqCst);
            (0, String::new())
        })
        .await;
        let mut ctx =
            crate::context::ApiContext::new(None, Some(crate::versioning::Platform::Web)).unwrap();
        ctx.cgi_base_url = format!("{base}/cgi-bin");
        let api = SongApi::new(Arc::new(ctx));
        let err = best_playable(&api, &song_with_master_and_flac(), None, true)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            QmError::Protocol {
                stage: "media-url",
                ..
            }
        ));
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }
}
