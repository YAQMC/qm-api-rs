//! 歌曲相关 API 模块 (对应 Python 端 `modules/song.py`).

use serde_json::{json, Value};

use super::ApiModule;
use crate::context::RequestOptions;
use crate::error::{QmError, Result};
use crate::models::song::*;
use crate::models::Credential;
use crate::models::Song;
use crate::utils::get_guid;

/// 歌曲音质档位 (按优先级从高到低).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SongQuality {
    /// 臻品母带 (AI00 / AIM0).
    Master,
    /// 杜比全景声 (D004 / D0M4).
    AtmosDb,
    /// 臻品音质 2.0 (Q000 / Q0M0).
    Atmos2,
    /// 臻品全景声 5.1 (Q001 / Q0M1).
    Atmos51,
    /// 臻品全景声 7.1 (Q003 / Q0M3).
    Atmos71,
    /// 腾讯自研 AICodec (TL01 / TLM1).
    Nac,
    /// SQ 无损 (F000 / F0M0).
    Flac,
    /// SQ 无损 OGG (O801 / O8M1).
    Ogg640,
    /// HQ 高品质 OGG (O800 / O8M0).
    Ogg320,
    /// HQ 高品质 MP3 (M800).
    Mp3_320,
    /// HQ 高品质 OGG 192 (O600 / O6M0).
    Ogg192,
    /// 流畅音质 OGG 96 (O400 / O4M0).
    Ogg96,
    /// HQ 高品质 AAC 192 (C600).
    Acc192,
    /// 标准音质 MP3 128 (M500).
    Mp3_128,
    /// 流畅音质 AAC 96 (C400).
    Acc96,
    /// 低品质 AAC 48 (C200).
    Acc48,
    /// DTS:X (DT03 / DTM3).
    DtsX,
}

