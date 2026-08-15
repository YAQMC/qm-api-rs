//! Comment API 返回模型定义 (对应 Python 端 `models/comment.py`).

use serde::Deserialize;
use serde_json::Value;

use crate::jsonpath_model;

/// 评论业务类型.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentBizType {
    /// 普通歌曲.
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
            CommentBizType::Song => 0,
            CommentBizType::Album => 1,
            CommentBizType::Mv => 2,
            CommentBizType::Songlist => 3,
            CommentBizType::Singer => 4,
            CommentBizType::Video => 5,
            CommentBizType::Audio => 6,
            CommentBizType::AudioAlbum => 7,
        }
    }
}

impl Default for CommentBizType {
    fn default() -> Self {
        CommentBizType::Song
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
