//! MV 相关 API (对应 Python 端 `modules/mv.py`).

use serde_json::json;

use super::ApiModule;
use crate::context::RequestOptions;
use crate::error::Result;
use crate::models::mv::*;
use crate::utils::get_guid;

/// MV 相关 API.
#[derive(Clone, Debug)]
pub struct MvApi {
    pub(crate) base: ApiModule,
}

impl MvApi {
    pub(crate) fn new(context: std::sync::Arc<crate::context::ApiContext>) -> Self {
        MvApi {
            base: ApiModule::new(context),
        }
    }

    /// 获取 MV 详细信息.
    pub async fn get_detail(&self, vids: &[String]) -> Result<GetMvDetailResponse> {
        let data = self
            .base
            .cgi(
                "video.VideoDataServer",
                "get_video_info_batch",
                json!({
                    "vidlist": vids,
                    "required": [
                        "vid", "type", "sid", "cover_pic", "duration", "singers", "video_switch",
                        "msg", "name", "desc", "playcnt", "pubdate", "isfav", "gmid",
                        "uploader_headurl", "uploader_nick", "uploader_encuin", "uploader_uin",
                        "uploader_hasfollow", "uploader_follower_num", "related_songs",
                    ],
                }),
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取 MV 播放链接.
    pub async fn get_mv_urls(&self, vids: &[String]) -> Result<GetMvUrlsResponse> {
        let data = self
            .base
            .cgi(
                "music.stream.MvUrlProxy",
                "GetMvUrls",
                json!({
                    "vids": vids,
                    "request_type": 10003,
                    "guid": get_guid(),
                    "videoformat": 1,
                    "format": 265,
                    "dolby": 1,
                    "use_new_domain": 1,
                    "use_ipv6": 1,
                }),
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取 MV 分类列表.
    pub async fn get_mv_list(&self, area: i64, version: i64, order: i64, num: i64, page: i64) -> Result<GetMvListResponse> {
        let data = self
            .base
            .cgi(
                "MvService.MvInfoProServer",
                "GetAllocMvInfo",
                json!({ "area": area, "version": version, "order": order, "start": num * (page - 1), "size": num }),
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }
}
