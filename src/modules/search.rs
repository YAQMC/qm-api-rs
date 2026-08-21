//! 搜索相关 API (对应 Python 端 `modules/search.py`).

use serde_json::{json, Value};

use super::ApiModule;
use crate::context::RequestOptions;
use crate::error::Result;
use crate::models::search::*;
use crate::models::Song;
use crate::utils::get_search_id;
use crate::versioning::Platform;

/// 搜索类型.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchType {
    Song,
    Singer,
    Album,
    Songlist,
    Mv,
    Lyric,
    User,
    Ringtone,
    AudioAlbum,
    Audio,
}

impl SearchType {
    pub fn value(&self) -> i64 {
        match self {
            SearchType::Song => 0,
            SearchType::Singer => 1,
            SearchType::Album => 2,
            SearchType::Songlist => 3,
            SearchType::Mv => 4,
            SearchType::Lyric => 7,
            SearchType::User => 8,
            SearchType::Ringtone => 10,
            SearchType::AudioAlbum => 15,
            SearchType::Audio => 18,
        }
    }
}

/// 搜索相关 API.
#[derive(Clone, Debug)]
pub struct SearchApi {
    pub(crate) base: ApiModule,
}

impl SearchApi {
    pub(crate) fn new(context: std::sync::Arc<crate::context::ApiContext>) -> Self {
        SearchApi {
            base: ApiModule::new(context),
        }
    }

    /// 获取热搜词列表.
    pub async fn get_hotkey(&self) -> Result<HotkeyResponse> {
        let data = self
            .base
            .cgi(
                "music.musicsearch.HotkeyService",
                "GetHotkeyForQQMusicMobile",
                json!({ "search_id": get_search_id() }),
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 搜索词补全建议.
    pub async fn complete(&self, keyword: &str) -> Result<CompleteResponse> {
        let data = self
            .base
            .cgi(
                "music.smartboxCgi.SmartBoxCgi",
                "GetSmartBoxResult",
                json!({
                    "search_id": get_search_id(),
                    "query": keyword,
                    "num_per_page": 0,
                    "page_idx": 0,
                }),
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 快速搜索.
    pub async fn quick_search(&self, keyword: &str) -> Result<QuickSearchResponse> {
        let opts = crate::client::HttpOptions {
            params: vec![("key".to_string(), keyword.to_string())],
            ..Default::default()
        };
        let text = self
            .base
            .context
            .request_http(
                crate::HttpMethod::Get,
                "https://c.y.qq.com/splcloud/fcgi-bin/smartbox_new.fcg",
                &opts,
            )
            .await?;
        Ok(serde_json::from_str(&text)?)
    }

    /// 综合搜索.
    #[allow(clippy::too_many_arguments)]
    pub async fn general_search(
        &self,
        keyword: &str,
        page: i64,
        num: i64,
        searchid: Option<&str>,
        page_start: Option<Value>,
        highlight: bool,
    ) -> Result<GeneralSearchResponse> {
        let mut param = json!({
            "searchid": searchid.map(|s| s.to_string()).unwrap_or_else(get_search_id),
            "search_type": 100,
            "page_num": num,
            "query": keyword,
            "page_id": page,
            "highlight": highlight,
            "grp": true,
        });
        if let Some(ps) = page_start {
            param["page_start"] = ps;
        }
        let data = self
            .base
            .cgi(
                "music.adaptor.SearchAdaptor",
                "do_search_v2",
                param,
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 类型搜索 (固定使用 Android 平台).
    ///
    /// 返回原始响应; 可通过 `resp.song/singer/album/...` 字段获取对应分类结果.
    #[allow(clippy::too_many_arguments)]
    pub async fn search_by_type(
        &self,
        keyword: &str,
        search_type: SearchType,
        num: i64,
        page: i64,
        selectors: &[SearchSelector],
        searchid: Option<&str>,
        highlight: bool,
    ) -> Result<SearchByTypeResponse> {
        let selector_map: Value = selectors
            .iter()
            .map(|s| (s.r#type.to_string(), Value::from(s.id)))
            .collect();
        let vec_selectors: Vec<Value> = selectors
            .iter()
            .map(|s| {
                json!({
                    "type": s.r#type,
                    "name": s.name,
                    "id": s.id,
                })
            })
            .collect();
        let param = json!({
            "searchid": searchid.map(|s| s.to_string()).unwrap_or_else(get_search_id),
            "query": keyword,
            "search_type": search_type.value(),
            "num_per_page": num,
            "page_num": page,
            "highlight": highlight,
            "grp": true,
            "selectors": selector_map,
            "vec_selectors": vec_selectors,
        });
        let mut opts = RequestOptions::default();
        opts.platform = Some(Platform::Android);
        let data = self
            .base
            .cgi(
                "music.search.SearchCgiService",
                "DoSearchForQQMusicMobile",
                param,
                opts,
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 类型搜索并提取当前分类下的条目.
    pub async fn search_songs(&self, keyword: &str, num: i64, page: i64) -> Result<Vec<Song>> {
        let resp = self
            .search_by_type(keyword, SearchType::Song, num, page, &[], None, true)
            .await?;
        Ok(resp.song.into_iter().map(|s| s.base).collect())
    }
}
