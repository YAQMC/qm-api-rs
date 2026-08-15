//! Search API 返回模型定义 (对应 Python 端 `models/search.py`).

use serde::Deserialize;
use serde_json::Value;

use super::base::{Album, Singer, Song, SongList, MV};
use crate::jsonpath_model;

/// 搜索场景下的歌曲详尽模型.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SongSearch {
    #[serde(flatten)]
    pub base: Song,
    pub search_title: String,
    pub title_main: String,
    pub title_extra: String,
    pub fav_show: String,
    pub desc: String,
    pub desc_icon: String,
    pub hotness: Value,
    pub hotness_desc: String,
    pub vec_hotness: Vec<Value>,
    pub content: String,
    #[serde(alias = "newStatus")]
    pub new_status: i64,
    pub protect: i64,
    pub relatedword_group: Value,
}

/// 搜索场景下的专辑详尽模型.
#[derive(Debug, Clone, Default)]
pub struct AlbumSearch {
    pub base: Album,
    pub album_type: i64,
    pub award_label: String,
    pub desc_detail: Value,
    pub description: String,
    pub description2: String,
    pub hotness: Value,
    pub hotness_desc: String,
    pub label_new: Value,
    pub audio_play: RankingInfo,
    pub pic: String,
    pub pic_icon: String,
    pub singer: String,
    pub singer_list: Vec<Singer>,
    pub tag_list: Vec<String>,
    pub url: String,
}

impl<'de> Deserialize<'de> for AlbumSearch {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let raw = Value::deserialize(de)?;
        let base = serde_json::from_value(raw.clone()).unwrap_or_default();
        let mut out = AlbumSearch {
            base,
            ..Default::default()
        };
        out.album_type = crate::jsonpath::extract_typed(&raw, "$.core_album_config.album_type");
        out.award_label = crate::jsonpath::extract_typed(&raw, "$.core_album_config.award_label");
        out.singer = crate::jsonpath::extract_typed(&raw, "$.singer");
        out.singer_list = crate::jsonpath::extract_typed(&raw, "$.singer_list");
        out.pic = crate::jsonpath::extract_typed(&raw, "$.pic");
        out.pic_icon = crate::jsonpath::extract_typed(&raw, "$.pic_icon");
        out.description = crate::jsonpath::extract_typed(&raw, "$.description");
        out.description2 = crate::jsonpath::extract_typed(&raw, "$.description2");
        out.desc_detail = crate::jsonpath::extract_typed(&raw, "$.desc_detail");
        out.hotness = crate::jsonpath::extract_typed(&raw, "$.hotness");
        out.hotness_desc = crate::jsonpath::extract_typed(&raw, "$.hotness_desc");
        out.label_new = crate::jsonpath::extract_typed(&raw, "$.label_new");
        out.audio_play = crate::jsonpath::extract_typed(&raw, "$.audio_play");
        out.tag_list = crate::jsonpath::extract_typed(&raw, "$.tag_list");
        out.url = crate::jsonpath::extract_typed(&raw, "$.url");
        Ok(out)
    }
}

/// 搜索专辑排行信息.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RankingInfo {
    pub rank: String,
    pub toplist: String,
}

/// 搜索结果中的歌单摘要.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SongListSearch {
    #[serde(flatten)]
    pub base: SongList,
    pub nickname: String,
    pub dirtype: i64,
}

/// 搜索场景下的歌手模型.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SingerSearch {
    #[serde(flatten)]
    pub base: Singer,
    #[serde(alias = "singerPic")]
    pub pic: String,
    #[serde(alias = "songNum")]
    pub song_num: i64,
    #[serde(alias = "albumNum")]
    pub album_num: i64,
    #[serde(alias = "mvNum")]
    pub mv_num: i64,
    pub subtitle: String,
}

/// 搜索场景下的 MV 模型.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct MvSearch {
    #[serde(flatten)]
    pub base: MV,
    #[serde(alias = "pic")]
    pub pic: String,
    #[serde(alias = "play_count")]
    pub play_count: i64,
    pub duration: i64,
    #[serde(alias = "publish_date")]
    pub publish_date: String,
    #[serde(alias = "singerid")]
    pub singer_id: i64,
    #[serde(alias = "singermid")]
    pub singer_mid: String,
    #[serde(alias = "singername")]
    pub singer_name: String,
}

/// 搜索筛选器选项.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SearchSelector {
    pub id: i64,
    pub name: String,
    pub r#type: i64,
}

