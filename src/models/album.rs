//! Album API 返回模型定义 (对应 Python 端 `models/album.py`).

use serde::Deserialize;

use super::base::{Album, Singer, Song};
use crate::jsonpath_model;

/// 专辑详情.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AlbumDetail {
    #[serde(flatten)]
    pub base: Album,
    #[serde(alias = "publishDate")]
    pub time_public: String,
    pub desc: String,
    pub language: String,
    #[serde(alias = "albumType")]
    pub album_type: String,
    pub genre: String,
    pub wikiurl: String,
}

/// 专辑发行公司.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AlbumCompany {
    #[serde(alias = "ID")]
    pub id: i64,
    pub name: String,
    #[serde(alias = "isShow")]
    pub is_show: i64,
    pub brief: String,
}

jsonpath_model!(GetAlbumDetailResponse {
    album: "$.basicInfo" => AlbumDetail,
    company: "$.company" => AlbumCompany,
    singers: "$.singer.singerList" => Vec<Singer>,
});

jsonpath_model!(GetAlbumSongResponse {
    album_mid: "$.albumMid" => String,
    total_num: "$.totalNum" => i64,
    song_list: "$.songList[*].songInfo" => Vec<Song>,
});

/// 新碟上架列表中的单张专辑摘要.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct NewAlbumItem {
    #[serde(flatten)]
    pub base: Album,
    pub singers: Vec<Singer>,
    pub release_time: String,
    pub r#type: i64,
    pub area: i64,
    pub genre: i64,
    pub language: i64,
}

/// 新碟上架接口的响应体.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct GetNewAlbumResponse {
    pub total: i64,
    pub albums: Vec<NewAlbumItem>,
}

/// 收藏 / 取消收藏专辑的写操作响应.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AlbumFavWriteResponse {
    pub result: i64,
    #[serde(alias = "v_failedAlbumId")]
    pub failed_album_id: Vec<i64>,
}

impl AlbumFavWriteResponse {
    pub fn success(&self) -> bool {
        self.result == 0 && self.failed_album_id.is_empty()
    }
}
