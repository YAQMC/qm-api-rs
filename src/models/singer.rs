//! Singer API 返回模型定义 (对应 Python 端 `models/singer.py`).

use serde::Deserialize;
use serde_json::Value;

use super::base::{Album, Singer, Song, MV};
use crate::jsonpath_model;
use crate::models::de::{null_as_default, str_or_zero};

/// 歌手筛选标签项.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TagOption {
    pub id: i64,
    pub name: String,
}

/// 歌手列表条目.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SingerBrief {
    #[serde(flatten)]
    pub base: Singer,
    pub area_id: i64,
    pub country_id: i64,
    pub country: String,
    pub other_name: String,
    pub spell: String,
    pub trend: i64,
    #[serde(alias = "concernNum")]
    pub concern_num: i64,
    pub singer_pic: String,
}

/// 歌手筛选标签集合.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SingerTagData {
    #[serde(deserialize_with = "null_as_default")]
    pub area: Vec<TagOption>,
    #[serde(deserialize_with = "null_as_default")]
    pub genre: Vec<TagOption>,
    #[serde(deserialize_with = "null_as_default")]
    pub sex: Vec<TagOption>,
    #[serde(deserialize_with = "null_as_default")]
    pub index: Vec<TagOption>,
}

/// 歌手列表响应.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SingerTypeListResponse {
    pub area: i64,
    pub sex: i64,
    pub genre: i64,
    #[serde(deserialize_with = "null_as_default")]
    pub singerlist: Vec<SingerBrief>,
    pub code: i64,
    #[serde(deserialize_with = "null_as_default")]
    pub hotlist: Vec<SingerBrief>,
    pub tags: SingerTagData,
}

/// 按索引分页的歌手列表响应.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SingerIndexPageResponse {
    pub index: i64,
    pub total: i64,
    #[serde(flatten)]
    pub base: SingerTypeListResponse,
}

/// 歌手主页基础信息.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct HomepageBaseInfo {
    #[serde(alias = "EncryptedUin")]
    pub encrypted_uin: String,
    #[serde(alias = "BackgroundImage")]
    pub background_image: String,
    #[serde(alias = "Avatar")]
    pub avatar: String,
    #[serde(alias = "Name")]
    pub name: String,
    #[serde(alias = "IsHost")]
    pub is_host: i64,
    #[serde(alias = "IsSinger")]
    pub is_singer: i64,
    #[serde(alias = "UserType")]
    pub user_type: i64,
}

/// 歌手主页歌手信息.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct HomepageSinger {
    #[serde(alias = "SingerID", alias = "singerID")]
    pub id: i64,
    #[serde(alias = "SingerMid", alias = "singerMid")]
    pub mid: String,
    #[serde(alias = "Name")]
    pub name: String,
    #[serde(alias = "SingerType")]
    pub r#type: i64,
    #[serde(alias = "SingerPic")]
    pub singer_pic: String,
    #[serde(alias = "SingerPMid")]
    pub singer_pmid: String,
}

/// 主页标签元信息.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TabMeta {
    #[serde(alias = "TabID")]
    pub tab_id: String,
    #[serde(alias = "TabName")]
    pub tab_name: String,
    #[serde(alias = "Title")]
    pub title: String,
}

/// 歌手相关专辑条目.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AlbumBrief {
    #[serde(flatten)]
    pub base: Album,
    #[serde(alias = "totalNum")]
    pub total_num: i64,
    #[serde(alias = "albumType")]
    pub album_type: String,
    #[serde(alias = "singerName")]
    pub singer_name: String,
    pub tags: Vec<String>,
}

/// 歌手视频条目.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct VideoBrief {
    #[serde(flatten)]
    pub base: MV,
    pub picurl: String,
    pub picformat: i64,
    pub duration: i64,
    pub playcnt: i64,
    pub pubdate: i64,
    pub icon_type: i64,
}

