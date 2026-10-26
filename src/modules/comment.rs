//! 评论模块 (对应 Python 端 `modules/comment.py`).

use serde_json::{json, Value};

use super::ApiModule;
use crate::context::RequestOptions;
use crate::error::Result;
use crate::models::comment::*;
use crate::models::Credential;
use crate::{CancellationToken, Pager, QmError};

/// Per-response protection, independent of the number of pages in a song.
pub const MAX_SONG_COMMENTS_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

/// 评论 API.
#[derive(Clone, Debug)]
pub struct CommentApi {
    pub(crate) base: ApiModule,
}

#[cfg(test)]
#[path = "comment_tests.rs"]
mod tests;

impl CommentApi {
    /// Read public song comments or a reply thread without inheriting login state.
    /// `limit` requests 1–50 rows; the service may return extra rows, which are
    /// preserved. Use `song_comments_pages` to read more pages on demand.
    pub async fn get_song_comments(
        &self,
        request: &SongCommentsRequest,
    ) -> Result<SongCommentsPage> {
        self.get_song_comments_with_cancellation(request, CancellationToken::new())
            .await
    }

    /// Read one page with cancellation during network I/O and background parsing.
    pub async fn get_song_comments_with_cancellation(
        &self,
        request: &SongCommentsRequest,
        cancellation: CancellationToken,
    ) -> Result<SongCommentsPage> {
        request.validate()?;
        let mut params = json!({"BizType":CommentBizType::Song.value(), "BizSubType":2,
            "BizId":request.song_id.to_string(), "PageSize":request.limit,
            "PageNum":request.page - 1, "LastCommentSeqNo":request.last_seq, "PicEnable":1});
        let (module, method) = if let Some(parent) = &request.parent_id {
            params["RootCmId"] = json!(parent);
            params["CmId"] = json!(parent);
            (
                "music.globalComment.CommentReadServer",
                "GetReplyCommentList",
            )
        } else if request.sort == SongCommentSort::Hot {
            params["HotType"] = json!(1);
            params["WithAirborne"] = json!(0);
            ("music.globalComment.CommentRead", "GetHotCommentList")
        } else {
            params["HashTagID"] = json!("");
            params["SelfSeeEnable"] = json!(1);
            params["AudioEnable"] = json!(1);
            ("music.globalComment.CommentRead", "GetNewCommentList")
        };
        let mut options = RequestOptions::default();
        options.platform = Some(crate::Platform::Web);
        options.credential = Some(Credential::default());
        options.cancellation = cancellation.clone();
        let data = self
            .base
            .context
            .request_cgi_with_response_limit(
                module,
                method,
                params,
                &options,
                Some(MAX_SONG_COMMENTS_RESPONSE_BYTES),
            )
            .await?
            .require_success()?;
        let parse = tokio::task::spawn_blocking(move || SongCommentsPage::parse(data));
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(QmError::cancelled()),
            result = parse => result.map_err(|_| QmError::Protocol {
                stage: "comment-parse",
                message: "comment parser task failed".into(),
            })?,
        }
    }

    /// Fetch exactly one upstream page per `next().await`, with no total-row or
    /// total-page cap. Drop each consumed page instead of calling `collect()` for
    /// large threads. Empty pages, repeated sequence numbers and page overflow
    /// terminate iteration. An error terminates the pager as for other `Pager`s.
    pub fn song_comments_pages(
        &self,
        request: SongCommentsRequest,
        cancellation: CancellationToken,
    ) -> Pager<SongCommentsPage> {
        let api = self.clone();
        let initial = json!({"page": request.page, "last_seq": request.last_seq});
        Pager::new(
            initial,
            move |params| {
                let api = api.clone();
                let mut request = request.clone();
                let cancellation = cancellation.clone();
                request.page = params["page"]
                    .as_u64()
                    .and_then(|page| u32::try_from(page).ok())
                    .unwrap_or(0);
                request.last_seq = params["last_seq"].as_str().unwrap_or_default().to_owned();
                Box::pin(async move {
                    api.get_song_comments_with_cancellation(&request, cancellation)
                        .await
                })
            },
            |params, page: &SongCommentsPage| {
                let (next_page, last_seq) = page.next_position(
                    u32::try_from(params["page"].as_u64()?).ok()?,
                    params["last_seq"].as_str()?,
                )?;
                Some(json!({"page": next_page, "last_seq": last_seq}))
            },
        )
    }

    pub(crate) fn new(context: std::sync::Arc<crate::context::ApiContext>) -> Self {
        CommentApi {
            base: ApiModule::new(context),
        }
    }

    fn default_sub_type(biz_type: &CommentBizType) -> Option<i64> {
        if *biz_type == CommentBizType::Song {
            Some(2)
        } else {
            None
        }
    }

    /// 获取评论数量.
    pub async fn get_comment_count(
        &self,
        biz_id: i64,
        biz_type: CommentBizType,
        biz_sub_type: Option<i64>,
    ) -> Result<CommentCountResponse> {
        let sub_type = biz_sub_type.or_else(|| Self::default_sub_type(&biz_type));
        let mut req_data = json!({
            "biz_id": biz_id.to_string(),
            "biz_type": biz_type.value(),
        });
        if let Some(st) = sub_type {
            req_data["biz_sub_type"] = json!(st);
        }
        let data = self
            .base
            .cgi(
                "music.globalComment.CommentCountSrv",
                "GetCmCount",
                json!({ "request": req_data }),
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    fn comment_opts(biz_type: CommentBizType, biz_sub_type: Option<i64>) -> (i64, Option<i64>) {
        (biz_type.value(), biz_sub_type)
    }

    /// 获取歌曲热评.
    #[allow(clippy::too_many_arguments)]
    pub async fn get_hot_comments(
        &self,
        biz_id: i64,
        page_num: i64,
        page_size: i64,
        last_comment_seq_no: &str,
        biz_type: CommentBizType,
        biz_sub_type: Option<i64>,
    ) -> Result<CommentListResponse> {
        let (biz_type_v, biz_sub_type_v) = Self::comment_opts(biz_type, biz_sub_type);
        let mut params = json!({
            "BizType": biz_type_v,
            "BizId": biz_id.to_string(),
            "LastCommentSeqNo": last_comment_seq_no,
            "PageSize": page_size,
            "PageNum": page_num - 1,
            "HotType": 1,
            "WithAirborne": 0,
            "PicEnable": 1,
        });
        if let Some(st) = biz_sub_type_v {
            params["BizSubType"] = json!(st);
        }
        let data = self
            .base
            .cgi(
                "music.globalComment.CommentRead",
                "GetHotCommentList",
                params,
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取歌曲最新评论.
    #[allow(clippy::too_many_arguments)]
    pub async fn get_new_comments(
        &self,
        biz_id: i64,
        page_num: i64,
        page_size: i64,
        last_comment_seq_no: &str,
        biz_type: CommentBizType,
        biz_sub_type: Option<i64>,
    ) -> Result<CommentListResponse> {
        let (biz_type_v, biz_sub_type_v) = Self::comment_opts(biz_type, biz_sub_type);
        let mut params = json!({
            "PageSize": page_size,
            "PageNum": page_num - 1,
            "HashTagID": "",
            "BizType": biz_type_v,
            "PicEnable": 1,
            "LastCommentSeqNo": last_comment_seq_no,
            "SelfSeeEnable": 1,
            "BizId": biz_id.to_string(),
            "AudioEnable": 1,
        });
        if let Some(st) = biz_sub_type_v {
            params["BizSubType"] = json!(st);
        }
        let data = self
            .base
            .cgi(
                "music.globalComment.CommentRead",
                "GetNewCommentList",
                params,
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取歌曲推荐评论.
    #[allow(clippy::too_many_arguments)]
    pub async fn get_recommend_comments(
        &self,
        biz_id: i64,
        page_num: i64,
        page_size: i64,
        last_comment_seq_no: &str,
        biz_type: CommentBizType,
        biz_sub_type: Option<i64>,
    ) -> Result<CommentListResponse> {
        let (biz_type_v, biz_sub_type_v) = Self::comment_opts(biz_type, biz_sub_type);
        let mut params = json!({
            "PageSize": page_size,
            "PageNum": page_num - 1,
            "BizType": biz_type_v,
            "PicEnable": 1,
            "Flag": 1,
            "LastCommentSeqNo": last_comment_seq_no,
            "CmListUIVer": 1,
            "BizId": biz_id.to_string(),
            "AudioEnable": 1,
        });
        if let Some(st) = biz_sub_type_v {
            params["BizSubType"] = json!(st);
        }
        let data = self
            .base
            .cgi(
                "music.globalComment.CommentRead",
                "GetRecCommentList",
                params,
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取歌曲时刻评论.
    #[allow(clippy::too_many_arguments)]
    pub async fn get_moment_comments(
        &self,
        biz_id: i64,
        page_size: i64,
        last_comment_seq_no: &str,
        biz_type: CommentBizType,
        biz_sub_type: Option<i64>,
    ) -> Result<MomentCommentResponse> {
        let (biz_type_v, biz_sub_type_v) = Self::comment_opts(biz_type, biz_sub_type);
        let mut params = json!({
            "LastPos": last_comment_seq_no,
            "HashTagID": "",
            "SeekTs": -1,
            "Size": page_size,
            "BizType": biz_type_v,
            "BizId": biz_id.to_string(),
        });
        if let Some(st) = biz_sub_type_v {
            params["BizSubType"] = json!(st);
        }
        let data = self
            .base
            .cgi(
                "music.globalComment.SongTsComment",
                "GetSongTsCmList",
                params,
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 添加评论.
    #[allow(clippy::too_many_arguments)]
    pub async fn add_comment(
        &self,
        biz_id: i64,
        content: &str,
        reply_cmt_id: Option<&str>,
        biz_type: CommentBizType,
        biz_sub_type: Option<i64>,
        credential: Option<&Credential>,
    ) -> Result<AddCommentResponse> {
        let mut req_data = json!({
            "Content": content,
            "BizType": biz_type.value(),
            "BizId": biz_id.to_string(),
        });
        if let Some(rid) = reply_cmt_id {
            req_data["RepliedCmId"] = json!(rid);
        }
        if let Some(st) = biz_sub_type {
            req_data["BizSubType"] = json!(st);
        }
        let mut opts = RequestOptions::default();
        opts.require_login = true;
        opts.retry = crate::RetryClass::Write;
        opts.credential = credential.cloned();
        let data = self
            .base
            .cgi(
                "music.globalComment.CommentWriteServer",
                "AddComment",
                req_data,
                opts,
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 删除评论.
    pub async fn delete_comment(
        &self,
        cm_id: &str,
        credential: Option<&Credential>,
    ) -> Result<bool> {
        let mut opts = RequestOptions::default();
        opts.require_login = true;
        opts.retry = crate::RetryClass::Write;
        opts.credential = credential.cloned();
        let data = self
            .base
            .cgi(
                "music.globalComment.CommentWriteServer",
                "DelComment",
                json!({ "CommentId": cm_id }),
                opts,
            )
            .await?;
        Ok(data.get("SubCode").and_then(Value::as_i64).unwrap_or(-1) == 0)
    }

    // ------------------------------------------------------------------
    // 以下为桌面客户端协议互操作接口.
    // ------------------------------------------------------------------

    /// ⚠️ **Raw 透传** — 获取回复评论列表
    /// (官方桌面端 `CommentReadServer / GetReplyCommentList`).
    ///
    /// 参数与响应 schema 未经 live 验证, 仅提供透传能力; 稳定业务代码请勿依赖
    /// 其具体字段, 应自行封装为类型化 DTO.
    pub async fn raw_get_reply_comments(
        &self,
        param: Value,
        credential: Option<&Credential>,
    ) -> Result<Value> {
        let mut opts = RequestOptions::default();
        opts.credential = credential.cloned();
        self.base
            .cgi(
                "music.globalComment.CommentReadServer",
                "GetReplyCommentList",
                param,
                opts,
            )
            .await
    }

    /// ⚠️ **Raw 透传** — 更新热评状态
    /// (官方桌面端 `GlobalCommentWriteServer / UpdateHotComment`).
    pub async fn raw_update_hot_comment(
        &self,
        param: Value,
        credential: Option<&Credential>,
    ) -> Result<Value> {
        let mut opts = RequestOptions::default();
        opts.require_login = true;
        opts.retry = crate::RetryClass::Write;
        opts.credential = credential.cloned();
        self.base
            .cgi(
                "GlobalComment.GlobalCommentWriteServer",
                "UpdateHotComment",
                param,
                opts,
            )
            .await
    }
}
