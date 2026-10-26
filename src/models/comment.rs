//! Comment API 返回模型定义 (对应 Python 端 `models/comment.py`).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::jsonpath_model;

/// 评论业务类型.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CommentBizType {
    /// 普通歌曲.
    #[default]
    Song,
    /// 专辑.
    Album,
    /// MV.
    Mv,
    /// 歌单.
    Songlist,
    /// 歌手主页.
    Singer,
    /// 视频.
    Video,
    /// 播客 (电台) 节目.
    Audio,
    /// 播客 (电台) 专辑.
    AudioAlbum,
}

impl CommentBizType {
    pub fn value(&self) -> i64 {
        match self {
            CommentBizType::Song => 1,
            CommentBizType::Album => 2,
            CommentBizType::Mv => 4,
            CommentBizType::Songlist => 3,
            CommentBizType::Singer => 4,
            CommentBizType::Video => 5,
            CommentBizType::Audio => 6,
            CommentBizType::AudioAlbum => 7,
        }
    }
}

/// 图标文本信息.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct IconTextInfo {
    pub txt: String,
    pub unique_id: String,
    pub r#type: i64,
    pub cmid: String,
    pub is_dynamic: bool,
}

jsonpath_model!(CommentCountResponse {
    biz_type: "$.response.biz_type" => i64,
    biz_id: "$.response.biz_id" => String,
    biz_sub_type: "$.response.biz_sub_type" => i64,
    count: "$.response.count" => i64,
    count_ver: "$.response.count_ver" => String,
    count_view: "$.response.count_view" => String,
    related_id: "$.response.related_id" => String,
    tip: "$.response.tip" => String,
    icon_list: "$.response.icon_list" => Vec<IconTextInfo>,
    cm_tab_type: "$.cmTabType" => i64,
});

/// 评论条目.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct CommentItem {
    #[serde(alias = "CmId")]
    pub cmid: String,
    #[serde(alias = "SeqNo")]
    pub seq_no: String,
    #[serde(alias = "Nick")]
    pub nick: String,
    #[serde(alias = "Avatar")]
    pub avatar: String,
    #[serde(alias = "EncryptUin")]
    pub encrypt_uin: String,
    #[serde(alias = "Content")]
    pub content: String,
    #[serde(alias = "PubTime")]
    pub pub_time: i64,
    #[serde(alias = "PraiseNum")]
    pub praise_num: i64,
    #[serde(alias = "ReplyCnt")]
    pub reply_cnt: i64,
    #[serde(alias = "IsPraised")]
    pub is_praised: i64,
    #[serde(alias = "IsSelf")]
    pub is_self: i64,
    #[serde(alias = "State")]
    pub state: i64,
    #[serde(alias = "HotScore")]
    pub hot_score: String,
    #[serde(alias = "RecScore")]
    pub rec_score: String,
    #[serde(alias = "SongId")]
    pub song_id: i64,
    #[serde(alias = "SongName")]
    pub song_name: String,
    #[serde(alias = "SingerNames")]
    pub singer_names: String,
    #[serde(alias = "SongTsElems", default)]
    pub song_ts_elems: Vec<Value>,
    #[serde(alias = "HashTagList", default)]
    pub hash_tag_list: Vec<Value>,
    #[serde(alias = "LittleTails", default)]
    pub little_tails: Vec<Value>,
    #[serde(alias = "IconList", default)]
    pub icon_list: Vec<Value>,
    #[serde(alias = "VipUI", default)]
    pub vip_ui: Value,
    #[serde(alias = "SubComments", default)]
    pub sub_comments: Vec<Value>,
    #[serde(alias = "RepliedComments", default)]
    pub replied_comments: Vec<CommentReplyTarget>,
}

/// A quoted parent comment included in a reply row.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct CommentReplyTarget {
    #[serde(alias = "Nick")]
    pub nick: String,
    #[serde(alias = "Content")]
    pub content: String,
}

impl CommentItem {
    /// QQ comment publication time is in seconds, independent of player units.
    pub fn timestamp_ms(&self) -> Option<u64> {
        u64::try_from(self.pub_time)
            .ok()
            .filter(|time| *time > 0)
            .and_then(|time| time.checked_mul(1000))
    }

    pub fn avatar_url(&self) -> Option<String> {
        crate::artwork::normalize_url(&self.avatar)
    }
}

/// Top-level song comment ordering. Reply threads follow service thread order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SongCommentSort {
    Hot,
    New,
}

/// One-page read request. `limit` is the requested page size, not a response cap.
/// Pages are one-based; sequence numbers stay opaque.
#[derive(Debug, Clone)]
pub struct SongCommentsRequest {
    pub song_id: i64,
    pub sort: SongCommentSort,
    pub page: u32,
    pub limit: u32,
    pub last_seq: String,
    pub parent_id: Option<String>,
}

// Keep the original YAQMC cursor encoding readable during the API migration.
// This is library-owned state; consumers must treat its JSON as opaque.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SongCommentsCursor {
    version: u8,
    song: i64,
    sort: SongCommentSort,
    parent: Option<String>,
    limit: u32,
    page: u32,
    seq: String,
}

impl SongCommentsRequest {
    /// Start a public comment or reply query. Validation occurs before fetching.
    pub fn new(song_id: i64, sort: SongCommentSort, limit: u32, parent_id: Option<String>) -> Self {
        Self {
            song_id,
            sort,
            page: 1,
            limit,
            last_seq: String::new(),
            parent_id,
        }
    }

