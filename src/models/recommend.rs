//! Recommend API 返回模型定义 (对应 Python 端 `models/recommend.py`).

use serde::Deserialize;
use serde_json::Value;

use super::base::{Song, SongList};
use crate::jsonpath_model;

/// 当前登录账号的“今日私享”歌单入口.
///
/// `songlist_id` 是账号相关的临时目录 ID；调用方应将其交给
/// [`crate::modules::SonglistApi`] 获取歌单详情，不应持久化为全局常量.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DailyRecommendationResponse {
    pub songlist_id: i64,
}

/// `get_radio_track` 参数.
///
/// `offset` 对应 wire 字段 `from`; 使用明确字段名避免调用方依赖原始 JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuessRecommendRequest {
    pub limit: u32,
    pub offset: u32,
    pub seed_song_ids: Vec<u64>,
}

impl Default for GuessRecommendRequest {
    fn default() -> Self {
        Self {
            limit: 5,
            offset: 0,
            seed_song_ids: Vec::new(),
        }
    }
}

/// `GetRadarSong` 参数.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RadarRecommendRequest {
    pub page: u32,
    pub request_type: u32,
    pub favorite_song_ids: Vec<u64>,
    pub entrance_song_ids: Vec<u64>,
}

impl Default for RadarRecommendRequest {
    fn default() -> Self {
        Self {
            page: 1,
            request_type: 0,
            favorite_song_ids: Vec::new(),
            entrance_song_ids: Vec::new(),
        }
    }
}

/// 首页推荐楼层中的细分卡片分组.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct RecommendNiche {
    pub id: i64,
    pub title_template: String,
    pub title_content: String,
    #[serde(alias = "v_card")]
    pub cards: Vec<Value>,
}

/// 首页推荐页中的单个楼层.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct RecommendShelf {
    pub id: i64,
    pub title_template: String,
    pub title_content: String,
    pub more: Value,
    #[serde(alias = "v_niche")]
    pub niches: Vec<RecommendNiche>,
}

jsonpath_model!(RecommendFeedCardResponse {
    retcode: "$.retcode" => i64,
    msg: "$.msg" => String,
    prompt: "$.prompt" => String,
    d_num: "$.d_num" => i64,
    load_mark: "$.load_mark" => i64,
    shelves: "$.v_shelf" => Vec<RecommendShelf>,
});

/// 猜你喜欢响应.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct GuessRecommendResponse {
    #[serde(alias = "tracks")]
    pub songs: Vec<Song>,
}

jsonpath_model!(RadarRecommendResponse {
    songs: "$.VecSongs[*].Track" => Vec<Song>,
    recommend_song_ids: "$.RecommendSongIds" => Vec<i64>,
    base_song_ids: "$.BaseSongIds" => Vec<i64>,
    has_more: "$.HasMore" => bool,
    toast: "$.Toast" => String,
    timestamp: "$.TimeStamp" => i64,
    video_cards: "$.VideoCards" => Value,
});

/// 推荐歌单列表中的单个歌单摘要.
#[derive(Debug, Clone, Default)]
pub struct RecommendSonglistItem {
    pub base: SongList,
    pub songnum: i64,
    pub listennum: i64,
    pub creator_nick: String,
}

impl<'de> Deserialize<'de> for RecommendSonglistItem {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let raw = Value::deserialize(de)?;
        Ok(RecommendSonglistItem {
            base: serde_json::from_value(raw.clone()).unwrap_or_default(),
            songnum: crate::jsonpath::extract_typed(&raw, "$.song_cnt"),
            listennum: crate::jsonpath::extract_typed(&raw, "$.play_cnt"),
            creator_nick: crate::jsonpath::extract_typed(&raw, "$.creator.nick"),
        })
    }
}

jsonpath_model!(RecommendSonglistResponse {
    songlists: "$.List[*].Playlist.basic" => Vec<RecommendSonglistItem>,
    has_more: "$.HasMore" => bool,
    from_limit: "$.FromLimit" => i64,
    msg: "$.Msg" => String,
});

/// 推荐新歌页的标签项.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct RecommendNewSongTag {
    pub id: i64,
    pub tagid: i64,
    pub tag: String,
    pub link: String,
    pub from_type: i64,
}

/// 推荐新歌响应.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct RecommendNewSongResponse {
    pub lanlist: Vec<Value>,
    pub lan: String,
    #[serde(alias = "songlist")]
    pub songs: Vec<Song>,
    pub ret_msg: String,
    pub r#type: i64,
    #[serde(alias = "songTagInfoList")]
    pub song_tags: Vec<RecommendNewSongTag>,
}
