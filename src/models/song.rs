//! Song API 返回模型定义 (对应 Python 端 `models/song.py`).

use serde::Deserialize;
use serde_json::Value;

use super::base::{Singer, Song, SongList, MV};
use crate::jsonpath_model;
use crate::models::de::null_as_default;

/// 批量歌曲查询响应.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct QuerySongResponse {
    #[serde(default)]
    pub tracks: Vec<Song>,
}

/// 单个文件授权结果.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct UrlinfoItem {
    #[serde(rename = "songmid")]
    pub mid: String,
    pub filename: String,
    pub purl: String,
    pub vkey: String,
    pub ekey: String,
    pub result: i64,
}

/// 歌曲播放地址响应.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct GetSongUrlsResponse {
    #[serde(default)]
    pub expiration: i64,
    #[serde(default, alias = "midurlinfo")]
    pub data: Vec<UrlinfoItem>,
}

/// 歌曲详情内容项.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ContentItem {
    pub id: i64,
    pub value: String,
    pub show_type: i64,
    pub jumpurl: String,
}

jsonpath_model!(GetSongDetailResponse {
    company: "$.info.company.content" => default(Vec<ContentItem>),
    genre: "$.info.genre.content" => default(Vec<ContentItem>),
    intro: "$.info.intro.content" => default(Vec<ContentItem>),
    lan: "$.info.lan.content" => default(Vec<ContentItem>),
    pub_time: "$.info.pub_time.content" => default(Vec<ContentItem>),
    extras: "$.info.extras" => default(Value),
    track: "$.track_info" => strict(Song),
});

/// 相似歌曲推荐分组.
#[derive(Debug, Clone, Default)]
pub struct SimilarSongGroup {
    pub title_template: String,
    pub title_content: String,
    pub song: Vec<Song>,
}

impl<'de> Deserialize<'de> for SimilarSongGroup {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let raw = Value::deserialize(de)?;
        Ok(SimilarSongGroup {
            title_template: raw
                .get("title_template")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            title_content: raw
                .get("title_content")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            song: crate::jsonpath::extract_typed::<Vec<Song>>(&raw, "$.songs[*].track"),
        })
    }
}

jsonpath_model!(GetSimilarSongResponse {
    tag: "$.songTagInfoList" => Vec<Value>,
    song: "$.vecSongNew" => Vec<SimilarSongGroup>,
});

/// 歌曲标签项.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct SongLabel {
    pub id: i64,
    #[serde(alias = "tagTxt")]
    pub tag_txt: String,
    #[serde(alias = "tagIcon")]
    pub tag_icon: String,
    #[serde(alias = "tagUrl")]
    pub tag_url: String,
    #[serde(alias = "tagType")]
    pub tag_type: i64,
    pub species: i64,
}

/// 获取歌曲标签结果.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct GetSongLabelsResponse {
    #[serde(default)]
    pub labels: Vec<SongLabel>,
}

/// 歌曲关联歌单.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct RelatedPlaylist {
    #[serde(default)]
    pub creator: String,
    #[serde(flatten)]
    pub base: SongList,
}

jsonpath_model!(GetRelatedSonglistResponse {
    has_more: "$.hasMore" => i64,
    songlist: "$.vecPlaylistNew[*].playlists[*]" => Vec<RelatedPlaylist>,
});

/// 歌曲关联 MV.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct RelatedMv {
    pub picurl: String,
    pub playcnt: i64,
    #[serde(default)]
    pub singers: Vec<Singer>,
    #[serde(flatten)]
    pub base: MV,
}

jsonpath_model!(GetRelatedMvResponse {
    has_more: "$.hasmore" => i64,
    mv: "$.list" => Vec<RelatedMv>,
});

jsonpath_model!(GetOtherVersionResponse {
    data: "$.versionList" => Vec<Song>,
});

/// 歌曲制作人项.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct SongProducer {
    #[serde(alias = "Type")]
    pub r#type: i64,
    #[serde(alias = "Name")]
    pub name: String,
    #[serde(alias = "Icon")]
    pub icon: String,
    #[serde(alias = "Scheme")]
    pub scheme: String,
    #[serde(alias = "SingerMid")]
    pub singer_mid: String,
    #[serde(alias = "Follow")]
    pub follow: i64,
}