jsonpath_model!(SearchByTypeResponse {
    searchid: "$.meta.searchid" => String,
    perpage: "$.meta.perpage" => i64,
    nextpage: "$.meta.nextpage" => i64,
    estimate_sum: "$.meta.estimate_sum" => i64,
    total_num: "$.meta.sum" => i64,
    song: "$.body.item_song" => Vec<SongSearch>,
    singer: "$.body.singer" => Vec<SingerSearch>,
    album: "$.body.item_album" => Vec<AlbumSearch>,
    songlist: "$.body.item_songlist" => Vec<SongListSearch>,
    user: "$.body.item_user" => Vec<Value>,
    audio_alum: "$.body.item_audio" => Vec<AlbumSearch>,
    mv: "$.body.item_mv" => Vec<MvSearch>,
    selectors: "$.body.multi_extern_info.selectors" => Vec<Vec<SearchSelector>>,
});

/// 综合搜索单分类结果容器.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct GeneralSearchBody<T> {
    pub estimate_sum: i64,
    pub total_num: i64,
    pub items: Vec<T>,
    pub more_info: Value,
}

jsonpath_model!(GeneralSearchResponse {
    searchid: "$.meta.sid" => String,
    perpage: "$.meta.perpage" => i64,
    nextpage: "$.meta.nextpage" => i64,
    nextpage_start: "$.meta.nextpage_start" => Value,
    song: "$.body.item_song" => GeneralSearchBody<SongSearch>,
    singer: "$.body.singer" => GeneralSearchBody<SingerSearch>,
    mv: "$.body.item_mv" => GeneralSearchBody<MvSearch>,
    album: "$.body.item_album" => GeneralSearchBody<AlbumSearch>,
    songlist: "$.body.item_songlist" => GeneralSearchBody<SongListSearch>,
    audio: "$.body.item_audio" => GeneralSearchBody<AlbumSearch>,
    direct: "$.body.direct_result.direct_group" => Vec<Value>,
    related: "$.body.item_related" => GeneralSearchBody<RelatedSearchWord>,
});

/// 相关搜索词推荐.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct RelatedSearchWord {
    #[serde(alias = "display_word")]
    pub display: String,
    #[serde(alias = "search_word")]
    pub search: String,
}

/// 快速搜索条目.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct QuickSearchItem {
    pub docid: String,
    pub id: String,
    pub mid: String,
    pub name: String,
    pub singer: String,
    pub pic: String,
    pub vid: String,
}

/// 快速搜索分类.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct QuickSearchCategory {
    pub count: i64,
    pub itemlist: Vec<QuickSearchItem>,
    pub name: String,
    pub order: i64,
    pub r#type: i64,
}

jsonpath_model!(QuickSearchResponse {
    song: "$.data.song" => QuickSearchCategory,
    singer: "$.data.singer" => QuickSearchCategory,
    album: "$.data.album" => QuickSearchCategory,
    mv: "$.data.mv" => QuickSearchCategory,
});

/// 热搜词条目.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Hotkey {
    pub hotkey_id: String,
    pub query: String,
    pub title: String,
    pub score: String,
    pub kind: i64,
    pub r#type: i64,
    pub source: i64,
    pub need_top: i64,
    pub subpos: i64,
    pub song_type: i64,
    pub direct_id: i64,
    pub jump_tab: String,
    pub jump_url: String,
    pub cover_pic_url: String,
    pub pic_url: String,
    pub description: String,
}

/// 热搜词列表响应.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct HotkeyResponse {
    pub ret_code: i64,
    pub hotkey_time: String,
    pub track_list_id: String,
    pub vec_hotkey: Vec<Hotkey>,
    pub vec_reckey: Vec<Value>,
}

/// 搜索补全建议条目.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct CompleteItem {
    pub hint: String,
    pub hint_hilight: String,
    pub docid: String,
    pub r#type: i64,
    pub res_type: String,
    pub score: f64,
    pub pre_search: bool,
    pub icon: String,
    pub icon_type: i64,
    pub jumptab: i64,
    pub jump_type: i64,
    pub jump_url: String,
    pub pic_url: String,
}

/// 搜索词补全响应.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct CompleteResponse {
    pub items: Vec<CompleteItem>,
    pub total_num: i64,
    pub search_id: String,
    pub expire_time: i64,
    pub use_default_search: i64,
    pub debug_info: String,
    pub expid: String,
    pub history_items: Vec<Value>,
    pub vec_direct_items: Vec<Value>,
    pub vec_related_items: Vec<Value>,
}