impl SongQuality {
    /// 全部音质档位 (按优先级从高到低).
    pub const ALL: &'static [SongQuality] = &[
        SongQuality::Master,
        SongQuality::AtmosDb,
        SongQuality::Atmos2,
        SongQuality::Atmos51,
        SongQuality::Atmos71,
        SongQuality::Nac,
        SongQuality::Flac,
        SongQuality::Ogg640,
        SongQuality::Ogg320,
        SongQuality::Mp3_320,
        SongQuality::Ogg192,
        SongQuality::Ogg96,
        SongQuality::Acc192,
        SongQuality::Mp3_128,
        SongQuality::Acc96,
        SongQuality::Acc48,
        SongQuality::DtsX,
    ];

    /// 播放优先级 (越小越优先).
    pub fn priority(&self) -> u8 {
        match self {
            SongQuality::Master => 0,
            SongQuality::AtmosDb => 1,
            SongQuality::Atmos2 => 2,
            SongQuality::Atmos51 => 3,
            SongQuality::Atmos71 => 4,
            SongQuality::Nac => 5,
            SongQuality::Flac => 6,
            SongQuality::Ogg640 => 7,
            SongQuality::Ogg320 => 8,
            SongQuality::Mp3_320 => 9,
            SongQuality::Ogg192 => 10,
            SongQuality::Ogg96 => 11,
            SongQuality::Acc192 => 12,
            SongQuality::Mp3_128 => 13,
            SongQuality::Acc96 => 14,
            SongQuality::Acc48 => 15,
            SongQuality::DtsX => 16,
        }
    }

    /// 是否支持加密版本 (走 `CgiGetEVkey`).
    pub fn has_encrypted(&self) -> bool {
        matches!(
            self,
            SongQuality::Master
                | SongQuality::AtmosDb
                | SongQuality::Atmos2
                | SongQuality::Atmos51
                | SongQuality::Atmos71
                | SongQuality::Nac
                | SongQuality::Flac
                | SongQuality::Ogg640
                | SongQuality::Ogg320
                | SongQuality::Ogg192
                | SongQuality::Ogg96
                | SongQuality::DtsX
        )
    }

    /// 获取实际使用的文件类型.
    pub fn file_type(&self, encrypted: bool) -> &'static dyn FileTypeLike {
        if encrypted {
            match self {
                SongQuality::Master => &EncryptedSongFileType::Master,
                SongQuality::AtmosDb => &EncryptedSongFileType::AtmosDb,
                SongQuality::Atmos2 => &EncryptedSongFileType::Atmos2,
                SongQuality::Atmos51 => &EncryptedSongFileType::Atmos51,
                SongQuality::Atmos71 => &EncryptedSongFileType::Atmos71,
                SongQuality::Nac => &EncryptedSongFileType::Nac,
                SongQuality::Flac => &EncryptedSongFileType::Flac,
                SongQuality::Ogg640 => &EncryptedSongFileType::Ogg640,
                SongQuality::Ogg320 => &EncryptedSongFileType::Ogg320,
                SongQuality::Ogg192 => &EncryptedSongFileType::Ogg192,
                SongQuality::Ogg96 => &EncryptedSongFileType::Ogg96,
                SongQuality::DtsX => &EncryptedSongFileType::DtsX,
                _ => self.plain_file_type(),
            }
        } else {
            self.plain_file_type()
        }
    }

    fn plain_file_type(&self) -> &'static dyn FileTypeLike {
        match self {
            SongQuality::Master => &SongFileType::Master,
            SongQuality::AtmosDb => &SongFileType::AtmosDb,
            SongQuality::Atmos2 => &SongFileType::Atmos2,
            SongQuality::Atmos51 => &SongFileType::Atmos51,
            SongQuality::Atmos71 => &SongFileType::Atmos71,
            SongQuality::Nac => &SongFileType::Nac,
            SongQuality::Flac => &SongFileType::Flac,
            SongQuality::Ogg640 => &SongFileType::Ogg640,
            SongQuality::Ogg320 => &SongFileType::Ogg320,
            SongQuality::Mp3_320 => &SongFileType::Mp3_320,
            SongQuality::Ogg192 => &SongFileType::Ogg192,
            SongQuality::Ogg96 => &SongFileType::Ogg96,
            SongQuality::Acc192 => &SongFileType::Acc192,
            SongQuality::Mp3_128 => &SongFileType::Mp3_128,
            SongQuality::Acc96 => &SongFileType::Acc96,
            SongQuality::Acc48 => &SongFileType::Acc48,
            SongQuality::DtsX => &SongFileType::DtsX,
        }
    }

    /// 读取对应 `file` 尺寸字段 (0 表示该音质不存在).
    pub fn size(&self, file: &crate::models::File) -> i64 {
        let new = |i: usize| file.size_new.get(i).copied().unwrap_or(0);
        match self {
            SongQuality::Master => new(0),
            SongQuality::Atmos2 => new(1),
            SongQuality::Atmos51 => new(2),
            SongQuality::Ogg320 => new(3),
            SongQuality::Ogg640 => new(5),
            SongQuality::Atmos71 => new(6),
            SongQuality::Nac => new(7),
            SongQuality::Flac => file.size_flac,
            SongQuality::AtmosDb => file.size_dolby,
            SongQuality::Ogg192 => file.size_192ogg,
            SongQuality::Ogg96 => file.size_96ogg,
            SongQuality::Mp3_320 => file.size_320mp3,
            SongQuality::Mp3_128 => file.size_128mp3,
            SongQuality::Acc192 => file.size_192aac,
            SongQuality::Acc96 => file.size_96aac,
            SongQuality::Acc48 => file.size_48aac,
            SongQuality::DtsX => file.size_dts,
        }
    }
}

/// 歌曲文件类型特征 (起始编码 + 扩展名).
pub trait FileTypeLike: std::fmt::Debug + Send + Sync {
    fn s(&self) -> &'static str;
    fn e(&self) -> &'static str;
    /// 是否为加密文件类型 (走 `GetEVkey` 接口).
    fn is_encrypted(&self) -> bool {
        false
    }
}

macro_rules! file_type_enum {
    ($name:ident { $($variant:ident: ($s:expr, $e:expr)),* $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name {
            $($variant),*
        }
        impl $name {
            pub const ALL: &'static [Self] = &[$(Self::$variant),*];
        }
        impl FileTypeLike for $name {
            fn s(&self) -> &'static str {
                match self { $(Self::$variant => $s),* }
            }
            fn e(&self) -> &'static str {
                match self { $(Self::$variant => $e),* }
            }
        }
    };
}

