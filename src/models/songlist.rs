//! Songlist (歌单) API 返回模型定义 (对应 Python 端 `models/songlist.py`).

use serde::Deserialize;

use super::base::{Song, SongList};
use crate::jsonpath_model;

/// 歌单创建者信息.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SonglistCreator {
    pub musicid: i64,
    pub nick: String,
    pub headurl: String,
    pub encrypt_uin: String,
}

/// 歌单基础元数据.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SonglistInfo {
    #[serde(flatten)]
    pub base: SongList,
    pub creator: SonglistCreator,
}

/// 歌单详情响应.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct GetSonglistDetailResponse {
    pub code: i64,
    pub subcode: i64,
    pub msg: String,
    #[serde(alias = "dirinfo")]
    pub info: SonglistInfo,
    #[serde(alias = "songlist_size")]
    pub size: i64,
    #[serde(alias = "songlist")]
    pub songs: Vec<Song>,
    #[serde(alias = "total_song_num")]
    pub total: i64,
    pub hasmore: i64,
}

// 创建 / 删除歌单响应.
jsonpath_model!(CreateDeleteSonglistResp {
    retCode: "$.retCode" => i64,
    id: "$.result.tid" => i64,
    dirid: "$.result.dirId" => i64,
    name: "$.result.dirName" => String,
});
