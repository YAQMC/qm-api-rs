//! 歌词相关 API (对应 Python 端 `modules/lyric.py`).

use serde_json::json;

use super::ApiModule;
use crate::context::RequestOptions;
use crate::error::Result;
use crate::models::lyric::*;
use crate::versioning::Platform;

/// 歌词相关 API.
#[derive(Clone, Debug)]
pub struct LyricApi {
    pub(crate) base: ApiModule,
}

impl LyricApi {
    pub(crate) fn new(context: std::sync::Arc<crate::context::ApiContext>) -> Self {
        LyricApi {
            base: ApiModule::new(context),
        }
    }

    /// 获取歌词原始数据.
    ///
    /// 固定走网页访客信封: [`Platform::Web`]、未签名 `musicu.fcg`、
    /// `GetPlayLyricInfo`. 不要给这首 CGI 加 zzc (`musics.fcg` 会 24001).
    /// `crypt` 默认省略 (与网页一致); 若服务端仍返回加密字段,
    /// [`GetLyricResponse::parse`] 会就地解密.
    #[allow(clippy::too_many_arguments)]
    pub async fn get_lyric(
        &self,
        value: &str,
        song_type: i64,
        qrc: bool,
        trans: bool,
        roma: bool,
        singing_annotations: bool,
    ) -> Result<GetLyricResponse> {
        self.get_lyric_with_crypt(
            value,
            song_type,
            qrc,
            trans,
            roma,
            singing_annotations,
            None,
        )
        .await
    }

    /// 同 [`Self::get_lyric`], 可显式带 `crypt` (网页默认省略; `Some(1)` 请求加密字段).
    #[allow(clippy::too_many_arguments)]
    pub async fn get_lyric_with_crypt(
        &self,
        value: &str,
        song_type: i64,
        qrc: bool,
        trans: bool,
        roma: bool,
        singing_annotations: bool,
        crypt: Option<i64>,
    ) -> Result<GetLyricResponse> {
        let mut params = json!({
            "lrc_t": 0,
            "qrc": qrc as i64,
            "qrc_t": 0,
            "roma": roma as i64,
            "roma_t": 0,
            "trans": trans as i64,
            "trans_t": 0,
            "needSingingAnnotations": singing_annotations,
            "type": song_type,
        });
        if let Some(crypt) = crypt {
            params["crypt"] = json!(crypt);
        }
        if value.chars().all(|c| c.is_ascii_digit()) && !value.is_empty() {
            params["songID"] = json!(value.parse::<i64>().unwrap_or(0));
        } else {
            params["songMID"] = json!(value);
            params["songMid"] = json!(value);
        }
        let mut opts = RequestOptions::default();
        opts.preserve_bool = true;
        opts.platform = Some(Platform::Web);
        opts.sign = false;
        let data = self
            .base
            .cgi(
                "music.musichallSong.PlayLyricInfo",
                "GetPlayLyricInfo",
                params,
                opts,
            )
            .await?;
        Ok(GetLyricResponse::parse(data)?)
    }