macro_rules! file_type_enum_encrypted {
    ($name:ident { $($variant:ident: ($s:expr, $e:expr)),* $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name {
            $($variant),*
        }
        impl $name {
            pub const ALL: &'static [Self] = &[$(Self::$variant),*];
        }
        impl FileTypeLike for $name {
            fn s(&self) -> &'static str {
                match self { $(Self::$variant => $s),* }
            }
            fn e(&self) -> &'static str {
                match self { $(Self::$variant => $e),* }
            }
            fn is_encrypted(&self) -> bool {
                true
            }
        }
    };
}

// 普通歌曲文件类型.
file_type_enum!(SongFileType {
    DtsX: ("DT03", ".mp4"),
    Master: ("AI00", ".flac"),
    Atmos2: ("Q000", ".flac"),
    Atmos51: ("Q001", ".flac"),
    Atmos71: ("Q003", ".ogg"),
    AtmosDb: ("D004", ".mp4"),
    Nac: ("TL01", ".nac"),
    Flac: ("F000", ".flac"),
    Ogg640: ("O801", ".ogg"),
    Ogg320: ("O800", ".ogg"),
    Ogg192: ("O600", ".ogg"),
    Ogg96: ("O400", ".ogg"),
    Mp3_320: ("M800", ".mp3"),
    Mp3_128: ("M500", ".mp3"),
    Acc192: ("C600", ".m4a"),
    Acc96: ("C400", ".m4a"),
    Acc48: ("C200", ".m4a"),
});

// 加密歌曲文件类型.
file_type_enum_encrypted!(EncryptedSongFileType {
    DtsX: ("DTM3", ".mmp4"),
    Vinyl: ("V0M0", ".mflac"),
    Master: ("AIM0", ".mflac"),
    Atmos2: ("Q0M0", ".mflac"),
    Atmos51: ("Q0M1", ".mflac"),
    Atmos71: ("Q0M3", ".mgg"),
    AtmosDb: ("D0M4", ".mmp4"),
    Nac: ("TLM1", ".mnac"),
    Flac: ("F0M0", ".mflac"),
    Ogg640: ("O8M1", ".mgg"),
    Ogg320: ("O8M0", ".mgg"),
    Ogg192: ("O6M0", ".mgg"),
    Ogg96: ("O4M0", ".mgg"),
});

// 特殊歌曲文件类型 (试听 / 伴奏 / AI 演奏等).
file_type_enum!(SpecialSongFileType {
    Try: ("RS02", ".mp3"),
    TryOgg640: ("O802", ".ogg"),
    Accom: ("O801", ".ogg"),
    Multi: ("O601", ".ogg"),
    Piano: ("AI01", ".ogg"),
    Bayin: ("AI02", ".ogg"),
    Guzheng: ("AI03", ".ogg"),
    Qudi: ("AI04", ".ogg"),
    Hulusi: ("AI05", ".ogg"),
    Suona: ("AI06", ".ogg"),
    Shoudie: ("AI07", ".ogg"),
    Guitar: ("AI08", ".ogg"),
    Drums: ("AI09", ".ogg"),
    Kazoo: ("A200", ".ogg"),
    Therapy: ("AA01", ".ogg"),
});

// 彩铃文件类型.
file_type_enum!(RingSongFileType {
    Ring128: ("R500", ".mp3"),
    Ring96: ("R400", ".m4a"),
    Ring48: ("R200", ".m4a"),
});

/// 歌曲文件信息.
#[derive(Debug)]
pub struct SongFileInfo {
    pub mid: String,
    pub file_type: Option<Box<dyn FileTypeLike + Send + Sync>>,
    pub song_type: Option<i64>,
    pub media_mid: Option<String>,
}

/// 包装一个 `'static` 文件类型引用, 使其可存入 `Box<dyn FileTypeLike>`.
#[derive(Debug)]
struct DynFileType(&'static dyn FileTypeLike);

impl FileTypeLike for DynFileType {
    fn s(&self) -> &'static str {
        self.0.s()
    }
    fn e(&self) -> &'static str {
        self.0.e()
    }
    fn is_encrypted(&self) -> bool {
        self.0.is_encrypted()
    }
}

