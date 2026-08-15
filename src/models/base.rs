//! 基础业务实体模型 (对应 Python 端 `models/base.py`).

use serde::Deserialize;

fn _cover_url(kind: &str, mid: &str, size: i32) -> String {
    let mid = mid.trim();
    if mid.is_empty() {
        return String::new();
    }
    let seg = match size {
        150 => "R150x150",
        300 => "R300x300",
        500 => "R500x500",
        800 => "R800x800",
        1200 => "R1200x1200",
        _ => "R1500x1500",
    };
    format!("https://y.gtimg.cn/music/photo_new/{kind}{seg}M000{mid}.jpg")
}

/// 歌手摘要.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Singer {
    #[serde(alias = "singerID", alias = "singerId", alias = "SingerID", alias = "singer_id")]
    pub id: i64,
    #[serde(alias = "singerMid", alias = "singerMID", alias = "SingerMid", alias = "singer_mid")]
    pub mid: String,
    #[serde(alias = "singerName", alias = "singer_name")]
    pub name: String,
    pub title: String,
    #[serde(alias = "SingerType", alias = "vt")]
    pub r#type: i64,
    pub uin: i64,
    #[serde(alias = "singerPmid", alias = "singer_pmid", alias = "pic_mid")]
    pub pmid: String,
}

impl Singer {
    pub fn cover_url(&self, size: i32) -> String {
        let mid = if self.mid.is_empty() { &self.pmid } else { &self.mid };
        _cover_url("T001", mid, size)
    }
}

/// 专辑摘要.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Album {
    #[serde(alias = "albumID", alias = "albumId")]
    pub id: i64,
    #[serde(alias = "albumMid", alias = "albumMID", alias = "albummid")]
    pub mid: String,
    #[serde(alias = "albumName")]
    pub name: String,
    pub title: String,
    #[serde(alias = "albumTranName")]
    pub subtitle: String,
    #[serde(alias = "publish_date", alias = "publishDate")]
    pub time_public: String,
    #[serde(alias = "logo")]
    pub pmid: String,
}

impl Album {
    pub fn cover_url(&self, size: i32) -> String {
        let mid = if self.mid.is_empty() { &self.pmid } else { &self.mid };
        _cover_url("T002", mid, size)
    }
}

/// 歌曲文件信息.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct File {
    pub media_mid: String,
    pub size_24aac: i64,
    pub size_48aac: i64,
    pub size_96aac: i64,
    pub size_192ogg: i64,
    pub size_192aac: i64,
    pub size_128mp3: i64,
    pub size_320mp3: i64,
    pub size_flac: i64,
    pub size_dts: i64,
    pub size_try: i64,
    pub try_begin: i64,
    pub try_end: i64,
    pub size_96ogg: i64,
    pub size_dolby: i64,
    pub size_new: Vec<i64>,
}

/// 付费属性.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Pay {
    pub pay_month: i64,
    pub price_track: i64,
    pub price_album: i64,
    pub pay_play: i64,
    pub pay_down: i64,
    pub pay_status: i64,
    pub time_free: i64,
}

/// MV 摘要.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct MV {
    #[serde(alias = "sid", alias = "mvid", alias = "singerId")]
    pub id: i64,
    pub vid: String,
    #[serde(alias = "vt")]
    pub r#type: i64,
    #[serde(alias = "mvname")]
    pub name: String,
    #[serde(alias = "title_main")]
    pub title: String,
}

/// 歌单摘要.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SongList {
    #[serde(alias = "tid", alias = "dissid")]
    pub id: i64,
    #[serde(alias = "dirId")]
    pub dirid: i64,
    #[serde(alias = "dissname", alias = "dirName")]
    pub title: String,
    #[serde(alias = "cover", alias = "picUrl")]
    pub picurl: String,
    #[serde(alias = "description")]
    pub desc: String,
    #[serde(alias = "songNum", alias = "song_cnt")]
    pub songnum: i64,
    #[serde(alias = "playCnt", alias = "play_cnt")]
    pub listennum: i64,
}

/// 歌曲基础模型.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Song {
    pub id: i64,
    pub mid: String,
    pub name: String,
    pub r#type: i64,
    pub title: String,
    pub subtitle: String,
    pub singer: Vec<Singer>,
    pub album: Album,
    pub mv: MV,
    pub file: File,
    pub pay: Pay,
    pub interval: i64,
    pub isonly: i64,
    pub language: i64,
    pub genre: i64,
    pub index_cd: i64,
    pub index_album: i64,
    pub time_public: String,
    pub status: i64,
    pub label: String,
    pub bpm: i64,
    pub ov: i64,
    pub sa: i64,
    pub es: String,
    pub vs: Vec<String>,
    pub vi: Vec<i64>,
    pub vf: Vec<f64>,
}

impl Song {
    pub fn cover_url(&self, size: i32) -> String {
        if !self.album.mid.is_empty() || !self.album.pmid.is_empty() {
            return self.album.cover_url(size);
        }
        for singer in &self.singer {
            if !singer.mid.is_empty() || !singer.pmid.is_empty() {
                return singer.cover_url(size);
            }
        }
        String::new()
    }
}
