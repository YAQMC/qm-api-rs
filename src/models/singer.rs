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
    #[serde(alias = "ID", alias = "id")]
    pub id: i64,
    #[serde(alias = "Title", alias = "title", alias = "name")]
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

/// 服务端下发的 Tab 筛选项.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TabFilterItem {
    #[serde(alias = "ID", alias = "id")]
    pub id: i64,
    #[serde(alias = "Title", alias = "title", alias = "name")]
    pub title: String,
}

/// 服务端下发的 Tab 筛选列表.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TabFilterList {
    #[serde(alias = "DefaultID")]
    pub default_id: i64,
    #[serde(alias = "ItemList")]
    pub items: Vec<TabFilterItem>,
}

/// 歌手名称特殊展示信息.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SingerNameSpecialDisplay {
    #[serde(alias = "DisplayType")]
    pub display_type: i64,
    #[serde(alias = "PicFile")]
    pub pic_file: String,
    #[serde(alias = "SignatureNameOverlapRatio")]
    pub signature_name_overlap_ratio: f64,
    #[serde(alias = "Name")]
    pub name: String,
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
    #[serde(alias = "SingerNameSpecialDisplay", alias = "singerNameSpecialDisplay")]
    pub name_special_display: Option<SingerNameSpecialDisplay>,
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
    #[serde(deserialize_with = "null_as_default")]
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
    song_tag_info_list: "$.SongTab.SongTagInfoList" => Vec<Value>,
    album_type_list: "$.AlbumTab.TypeList" => TabFilterList,
    video_tag_list: "$.VideoTab.TagList" => Vec<TabFilterItem>,
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

/// A malformed row must not silently erase an entire album page.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SingerAlbumListResponse {
    pub singer_mid: String,
    pub total: i64,
    #[serde(default, deserialize_with = "null_as_default")]
    pub album_list: Vec<AlbumBrief>,
}

jsonpath_model!(SingerMvListResponse {
    total: "$.total" => i64,
    mv_list: "$.list" => Vec<VideoBrief>,
});

jsonpath_model!(SingerMvTagResponse {
    tags: "$.list[*]" => Vec<TagOption>,
});

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn homepage_singer_keeps_special_name_display() {
        let singer: HomepageSinger = serde_json::from_value(json!({
            "SingerMid":"mid",
            "Name":"Taylor Swift",
            "SingerNameSpecialDisplay":{
                "DisplayType":2,
                "PicFile":"https://example.test/name.png",
                "SignatureNameOverlapRatio":0.25,
                "Name":"Taylor Swift"
            }
        }))
        .unwrap();
        let special = singer.name_special_display.unwrap();
        assert_eq!(special.display_type, 2);
        assert_eq!(special.pic_file, "https://example.test/name.png");
        assert_eq!(special.signature_name_overlap_ratio, 0.25);
    }

    #[test]
    fn tab_filters_and_mv_tags_decode_server_alias_shapes() {
        let tab: HomepageTabDetailResponse = serde_json::from_value(json!({
            "AlbumTab":{"TypeList":{"DefaultID":0,"ItemList":[{"ID":3,"Title":"录音室"}]}},
            "VideoTab":{"TagList":[{"ID":8,"Title":"官方"}]},
            "SongTab":{"SongTagInfoList":[{"foo":"bar"}]}
        }))
        .unwrap();
        assert_eq!(tab.album_type_list.items[0].id, 3);
        assert_eq!(tab.video_tag_list[0].title, "官方");
        assert_eq!(tab.song_tag_info_list[0]["foo"], "bar");

        let tags: SingerMvTagResponse =
            serde_json::from_value(json!({"list":[{"ID":7,"Title":"现场"}]})).unwrap();
        assert_eq!(tags.tags[0].id, 7);
        assert_eq!(tags.tags[0].name, "现场");
    }
}
