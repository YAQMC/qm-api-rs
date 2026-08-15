//! Top (排行榜) API 返回模型定义 (对应 Python 端 `models/top.py`).

use serde::Deserialize;
use serde_json::Value;

use super::base::Song;
use crate::jsonpath_model;

/// 排行榜预览歌曲条目.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TopPreviewSong {
    pub rank: i64,
    #[serde(alias = "rankType")]
    pub rank_type: i64,
    #[serde(alias = "rankValue")]
    pub rank_value: String,
    #[serde(alias = "songId")]
    pub id: i64,
    #[serde(alias = "title")]
    pub name: String,
    #[serde(alias = "singerName")]
    pub singer_name: String,
    #[serde(alias = "singerMid")]
    pub singer_mid: String,
    #[serde(alias = "albumMid")]
    pub album_mid: String,
    pub cover: String,
    #[serde(alias = "mvid")]
    pub mv_id: i64,
}

/// 排行榜摘要信息.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TopSummary {
    #[serde(alias = "topId")]
    pub id: i64,
    #[serde(alias = "title")]
    pub name: String,
    #[serde(alias = "titleDetail")]
    pub title_detail: String,
    #[serde(alias = "titleSub")]
    pub title_sub: String,
    pub intro: String,
    pub period: String,
    #[serde(alias = "updateTime")]
    pub update_time: String,
    #[serde(alias = "listenNum")]
    pub listen_num: i64,
    #[serde(alias = "totalNum")]
    pub total_num: i64,
    #[serde(alias = "song")]
    pub songs: Vec<TopPreviewSong>,
    #[serde(alias = "frontPicUrl")]
    pub front_pic_url: String,
    #[serde(alias = "headPicUrl")]
    pub head_pic_url: String,
    #[serde(alias = "h5JumpUrl")]
    pub h5_jump_url: String,
    #[serde(alias = "specialScheme")]
    pub special_scheme: String,
}

/// 排行榜分类.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TopCategory {
    #[serde(alias = "groupId")]
    pub id: i64,
    #[serde(alias = "groupName")]
    pub name: String,
    pub toplist: Vec<TopSummary>,
}

/// 排行榜分类响应.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TopCategoryResponse {
    pub group: Vec<TopCategory>,
}

jsonpath_model!(TopDetailResponse {
    info: "$.data" => TopSummary,
    songs: "$.songInfoList" => Vec<Song>,
    song_tags: "$.songTagInfoList" => Vec<Value>,
    ext_info_list: "$.extInfoList" => Vec<Value>,
    index_info_list: "$.indexInfoList" => Vec<Value>,
});
