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

/// 歌手条目排序方式.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderType {
    Latest = 0,
    Hot = 1,
}

impl OrderType {
    pub fn value(self) -> i64 {
        self as i64
    }
}

/// 歌手专辑分类筛选.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlbumFilterType {
    Studio = 0,
    Live = 1,
    Ep = 11,
}

impl AlbumFilterType {
    pub fn value(self) -> i64 {
        self as i64
    }
}

/// 歌手主页专辑 Tab 的筛选选项.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AlbumTabOptions {
    pub filter_type: Option<AlbumFilterType>,
    pub include_filter_list: bool,
}

/// 歌手主页视频 Tab 的筛选选项.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VideoTabOptions {
    pub tag_id: Option<i64>,
    pub include_tag_list: bool,
}

/// 歌手主页 Tab 的类型化扩展参数.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SingerTabExtension {
    Album(AlbumTabOptions),
    Video(VideoTabOptions),
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

    /// 获取歌手主页特定 Tab 的详情，默认按最新排序且不附加筛选.
    pub async fn get_tab_detail(
        &self,
        mid: &str,
        tab_type: TabType,
        page: i64,
        num: i64,
    ) -> Result<HomepageTabDetailResponse> {
        self.get_tab_detail_with_options(mid, tab_type, page, num, OrderType::Latest, None)
            .await
    }

    /// 获取歌手主页特定 Tab 的详情，并支持排序和筛选扩展参数.
    #[allow(clippy::too_many_arguments)]
    pub async fn get_tab_detail_with_options(
        &self,
        mid: &str,
        tab_type: TabType,
        page: i64,
        num: i64,
        order: OrderType,
        extension: Option<SingerTabExtension>,
    ) -> Result<HomepageTabDetailResponse> {
        let page_num = page
            .checked_sub(1)
            .filter(|_| page > 0 && num > 0 && !mid.trim().is_empty())
            .ok_or_else(|| crate::QmError::ValueError("invalid singer tab pagination".into()))?;
        let mut param = json!({
            "SingerMid": mid,
            "IsQueryTabDetail": 1,
            "TabID": tab_type.tab_id(),
            "PageNum": page_num,
            "PageSize": num,
            "Order": order.value(),
        });
        match (tab_type, extension) {
            (TabType::Album, Some(SingerTabExtension::Album(options))) => {
                param["AlbumExtension"] = json!({
                    "IsNeedFilterType": if options.include_filter_list { 1 } else { 0 },
                    "FilterType": options.filter_type.map(AlbumFilterType::value).unwrap_or_default(),
                });
            }
            (TabType::Video, Some(SingerTabExtension::Video(options))) => {
                if options.tag_id.is_some_and(|tag_id| tag_id < 0) {
                    return Err(crate::QmError::ValueError(
                        "singer video tab tag id must not be negative".into(),
                    ));
                }
                param["VideoExtension"] = json!({
                    "TagID": options.tag_id.unwrap_or_default(),
                    "IsNeedTagList": if options.include_tag_list { 1 } else { 0 },
                });
            }
            (_, Some(_)) => {
                return Err(crate::QmError::ValueError(
                    "singer tab extension does not match tab type".into(),
                ));
            }
            (_, None) => {}
        }
        let data = self
            .base
            .cgi(
                "music.UnifiedHomepage.UnifiedHomepageSrv",
                "GetHomepageTabDetail",
                param,
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

    /// 获取歌手的歌曲列表，默认按热度排序.
    pub async fn get_songs_list(
        &self,
        mid: &str,
        num: i64,
        page: i64,
    ) -> Result<SingerSongListResponse> {
        self.get_songs_list_with_order(mid, num, page, OrderType::Hot)
            .await
    }

    /// 获取歌手的歌曲列表并指定排序.
    pub async fn get_songs_list_with_order(
        &self,
        mid: &str,
        num: i64,
        page: i64,
        order: OrderType,
    ) -> Result<SingerSongListResponse> {
        let begin = singer_offset(page, num, mid, "singer song")?;
        let data = self
            .base
            .cgi(
                "musichall.song_list_server",
                "GetSingerSongList",
                json!({"singerMid": mid, "order": order.value(), "number": num, "begin": begin}),
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取歌手的专辑列表，默认按热度排序.
    pub async fn get_album_list(
        &self,
        mid: &str,
        num: i64,
        page: i64,
    ) -> Result<SingerAlbumListResponse> {
        self.get_album_list_with_filter(mid, num, page, OrderType::Hot, None)
            .await
    }

    /// 获取歌手的专辑列表，并支持排序和专辑类型筛选.
    pub async fn get_album_list_with_filter(
        &self,
        mid: &str,
        num: i64,
        page: i64,
        order: OrderType,
        filter_type: Option<AlbumFilterType>,
    ) -> Result<SingerAlbumListResponse> {
        let begin = singer_offset(page, num, mid, "singer album")?;
        let mut param = json!({
            "singerMid": mid,
            "order": order.value(),
            "number": num,
            "begin": begin,
        });
        if let Some(filter_type) = filter_type {
            param["types"] = json!([filter_type.value()]);
        }
        let data = self
            .base
            .cgi(
                "music.musichallAlbum.AlbumListServer",
                "GetAlbumList",
                param,
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取歌手 MV 分类标签.
    pub async fn get_mv_tag(&self, mid: &str) -> Result<SingerMvTagResponse> {
        if mid.trim().is_empty() {
            return Err(crate::QmError::ValueError(
                "singer mid must not be empty".into(),
            ));
        }
        let data = self
            .base
            .cgi(
                "MvService.MvInfoProServer",
                "GetSingerMvTag",
                json!({ "singermid": mid }),
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取歌手 MV 列表，默认按热度排序且不筛选标签.
    pub async fn get_mv_list(
        &self,
        mid: &str,
        num: i64,
        page: i64,
    ) -> Result<SingerMvListResponse> {
        self.get_mv_list_with_filter(mid, num, page, OrderType::Hot, None)
            .await
    }

    /// 获取歌手 MV 列表，并支持排序及分类标签筛选.
    pub async fn get_mv_list_with_filter(
        &self,
        mid: &str,
        num: i64,
        page: i64,
        order: OrderType,
        tag_id: Option<i64>,
    ) -> Result<SingerMvListResponse> {
        let start = singer_offset(page, num, mid, "singer mv")?;
        let mut param = json!({
            "singermid": mid,
            "order": order.value(),
            "count": num,
            "start": start,
        });
        if let Some(tag_id) = tag_id {
            if tag_id < 0 {
                return Err(crate::QmError::ValueError(
                    "singer mv tag id must not be negative".into(),
                ));
            }
            param["tagid"] = json!(tag_id);
        }
        let data = self
            .base
            .cgi(
                "MvService.MvInfoProServer",
                "GetSingerMvList",
                param,
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }
}

fn singer_offset(page: i64, num: i64, mid: &str, kind: &str) -> Result<i64> {
    page.checked_sub(1)
        .filter(|_| page > 0 && num > 0 && !mid.trim().is_empty())
        .and_then(|offset| offset.checked_mul(num))
        .ok_or_else(|| crate::QmError::ValueError(format!("invalid {kind} pagination")))
}