    /// 获取助唱标注歌词信息.
    pub async fn get_singing_annotations_info(
        &self,
        songid: i64,
    ) -> Result<GetSingingAnnotationsInfoResponse> {
        let mut opts = RequestOptions::default();
        opts.preserve_bool = true;
        let data = self
            .base
            .cgi(
                "music.musichallSong.PlayLyricInfo",
                "GetSingingAnnotationsInfo",
                json!({ "songID": songid, "needNum": false }),
                opts,
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取多风格翻译歌词 (如诗意、粤语、方言等).
    pub async fn get_multi_style_trans_lyric(
        &self,
        songid: i64,
    ) -> Result<BatchGetMultiStyleTransLyricResponse> {
        let data = self
            .base
            .cgi(
                "music.musichallSong.PlayLyricInfo",
                "BatchGetMultiStyleTransLyric",
                json!({ "songID": songid }),
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 检查是否存在 AI 歌词词典.
    pub async fn is_ai_dict_exists(&self, songid: i64) -> Result<IsAIDictExistsResponse> {
        let data = self
            .base
            .cgi(
                "music.musichallSong.PlayLyricInfo",
                "IsAIDictExists",
                json!({ "songID": songid }),
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取 AI 歌词词典信息.
    pub async fn get_ai_dict(&self, songid: i64) -> Result<GetAIDictResponse> {
        let data = self
            .base
            .cgi(
                "music.musichallSong.PlayLyricInfo",
                "GetAIDictInfo",
                json!({ "songID": songid }),
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::ApiContext;
    use serde_json::Value;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct CapturedCgi {
        referer: Option<String>,
        origin: Option<String>,
        uri: String,
        body: Value,
    }

    async fn spawn_capturing_cgi(cap: Arc<Mutex<CapturedCgi>>) -> String {
        use axum::{extract::OriginalUri, http::HeaderMap, routing::post, Json, Router};
        let app = Router::new().route(
            "/cgi-bin/musicu.fcg",
            post({
                let cap = cap.clone();
                move |headers: HeaderMap, uri: OriginalUri, Json(body): Json<Value>| {
                    let cap = cap.clone();
                    async move {
                        let mut seen = cap.lock().unwrap();
                        seen.referer = headers
                            .get("referer")
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_string);
                        seen.origin = headers
                            .get("origin")
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_string);
                        seen.uri = uri.0.to_string();
                        seen.body = body;
                        r#"{"code":0,"req_0":{"code":0,"data":{"lyric":"[ti:x]","trans":"","roma":""}}}"#
                    }
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

    fn lyric_api_on_android_client(base: &str) -> LyricApi {
        let mut ctx = ApiContext::new_with_proxy(None, Some(Platform::Android), None).unwrap();
        ctx.cgi_base_url = format!("{base}/cgi-bin");
        LyricApi::new(Arc::new(ctx))
    }

    #[tokio::test]
    async fn get_lyric_uses_unsigned_web_envelope_and_yqq_headers() {
        let cap = Arc::new(Mutex::new(CapturedCgi::default()));
        let base = spawn_capturing_cgi(cap.clone()).await;
        let api = lyric_api_on_android_client(&base);
        api.get_lyric("001X3HEN1oK0Jr", 1, true, true, true, false)
            .await
            .unwrap();
        let seen = cap.lock().unwrap();
        assert_eq!(seen.referer.as_deref(), Some("https://y.qq.com/"));
        assert_eq!(seen.origin.as_deref(), Some("https://y.qq.com"));
        assert!(
            seen.uri.contains("musicu.fcg"),
            "unsigned lyric CGI must hit musicu.fcg, got {}",
            seen.uri
        );
        assert!(
            !seen.uri.contains("musics.fcg") && !seen.uri.contains("sign="),
            "GetPlayLyricInfo must not be zzc-signed, got {}",
            seen.uri
        );
        assert_eq!(seen.body["comm"]["ct"], 24);
        let param = &seen.body["req_0"]["param"];
        assert_eq!(param["songMID"], "001X3HEN1oK0Jr");
        assert_eq!(param["songMid"], "001X3HEN1oK0Jr");
        assert!(param.get("crypt").is_none());
        assert!(param.get("songID").is_none());
        assert_eq!(seen.body["req_0"]["method"], "GetPlayLyricInfo");
    }

    #[tokio::test]
    async fn get_lyric_numeric_id_sends_song_id() {
        let cap = Arc::new(Mutex::new(CapturedCgi::default()));
        let base = spawn_capturing_cgi(cap.clone()).await;
        let api = lyric_api_on_android_client(&base);
        api.get_lyric("123456", 1, false, false, false, false)
            .await
            .unwrap();
        let param = &cap.lock().unwrap().body["req_0"]["param"];
        assert_eq!(param["songID"], 123456);
        assert!(param.get("songMID").is_none());
        assert!(param.get("songMid").is_none());
    }

    #[tokio::test]
    async fn get_lyric_with_crypt_sends_crypt_when_enabled() {
        let cap = Arc::new(Mutex::new(CapturedCgi::default()));
        let base = spawn_capturing_cgi(cap.clone()).await;
        let api = lyric_api_on_android_client(&base);
        api.get_lyric_with_crypt("001X3HEN1oK0Jr", 1, false, false, false, false, Some(1))
            .await
            .unwrap();
        let param = &cap.lock().unwrap().body["req_0"]["param"];
        assert_eq!(param["crypt"], 1);
        assert_eq!(param["songMID"], "001X3HEN1oK0Jr");
        assert_eq!(param["songMid"], "001X3HEN1oK0Jr");
    }
}