impl SongFileInfo {
    pub fn new(mid: &str) -> Self {
        SongFileInfo {
            mid: mid.to_string(),
            file_type: None,
            song_type: None,
            media_mid: None,
        }
    }
    pub fn with_type(mut self, t: impl FileTypeLike + 'static) -> Self {
        self.file_type = Some(Box::new(t));
        self
    }
    /// 以 `'static` 引用方式指定文件类型 (用于 `SongQuality::file_type` 返回值).
    pub fn with_type_ref(mut self, t: &'static (dyn FileTypeLike + Send + Sync)) -> Self {
        self.file_type = Some(Box::new(DynFileType(t)));
        self
    }
    pub fn with_song_type(mut self, t: i64) -> Self {
        self.song_type = Some(t);
        self
    }
    pub fn with_media_mid(mut self, m: &str) -> Self {
        self.media_mid = Some(m.to_string());
        self
    }
}

/// 歌曲查询信息.
#[derive(Debug, Clone)]
pub struct SongQueryInfo {
    pub id: Option<i64>,
    pub mid: Option<String>,
    pub song_type: Option<i64>,
}

impl SongQueryInfo {
    pub fn by_id(id: i64) -> Self {
        SongQueryInfo {
            id: Some(id),
            mid: None,
            song_type: None,
        }
    }
    pub fn by_mid(mid: &str) -> Self {
        SongQueryInfo {
            id: None,
            mid: Some(mid.to_string()),
            song_type: None,
        }
    }
}

/// 歌曲相关 API.
#[derive(Clone, Debug)]
pub struct SongApi {
    pub(crate) base: ApiModule,
    pub(crate) _get_song_urls_max_mid: u32,
    pub(crate) _song_url_fallback_domain: &'static str,
}

impl SongApi {
    pub(crate) fn new(context: std::sync::Arc<crate::context::ApiContext>) -> Self {
        SongApi {
            base: ApiModule::new(context),
            _get_song_urls_max_mid: 100,
            _song_url_fallback_domain: "https://isure.stream.qqmusic.qq.com/",
        }
    }

