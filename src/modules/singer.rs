//! 歌手相关 API (对应 Python 端 `modules/singer.py`).

use serde_json::json;

use super::ApiModule;
use crate::context::RequestOptions;
use crate::error::Result;
use crate::models::singer::*;
use crate::versioning::Platform;

/// 地区类型.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AreaType {
    All = -100,
    China = 200,
    Taiwan = 2,
    America = 5,
    Japan = 4,
    Korea = 3,
}

/// 风格类型.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenreType {
    All = -100,
    Pop = 7,
    Rap = 3,
    ChineseStyle = 19,
    Rock = 4,
    Electronic = 2,
    Folk = 8,
    RAndB = 11,
    Ethnic = 37,
    LightMusic = 93,
    Jazz = 14,
    Classical = 33,
    Country = 13,
    Blues = 10,
}

/// 性别类型.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SexType {
    All = -100,
    Male = 0,
    Female = 1,
    Group = 2,
}

/// 首字母索引.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexType {
    A = 1,
    B = 2,
    C = 3,
    D = 4,
    E = 5,
    F = 6,
    G = 7,
    H = 8,
    I = 9,
    J = 10,
    K = 11,
    L = 12,
    M = 13,
    N = 14,
    O = 15,
    P = 16,
    Q = 17,
    R = 18,
    S = 19,
    T = 20,
    U = 21,
    V = 22,
    W = 23,
    X = 24,
    Y = 25,
    Z = 26,
    All = -100,
    Hash = 27,
}

/// 歌手主页 Tab 类型.
#[derive(Debug, Clone, Copy)]
pub enum TabType {
    Wiki,
    Album,
    Composer,
    Lyricist,
    Producer,
    Arranger,
    Musician,
    Song,
    Video,
}

impl TabType {
    pub fn tab_id(&self) -> &'static str {
        match self {
            TabType::Wiki => "wiki",
            TabType::Album => "album",
            TabType::Composer => "song_composing",
            TabType::Lyricist => "song_lyric",
            TabType::Producer => "producer",
            TabType::Arranger => "arranger",
            TabType::Musician => "musician",
            TabType::Song => "song_sing",
            TabType::Video => "video",
        }
    }
    pub fn tab_name(&self) -> &'static str {
        match self {
            TabType::Wiki => "IntroductionTab",
            TabType::Album => "AlbumTab",
            _ => "SongTab",
        }
    }
}

/// 歌手相关 API.
#[derive(Clone, Debug)]
pub struct SingerApi {
    pub(crate) base: ApiModule,
}

impl SingerApi {
    pub(crate) fn new(context: std::sync::Arc<crate::context::ApiContext>) -> Self {
        SingerApi {
            base: ApiModule::new(context),
        }
    }

    /// 获取歌手列表原始数据.
    pub async fn get_singer_list(
        &self,
        area: i64,
        sex: i64,
        genre: i64,
    ) -> Result<SingerTypeListResponse> {
        let data = self
            .base
            .cgi(
                "music.musichallSinger.SingerList",
                "GetSingerList",
                json!({ "hastag": 0, "area": area, "sex": sex, "genre": genre }),
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取按索引分页的歌手列表原始数据.
    pub async fn get_singer_list_index(
        &self,
        area: i64,
        sex: i64,
        genre: i64,
        index: i64,
        page: i64,
        num: i64,
    ) -> Result<SingerIndexPageResponse> {
        let data = self
            .base
            .cgi(
                "music.musichallSinger.SingerList",
                "GetSingerListIndex",
                json!({
                    "area": area,
                    "sex": sex,
                    "genre": genre,
                    "index": index,
                    "sin": (page - 1) * num,
                    "cur_page": page,
                }),
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取歌手主页基本信息 (固定使用 Android 平台).
    pub async fn get_info(&self, mid: &str) -> Result<HomepageHeaderResponse> {
        let mut opts = RequestOptions::default();
        opts.platform = Some(Platform::Android);
        let data = self
            .base
            .cgi(
                "music.UnifiedHomepage.UnifiedHomepageSrv",
                "GetHomepageHeader",
                json!({ "SingerMid": mid }),
                opts,
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取歌手主页特定 Tab 的详情原始数据.
    pub async fn get_tab_detail(
        &self,
        mid: &str,
        tab_type: TabType,
        page: i64,
        num: i64,
    ) -> Result<HomepageTabDetailResponse> {
        let data = self
            .base
            .cgi(
                "music.UnifiedHomepage.UnifiedHomepageSrv",
                "GetHomepageTabDetail",
                json!({
                    "SingerMid": mid,
                    "IsQueryTabDetail": 1,
                    "TabID": tab_type.tab_id(),
                    "PageNum": page - 1,
                    "PageSize": num,
                    "Order": 0,
                }),
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取歌手列表的描述信息.
    #[allow(clippy::too_many_arguments)]
    pub async fn get_desc(
        &self,
        mids: &[String],
        ex_singer: bool,
        wiki_singer: bool,
        group_singer: bool,
        pic: bool,
        photos: bool,
    ) -> Result<SingerDetailResponse> {
        let data = self
            .base
            .cgi(
                "music.musichallSinger.SingerInfoInter",
                "GetSingerDetail",
                json!({
                    "singer_mids": mids,
                    "group_singer": group_singer,
                    "wiki_singer": wiki_singer,
                    "ex_singer": ex_singer,
                    "pic": pic,
                    "photos": photos,
                }),
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取相似歌手列表.
    pub async fn get_similar(&self, mid: &str, number: i64) -> Result<SimilarSingerResponse> {
        let data = self
            .base
            .cgi(
                "music.SimilarSingerSvr",
                "GetSimilarSingerList",
                json!({ "singerMid": mid, "number": number }),
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取歌手的歌曲列表.
    pub async fn get_songs_list(
        &self,
        mid: &str,
        num: i64,
        page: i64,
    ) -> Result<SingerSongListResponse> {
        let data = self
            .base
            .cgi(
                "musichall.song_list_server",
                "GetSingerSongList",
                json!({ "singerMid": mid, "order": 1, "number": num, "begin": (page - 1) * num }),
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取歌手的专辑列表.
    pub async fn get_album_list(
        &self,
        mid: &str,
        num: i64,
        page: i64,
    ) -> Result<SingerAlbumListResponse> {
        let data = self
            .base
            .cgi(
                "music.musichallAlbum.AlbumListServer",
                "GetAlbumList",
                json!({ "singerMid": mid, "order": 1, "number": num, "begin": (page - 1) * num }),
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取歌手 MV 列表数据.
    pub async fn get_mv_list(
        &self,
        mid: &str,
        num: i64,
        page: i64,
    ) -> Result<SingerMvListResponse> {
        let data = self
            .base
            .cgi(
                "MvService.MvInfoProServer",
                "GetSingerMvList",
                json!({ "singermid": mid, "order": 1, "count": num, "start": (page - 1) * num }),
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }
}