/// 歌曲制作人分组.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct SongProducerGroup {
    #[serde(alias = "Title")]
    pub title: String,
    #[serde(alias = "Producers")]
    pub producers: Vec<SongProducer>,
    #[serde(alias = "Type")]
    pub r#type: i64,
}

jsonpath_model!(GetProducerResponse {
    data: "$.Lst" => Vec<SongProducerGroup>,
    reinforce_msg: "$.ReinforceMsg" => String,
});

/// 曲谱项.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct SheetMusic {
    #[serde(alias = "scoreMID")]
    pub score_mid: String,
    #[serde(alias = "scoreName")]
    pub score_name: String,
    #[serde(alias = "picURLs", deserialize_with = "null_as_default")]
    pub pic_urls: Vec<String>,
    pub version: String,
    pub tonality: i64,
    #[serde(alias = "scoreType")]
    pub score_type: i64,
    #[serde(alias = "strScoreType")]
    pub score_type_text: String,
    pub uploader: String,
    #[serde(alias = "viewFrequency")]
    pub view_frequency: i64,
    pub tonality2: i64,
    pub author: String,
    pub composer: String,
    pub lyricist: String,
    pub singer: String,
    pub performer: String,
    #[serde(alias = "songMID")]
    pub song_mid: String,
    #[serde(alias = "subName")]
    pub sub_name: String,
    pub url: String,
    #[serde(alias = "albumURL")]
    pub album_url: String,
    #[serde(alias = "insType")]
    pub ins_type: i64,
    #[serde(alias = "strInsType")]
    pub ins_type_text: String,
    #[serde(alias = "coverURL")]
    pub cover_url: String,
    pub difficulty: String,
    #[serde(alias = "sheetFile")]
    pub sheet_file: String,
}

/// 曲谱来源类型.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SheetType {
    /// 用户上传.
    User = 0,
    /// 引擎 / AI 曲谱.
    EngineAi = 1,
    /// 虫虫钢琴.
    ChongChong = 2,
}

/// 曲谱响应.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct GetSheetResponse {
    #[serde(deserialize_with = "null_as_default")]
    pub result: Vec<SheetMusic>,
    #[serde(alias = "totalMap", default)]
    pub total_map: std::collections::HashMap<String, i64>,
}

/// 检查曲谱存在状态响应.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct HasSheetMusicResponse {
    #[serde(alias = "hasGuitar")]
    pub has_guitar: bool,
    #[serde(alias = "hasMore")]
    pub has_more: bool,
    #[serde(alias = "hasLDY")]
    pub has_ldy: bool,
    #[serde(alias = "hasQRCX")]
    pub has_qrcx: bool,
    #[serde(alias = "hasChongChong")]
    pub has_chong_chong: bool,
}

jsonpath_model!(GetFavNumResponse {
    numbers: "$.m_numbers" => std::collections::HashMap<String, i64>,
    show: "$.m_show" => std::collections::HashMap<String, String>,
});

/// CDN 调度节点信息.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct CdnDispatchSipInfo {
    pub cdn: String,
    pub quic: i64,
    pub ipstack: i64,
    pub quichost: String,
    #[serde(default, alias = "plaintextquic")]
    pub plaintext_quic: i64,
    #[serde(default, alias = "encryptquic")]
    pub encrypt_quic: i64,
}

/// 音频 CDN 调度响应.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct GetCdnDispatchResponse {
    pub retcode: i64,
    #[serde(default)]
    pub sip: Vec<String>,
    #[serde(default)]
    pub sipinfo: Vec<CdnDispatchSipInfo>,
    #[serde(alias = "keepalivefile")]
    pub test_file: String,
    pub expiration: i64,
    #[serde(alias = "refreshTime")]
    pub refresh_time: i64,
    #[serde(alias = "cacheTime")]
    pub cache_time: i64,
}