    /// 批量获取歌曲信息.
    pub async fn query_song(&self, song_info: &[SongQueryInfo]) -> Result<QuerySongResponse> {
        if song_info.is_empty() {
            return Err(QmError::ValueError("song_info 不能为空".into()));
        }
        let mut ids = Vec::new();
        let mut mids = Vec::new();
        let mut types = Vec::new();
        for item in song_info {
            match (item.id, item.mid.as_ref()) {
                (None, None) | (Some(_), Some(_)) => {
                    return Err(QmError::ValueError(
                        "SongQueryInfo 必须提供 id 或 mid 且不能同时提供".into(),
                    ));
                }
                (Some(id), None) => ids.push(id),
                (None, Some(mid)) => mids.push(mid.clone()),
            }
            types.push(item.song_type.unwrap_or(0));
        }
        let mut param = json!({
            "ctx": 0,
            "client": 1,
            "types": types,
            "modify_stamp": vec![0; types.len()],
        });
        if !ids.is_empty() {
            param["ids"] = Value::Array(ids.into_iter().map(Value::from).collect());
        }
        if !mids.is_empty() {
            param["mids"] = Value::Array(mids.into_iter().map(Value::from).collect());
        }
        let data = self
            .base
            .cgi(
                "music.trackInfo.UniformRuleCtrl",
                "CgiGetTrackInfo",
                param,
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取音频链接 CDN 信息.
    pub async fn get_cdn_dispatch(&self) -> Result<GetCdnDispatchResponse> {
        let data = self
            .base
            .cgi(
                "music.audioCdnDispatch.cdnDispatch",
                "GetCdnDispatch",
                json!({
                    "guid": get_guid(),
                    "uid": "0",
                    "use_new_domain": 1,
                    "use_ipv6": 1,
                }),
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取歌曲文件链接.
    pub async fn get_song_urls(
        &self,
        file_info: &[SongFileInfo],
        file_type: &dyn FileTypeLike,
        credential: Option<&Credential>,
    ) -> Result<GetSongUrlsResponse> {
        if file_info.len() > self._get_song_urls_max_mid as usize {
            return Err(QmError::ValueError("mid 数量超过上限".into()));
        }
        let encrypted = file_type.is_encrypted();
        let (module, method) = if encrypted {
            ("music.vkey.GetEVkey", "CgiGetEVkey")
        } else {
            ("music.vkey.GetVkey", "UrlGetVkey")
        };

        let mut songmid = Vec::new();
        let mut filename = Vec::new();
        let mut songtype = Vec::new();
        for item in file_info {
            let ft: &dyn FileTypeLike = match item.file_type.as_deref() {
                Some(t) => t,
                None => file_type,
            };
            songmid.push(item.mid.clone());
            filename.push(match &item.media_mid {
                Some(mm) => format!("{}{}{}", ft.s(), mm, ft.e()),
                None => format!("{}{}{}{}", ft.s(), item.mid, item.mid, ft.e()),
            });
            songtype.push(item.song_type.unwrap_or(0));
        }
        let cred = credential
            .cloned()
            .unwrap_or_else(|| self.base.credential());
        let param = json!({
            "uin": cred.str_musicid(),
            "filename": filename,
            "guid": get_guid(),
            "songmid": songmid,
            "songtype": songtype,
            "ctx": 0,
        });
        let mut opts = RequestOptions::default();
        opts.credential = credential.cloned();
        let data = self.base.cgi(module, method, param, opts).await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取歌曲详细信息 (固定使用 Web 平台).
    pub async fn get_detail(&self, value: &str) -> Result<GetSongDetailResponse> {
        let param = if value.chars().all(|c| c.is_ascii_digit()) && !value.is_empty() {
            json!({ "song_id": value.parse::<i64>().unwrap_or(0) })
        } else {
            json!({ "song_mid": value })
        };
        let mut opts = RequestOptions::default();
        opts.platform = Some(crate::versioning::Platform::Web);
        let data = self
            .base
            .cgi(
                "music.pf_song_detail_svr",
                "get_song_detail_yqq",
                param,
                opts,
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取相似歌曲.
    pub async fn get_similar_song(&self, songid: i64) -> Result<GetSimilarSongResponse> {
        let data = self
            .base
            .cgi(
                "music.recommend.TrackRelationServer",
                "GetSimilarSongs",
                json!({ "songid": songid }),
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取歌曲标签.
    pub async fn get_labels(&self, songid: i64) -> Result<GetSongLabelsResponse> {
        let data = self
            .base
            .cgi(
                "music.recommend.TrackRelationServer",
                "GetSongLabels",
                json!({ "songid": songid }),
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取歌曲相关歌单.
    pub async fn get_related_songlist(
        &self,
        songid: i64,
        last: &[i64],
    ) -> Result<GetRelatedSonglistResponse> {
        let data = self
            .base
            .cgi(
                "music.recommend.TrackRelationServer",
                "GetRelatedPlaylist",
                json!({ "songid": songid, "vecPlaylist": last }),
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取歌曲相关 MV.
    pub async fn get_related_mv(
        &self,
        songid: i64,
        last_mvid: Option<&str>,
    ) -> Result<GetRelatedMvResponse> {
        let data = self
            .base
            .cgi(
                "MvService.MvInfoProServer",
                "GetSongRelatedMv",
                json!({ "songid": songid.to_string(), "songtype": 1, "lastmvid": last_mvid.unwrap_or("0") }),
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取歌曲其他版本.
    pub async fn get_other_version(&self, value: &str) -> Result<GetOtherVersionResponse> {
        let param = if value.chars().all(|c| c.is_ascii_digit()) && !value.is_empty() {
            json!({ "songid": value.parse::<i64>().unwrap_or(0) })
        } else {
            json!({ "songmid": value })
        };
        let data = self
            .base
            .cgi(
                "music.musichallSong.OtherVersionServer",
                "GetOtherVersionSongs",
                param,
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取歌曲制作人信息.
    pub async fn get_producer(&self, value: &str) -> Result<GetProducerResponse> {
        let param = if value.chars().all(|c| c.is_ascii_digit()) && !value.is_empty() {
            json!({ "songid": value.parse::<i64>().unwrap_or(0) })
        } else {
            json!({ "songmid": value })
        };
        let data = self
            .base
            .cgi(
                "music.sociality.KolWorksTag",
                "SongProducer",
                param,
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取歌曲相关曲谱.
    ///
    /// `ttype`: `SheetType::User`(0)/`EngineAi`(1)/`ChongChong`(2).
    pub async fn get_sheet(&self, mid: &str, ttype: SheetType) -> Result<GetSheetResponse> {
        if ttype == SheetType::ChongChong {
            let mut opts = RequestOptions::default();
            opts.override_comm = true;
            opts.sign = true;
            opts.comm = Some(json!({
                "g_tk": 5381,
                "uin": "",
                "format": "json",
                "inCharset": "utf-8",
                "outCharset": "utf-8",
                "notice": 0,
                "platform": "h5",
                "needNewCode": 1,
            }));
            let reply = self
                .base
                .cgi_reply(
                    "music.mir.SheetMusicSvr",
                    "GetChongChongSheetMusic",
                    json!({ "songMid": mid, "begin": 0, "end": 100, "scoreType": -1, "ttype": 1 }),
                    opts,
                )
                .await?;
            let data = reply.require_success_allowing(&[10007])?;
            return Ok(serde_json::from_value(data)?);
        }
        let score_type = if ttype == SheetType::EngineAi {
            -473
        } else {
            -1
        };
        let mut opts = RequestOptions::default();
        opts.override_comm = true;
        opts.comm = Some(json!({
            "g_tk": 5381,
            "uin": "",
            "format": "json",
            "inCharset": "utf-8",
            "outCharset": "utf-8",
            "notice": 0,
            "needNewCode": 1,
        }));
        let reply = self
            .base
            .cgi_reply(
                "music.mir.SheetMusicSvr",
                "GetMoreSheetMusic",
                json!({ "songMid": mid, "begin": 0, "end": 100, "scoreType": score_type, "ttype": ttype as i64 }),
                opts,
            )
            .await?;
        let data = reply.require_success_allowing(&[10007])?;
        Ok(serde_json::from_value(data)?)
    }

    /// 检查歌曲是否有曲谱.
    pub async fn has_sheet(&self, mid: &str) -> Result<HasSheetMusicResponse> {
        let mut opts = RequestOptions::default();
        opts.override_comm = true;
        opts.comm = Some(json!({
            "g_tk": 5381,
            "uin": "",
            "format": "json",
            "inCharset": "utf-8",
            "outCharset": "utf-8",
            "notice": 0,
            "needNewCode": 1,
        }));
        let data = self
            .base
            .cgi(
                "music.mir.SheetMusicSvr",
                "HasSheetMusic",
                json!({ "songMid": mid }),
                opts,
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取歌曲收藏数量原始数据.
    pub async fn get_fav_num(&self, song_ids: &[i64]) -> Result<GetFavNumResponse> {
        let data = self
            .base
            .cgi(
                "music.musicasset.SongFavRead",
                "GetSongFansNumberById",
                json!({ "v_songId": song_ids }),
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    // ------------------------------------------------------------------
    // VIP 音质辅助 (配合 VIP 账号使用, 非破解)
    // ------------------------------------------------------------------

    /// 列出歌曲可用音质 (按优先级从高到低, 依据 `file` 尺寸字段).
    pub fn available_qualities(&self, song: &Song) -> Vec<SongQuality> {
        let mut list: Vec<SongQuality> = SongQuality::ALL
            .iter()
            .copied()
            .filter(|q| q.size(&song.file) > 0)
            .collect();
        list.sort_by_key(|q| q.priority());
        list
    }

    /// 获取歌曲最高可用音质的播放链接.
    ///
    /// - `credential`: VIP 账号凭证 (高音质通常需要绿钻).
    /// - `allow_encrypted`: 是否允许加密音质 (走 `CgiGetEVkey`, 返回 `.mflac` 等,
    ///   需配合 `qmc` 解密播放).
    ///
    /// 返回 `(音质档位, 播放链接响应)`; 歌曲无可用音质时返回错误.
    pub async fn get_best_song_url(
        &self,
        song: &Song,
        credential: Option<&Credential>,
        allow_encrypted: bool,
    ) -> Result<(SongQuality, GetSongUrlsResponse)> {
        let available = self.available_qualities(song);
        let quality = *available
            .first()
            .ok_or_else(|| QmError::ApiData("歌曲无可用音质".into()))?;
        let file_type = quality.file_type(allow_encrypted);
        let urls = self
            .get_song_urls(
                &[SongFileInfo::new(&song.mid)
                    .with_song_type(song.r#type)
                    .with_media_mid(&song.file.media_mid)
                    .with_type_ref(file_type)],
                file_type,
                credential,
            )
            .await?;
        Ok((quality, urls))
    }

    /// 获取歌曲最高可用音质的来源描述 (`MediaSource`, 播放器可直接消费).
    ///
    /// 等价于 `media::MediaSource::best(self, song, credential, allow_encrypted)`.
    pub async fn media_source(
        &self,
        song: &Song,
        credential: Option<&Credential>,
        allow_encrypted: bool,
    ) -> Result<crate::media::MediaSource> {
        crate::media::MediaSource::best(self, song, credential, allow_encrypted).await
    }

    /// 获取歌曲**实际可播放**的最高音质来源描述 (只 resolve, 不下载).
    ///
    /// 按可用音质从高到低降级, 返回第一个 `playable()` 的来源; 这是
    /// YAQMC Provider 获取播放来源的推荐入口.
    pub async fn best_playable(
        &self,
        song: &Song,
        credential: Option<&Credential>,
        allow_encrypted: bool,
    ) -> Result<crate::media::MediaSource> {
        crate::media::best_playable(self, song, credential, allow_encrypted).await
    }

    /// 下载并解密指定音质的音频 (媒体层助手, 见 `media::download_quality`).
    pub async fn download_quality(
        &self,
        song: &Song,
        quality: SongQuality,
        credential: Option<&Credential>,
    ) -> Result<(Vec<u8>, String)> {
        crate::media::download_quality(self, song, quality, credential).await
    }

    /// 下载并解密歌曲最高可用音质 (媒体层助手, 见 `media::download_best`).
    pub async fn download_best(
        &self,
        song: &Song,
        credential: Option<&Credential>,
    ) -> Result<(SongQuality, Vec<u8>, String)> {
        crate::media::download_best(self, song, credential).await
    }

    // ------------------------------------------------------------------
    // 以下接口补充自官方桌面客户端 (Electron ASAR) `common.js`.
    // ------------------------------------------------------------------

    /// ⚠️ **Raw 透传** — 检查歌曲是否已被收藏
    /// (官方桌面端 `music.musicasset.SongFavRead / IsSongFanByMid`).
    ///
    /// `param` 通常形如 `{"songmid": ["xxx"]}` 或 `{"song_id": [id]}`;
    /// 参数与响应 schema 未经 live 验证, 仅提供透传能力.
    pub async fn raw_is_song_fan_by_mid(
        &self,
        param: Value,
        credential: Option<&Credential>,
    ) -> Result<Value> {
        let mut opts = RequestOptions::default();
        opts.require_login = true;
        opts.credential = credential.cloned();
        self.base
            .cgi(
                "music.musicasset.SongFavRead",
                "IsSongFanByMid",
                param,
                opts,
            )
            .await
    }

    /// ⚠️ **Raw 透传** — 获取收藏歌曲列表
    /// (官方桌面端 `music.musicasset.SongFavRead / GetFavSonglist`).
    pub async fn raw_get_fav_songlist(
        &self,
        param: Value,
        credential: Option<&Credential>,
    ) -> Result<Value> {
        let mut opts = RequestOptions::default();
        opts.require_login = true;
        opts.credential = credential.cloned();
        self.base
            .cgi(
                "music.musicasset.SongFavRead",
                "GetFavSonglist",
                param,
                opts,
            )
            .await
    }

    /// ⚠️ **Raw 透传** — 获取播放链接
    /// (官方桌面端 `music.vkey.GetVkey / GetUrl`, 用于下载).
    ///
    /// `param` 通常形如 `{"songmid": ["xxx"], "songtype": [0]}`;
    /// 播放链接请优先使用类型化的 `get_song_urls`.
    pub async fn raw_get_url_vkey(&self, param: Value) -> Result<Value> {
        let base = json!({
            "uin": self.base.credential().str_musicid(),
            "guid": "5640789320",
            "downloadfrom": 1,
            "ctx": 1,
            "scene": 0,
            "nettype": "",
            "platform": "20",
        });
        let mut final_param = base;
        if let Value::Object(map) = param {
            for (k, v) in map {
                final_param[k] = v;
            }
        }
        self.base
            .cgi(
                "music.vkey.GetVkey",
                "GetUrl",
                final_param,
                RequestOptions::default(),
            )
            .await
    }
}