jsonpath_model!(HomepageTabDetailResponse {
    tab_id: "$.TabID" => String,
    has_more: "$.HasMore" => i64,
    need_show_tab: "$.NeedShowTab" => i64,
    order: "$.Order" => i64,
    tab_list: "$.TabList" => Vec<TabMeta>,
    introduction_tab: "$.IntroductionTab.List" => Vec<Value>,
    song_tab: "$.SongTab.List[*]" => Vec<Song>,
    album_tab: "$.AlbumTab.AlbumList[*]" => Vec<AlbumBrief>,
    video_tab: "$.VideoTab.VideoList[*]" => Vec<VideoBrief>,
});

jsonpath_model!(HomepageHeaderResponse {
    status: "$.Status" => i64,
    singer: "$.Info.Singer" => HomepageSinger,
    base_info: "$.Info.BaseInfo" => HomepageBaseInfo,
    tab_detail: "$.TabDetail" => HomepageTabDetailResponse,
    prompt: "$.Prompt" => Value,
});

/// 歌手详情基础信息.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SingerBasicInfo {
    #[serde(alias = "singer_id")]
    pub id: i64,
    #[serde(alias = "singer_mid")]
    pub mid: String,
    pub name: String,
    pub r#type: i64,
    #[serde(alias = "singer_pmid")]
    pub pmid: String,
    #[serde(alias = "has_photo")]
    pub has_photo: i64,
    pub wikiurl: String,
}

/// 歌手详情扩展信息.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SingerExtraInfo {
    #[serde(deserialize_with = "str_or_zero")]
    pub area: String,
    pub desc: String,
    pub tag: String,
    #[serde(deserialize_with = "str_or_zero")]
    pub identity: String,
    #[serde(deserialize_with = "str_or_zero")]
    pub instrument: String,
    #[serde(deserialize_with = "str_or_zero")]
    pub genre: String,
    pub foreign_name: String,
    pub birthday: String,
    #[serde(deserialize_with = "str_or_zero")]
    pub enter: String,
    #[serde(alias = "blogFlag")]
    pub blog_flag: i64,
}

/// 歌手图片地址集合.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SingerPic {
    pub big_black: String,
    pub big_white: String,
    pub pic: String,
}

/// 歌手相册图片项.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SingerPhotoItem {
    pub big: String,
    pub small: String,
}

/// 歌手详情条目.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SingerDetail {
    pub basic_info: SingerBasicInfo,
    pub ex_info: SingerExtraInfo,
    pub wiki: String,
    #[serde(deserialize_with = "null_as_default")]
    pub group_list: Vec<Value>,
    pub pic: SingerPic,
    #[serde(deserialize_with = "null_as_default")]
    pub photos: Vec<SingerPhotoItem>,
    #[serde(deserialize_with = "null_as_default")]
    pub group_info: Vec<Value>,
}

/// 歌手详情响应.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SingerDetailResponse {
    #[serde(deserialize_with = "null_as_default")]
    pub singer_list: Vec<SingerDetail>,
}

/// 相似歌手条目.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SimilarSinger {
    #[serde(flatten)]
    pub base: Singer,
    #[serde(alias = "singerPic")]
    pub singer_pic: String,
    pub trace: String,
    pub abt: String,
    pub tf: String,
}

/// 相似歌手列表响应.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SimilarSingerResponse {
    #[serde(deserialize_with = "null_as_default")]
    pub singerlist: Vec<SimilarSinger>,
    pub code: i64,
    #[serde(alias = "errMsg")]
    pub err_msg: String,
}

jsonpath_model!(SingerSongListResponse {
    singer_mid: "$.singerMid" => String,
    total_num: "$.totalNum" => i64,
    song_list: "$.songList[*].songInfo" => Vec<Song>,
});

jsonpath_model!(SingerAlbumListResponse {
    singer_mid: "$.singerMid" => String,
    total: "$.total" => i64,
    album_list: "$.albumList" => Vec<AlbumBrief>,
});

jsonpath_model!(SingerMvListResponse {
    total: "$.total" => i64,
    mv_list: "$.list" => Vec<VideoBrief>,
});