    /// Resume with a library-produced cursor scoped to this song/sort/thread/size.
    pub fn with_cursor(mut self, cursor: Option<&str>) -> crate::Result<Self> {
        if let Some(value) = cursor {
            if value.len() > 1024 {
                return Err(invalid_comment_request());
            }
            let cursor: SongCommentsCursor =
                serde_json::from_str(value).map_err(|_| invalid_comment_request())?;
            if cursor.version != 1
                || cursor.song != self.song_id
                || cursor.sort != self.sort
                || cursor.parent != self.parent_id
                || cursor.limit != self.limit
            {
                return Err(invalid_comment_request());
            }
            self.page = cursor
                .page
                .checked_add(1)
                .ok_or_else(invalid_comment_request)?;
            self.last_seq = cursor.seq;
        }
        self.validate()?;
        Ok(self)
    }

    pub(crate) fn validate(&self) -> crate::Result<()> {
        if self.song_id <= 0
            || !(1..=50).contains(&self.limit)
            || self.page == 0
            || self.last_seq.len() > 256
            || self
                .parent_id
                .as_ref()
                .is_some_and(|id| id.trim().is_empty() || id.len() > 256)
        {
            return Err(invalid_comment_request());
        }
        Ok(())
    }
}

fn invalid_comment_request() -> crate::QmError {
    crate::QmError::ValueError("invalid song comment request".into())
}

/// Validated wire response, independent of any player's rendering DTOs.
#[derive(Debug, Clone)]
pub struct SongCommentsPage {
    pub comments: Vec<CommentItem>,
    pub has_more: bool,
    pub total: Option<u64>,
    pub total_comments: Option<u64>,
}

impl SongCommentsPage {
    /// Preserve every returned row when continuing; never advance from a UI slice.
    pub fn next_cursor(&self, request: &SongCommentsRequest) -> crate::Result<Option<String>> {
        request.validate()?;
        let Some((page, seq)) = self.next_position(request.page, &request.last_seq) else {
            return Ok(None);
        };
        Ok(Some(serde_json::to_string(&SongCommentsCursor {
            version: 1,
            song: request.song_id,
            sort: request.sort,
            parent: request.parent_id.clone(),
            limit: request.limit,
            page: page - 1,
            seq,
        })?))
    }

    pub(crate) fn next_position(&self, page: u32, last_seq: &str) -> Option<(u32, String)> {
        let seq = &self.comments.last()?.seq_no;
        if !self.has_more || seq.is_empty() || seq == last_seq {
            return None;
        }
        Some((page.checked_add(1)?, seq.clone()))
    }

    pub(crate) fn parse(data: Value) -> crate::Result<Self> {
        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct Page {
            comment_list: List,
            #[serde(default)]
            total_cm_num: Option<u64>,
            #[serde(default)]
            sub_code: i64,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct List {
            comments: Vec<CommentItem>,
            has_more: i64,
            total: Option<u64>,
        }
        let page: Page = serde_json::from_value(data)?;
        if page.sub_code != 0 {
            return Err(crate::reply::map_cgi_code(page.sub_code, &Value::Null));
        }
        let list = page.comment_list;
        let mut ids = std::collections::HashSet::new();
        // The service may append extra rows (observed: 21 for PageSize=20).
        // Preserve them and their final sequence number for lossless pagination.
        if list.has_more < 0
            || list.comments.iter().any(|row| {
                row.cmid.trim().is_empty()
                    || row.cmid.len() > 256
                    || row.seq_no.len() > 256
                    || !ids.insert(&row.cmid)
            })
        {
            return Err(crate::QmError::ApiData(
                "invalid comment page or identity".into(),
            ));
        }
        Ok(Self {
            comments: list.comments,
            has_more: list.has_more > 0,
            total: list.total,
            total_comments: page.total_cm_num,
        })
    }
}

jsonpath_model!(CommentListResponse {
    comments: "$.CommentList.Comments[*]" => Vec<CommentItem>,
    comment_ids: "$.CommentList.CommentIds[*]" => Vec<String>,
    has_more: "$.CommentList.HasMore" => i64,
    next_offset: "$.CommentList.NextOffset" => i64,
    total: "$.CommentList.Total" => i64,
    total_cm_num: "$.TotalCmNum" => i64,
    comment_tip: "$.CommentTip" => String,
    has_ts_cm: "$.HasTsCm" => i64,
    share_cnt: "$.ShareCnt" => i64,
    msg: "$.Msg" => String,
    sub_code: "$.SubCode" => i64,
});

/// 歌曲时刻评论条目.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct MomentCommentItem {
    #[serde(alias = "CmId")]
    pub cmid: String,
    #[serde(alias = "SeqNo")]
    pub seq_no: String,
    #[serde(alias = "Content")]
    pub content: String,
    #[serde(alias = "EncryptUin")]
    pub encrypt_uin: String,
    #[serde(alias = "PubTime")]
    pub pub_time: i64,
}

/// 歌曲时刻评论响应.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct MomentCommentResponse {
    #[serde(alias = "Comments")]
    pub comments: Vec<MomentCommentItem>,
    pub has_more: i64,
    #[serde(alias = "NextPos")]
    pub next_pos: String,
}

/// 添加评论响应.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AddCommentResponse {
    pub comment_id: String,
}
