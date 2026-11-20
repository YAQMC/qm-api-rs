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

    /// 获取 MV 播放链接，默认请求浏览器兼容性更稳定的 H.264 编码.
    pub async fn get_mv_urls(&self, vids: &[String]) -> Result<GetMvUrlsResponse> {
        self.get_mv_urls_with_format(vids, 264).await
    }

    /// 获取指定视频编码格式的 MV 播放链接.
    ///
    /// `format` 使用上游视频编码编号：`264` 为 H.264，`265` 为 HEVC。
    pub async fn get_mv_urls_with_format(
        &self,
        vids: &[String],
        format: i64,
    ) -> Result<GetMvUrlsResponse> {
        if !matches!(format, 264 | 265) {
            return Err(crate::QmError::ValueError(
                "MV format must be 264 or 265".into(),
            ));
        }
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
                    "format": format,
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
    pub async fn get_mv_list(
        &self,
        area: i64,
        version: i64,
        order: i64,
        num: i64,
        page: i64,
    ) -> Result<GetMvListResponse> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ApiTransport, Client, HttpBody, Platform, TransportRequest, TransportResponse};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Fixture {
        requests: Mutex<Vec<serde_json::Value>>,
    }

    #[async_trait::async_trait]
    impl ApiTransport for Fixture {
        async fn execute(&self, request: TransportRequest) -> Result<TransportResponse> {
            let HttpBody::Json(body) = request.body else {
                panic!("expected JSON CGI request");
            };
            self.requests.lock().unwrap().push(body);
            Ok(TransportResponse {
                status: 200,
                final_url: request.url,
                headers: Vec::new(),
                body: br#"{"code":0,"req_0":{"code":0,"data":{}}}"#.to_vec(),
            })
        }
    }

    #[tokio::test]
    async fn default_mv_urls_request_h264_for_webview_compatibility() {
        let fixture = Arc::new(Fixture::default());
        let client = Client::new_with_transport(None, Some(Platform::Web), fixture.clone());
        client
            .mv
            .get_mv_urls(&["fixture-vid".to_owned()])
            .await
            .unwrap();

        let requests = fixture.requests.lock().unwrap();
        let request = &requests[0];
        assert_eq!(request["req_0"]["module"], "music.stream.MvUrlProxy");
        assert_eq!(request["req_0"]["method"], "GetMvUrls");
        assert_eq!(request["req_0"]["param"]["format"], 264);
    }

    #[tokio::test]
    async fn mv_url_format_is_bounded() {
        let fixture = Arc::new(Fixture::default());
        let client = Client::new_with_transport(None, Some(Platform::Web), fixture);
        let error = client
            .mv
            .get_mv_urls_with_format(&["fixture-vid".to_owned()], 1)
            .await
            .unwrap_err();
        assert!(matches!(error, crate::QmError::ValueError(_)));
    }
}
