//! 歌词相关 API (对应 Python 端 `modules/lyric.py`).

use serde_json::json;

use super::ApiModule;
use crate::context::RequestOptions;
use crate::error::Result;
use crate::models::lyric::*;

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
        let mut params = json!({
            "crypt": 1,
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
        if value.chars().all(|c| c.is_ascii_digit()) && !value.is_empty() {
            params["songId"] = json!(value.parse::<i64>().unwrap_or(0));
        } else {
            params["songMid"] = json!(value);
        }
        let mut opts = RequestOptions::default();
        opts.preserve_bool = true;
        let data = self
            .base
            .cgi("music.musichallSong.PlayLyricInfo", "GetPlayLyricInfo", params, opts)
            .await?;
        Ok(GetLyricResponse::parse(data)?)
    }

    /// 获取助唱标注歌词信息.
    pub async fn get_singing_annotations_info(&self, songid: i64) -> Result<GetSingingAnnotationsInfoResponse> {
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
    pub async fn get_multi_style_trans_lyric(&self, songid: i64) -> Result<BatchGetMultiStyleTransLyricResponse> {
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
