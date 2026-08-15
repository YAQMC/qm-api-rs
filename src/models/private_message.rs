//! 私信模块返回模型定义 (对应 Python 端 `models/private_message.py`).

use serde::Deserialize;
use serde_json::Value;

use crate::models::de::null_as_default;

/// 私信用户信息.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PrivateMessageUser {
    pub avatar: String,
    pub encrypt_uin: String,
    pub uin: String,
    pub identity_pic: String,
    pub nick: String,
    pub identity: i64,
    pub r#type: i64,
    pub is_concern: i64,
}

/// 私信消息元数据.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PrivateMessageMetaData {
    pub title: String,
    pub content: String,
    pub pic: String,
    pub biz_id: String,
    pub biz_type: i64,
    pub url: String,
    pub width: i64,
    pub height: i64,
    #[serde(alias = "Duration")]
    pub duration: i64,
    #[serde(alias = "Size")]
    pub size: i64,
}

/// 私信消息项.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PrivateMessageInfo {
    pub id: String,
    pub meta_data: Option<PrivateMessageMetaData>,
    pub client_key: String,
    pub from_user: Option<PrivateMessageUser>,
    pub time: i64,
    pub state: i64,
    pub result: i64,
    pub tips: String,
    pub sequence: i64,
    pub show_type: i64,
    pub msg_type: i64,
    pub confirm: i64,
    pub sort_time: i64,
    #[serde(alias = "complainTip")]
    pub complain_tip: String,
    #[serde(alias = "complainUrl")]
    pub complain_url: String,
}

/// 私信会话尾部标签.
///
/// 若服务端返回未建模的原始标签对象, 会自动包装到 `data` 字段中.
#[derive(Debug, Clone, Default)]
pub struct PrivateMessageTailTag {
    pub data: Value,
}

impl<'de> Deserialize<'de> for PrivateMessageTailTag {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let raw = Value::deserialize(de)?;
        if let Value::Object(map) = &raw {
            if map.contains_key("data") {
                Ok(PrivateMessageTailTag {
                    data: map.get("data").cloned().unwrap_or_default(),
                })
            } else {
                Ok(PrivateMessageTailTag { data: raw })
            }
        } else {
            Ok(PrivateMessageTailTag { data: raw })
        }
    }
}

/// 私信会话项.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PrivateMessageSession {
    pub session_id: String,
    pub user: Option<PrivateMessageUser>,
    pub new_msg: Option<PrivateMessageInfo>,
    pub new_msg_cnt: i64,
    pub sort_time: i64,
    pub url: String,
    #[serde(alias = "create_time")]
    pub create_time: i64,
    #[serde(alias = "from")]
    pub from: i64,
    /// 服务端字段名拼写如此, 保持别名与实际返回一致.
    #[serde(alias = "SmStarVirtaulUin")]
    pub sm_star_virtual_uin: String,
    #[serde(alias = "Auth")]
    pub auth: String,
    #[serde(alias = "Ext")]
    pub ext: std::collections::HashMap<String, String>,
    #[serde(alias = "TailTags")]
    pub tail_tags: Vec<PrivateMessageTailTag>,
}

/// 私信会话列表响应.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PrivateSessionListResponse {
    pub has_more: i64,
    pub msg: String,
    pub new_msg_cnt: i64,
    pub sessions: Vec<PrivateMessageSession>,
    pub subcode: i64,
    pub setting_guide: i64,
    pub state: i64,
    pub extra: std::collections::HashMap<String, String>,
}

/// 拍一拍文案信息.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PrivateMessagePatText {
    #[serde(alias = "Nick")]
    pub nick: String,
    #[serde(alias = "PatTxt")]
    pub pat_text: String,
}

/// 私信消息列表响应.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PrivateMessageListResponse {
    pub has_more: i64,
    pub messages: Vec<PrivateMessageInfo>,
    pub msg: String,
    pub session: Option<PrivateMessageSession>,
    pub subcode: i64,
    pub end_msg_seq: i64,
    #[serde(alias = "Attach", deserialize_with = "null_as_default")]
    pub attach: Value,
    #[serde(alias = "PatInterval")]
    pub pat_interval: i64,
    #[serde(alias = "PatMap", deserialize_with = "null_as_default")]
    pub pat_map: std::collections::HashMap<String, PrivateMessagePatText>,
    #[serde(alias = "EncryptStar")]
    pub encrypt_star: String,
    #[serde(alias = "LocationTips")]
    pub location_tips: String,
    #[serde(alias = "NewMsgCnt")]
    pub new_msg_cnt: i64,
}

/// 私信发送响应.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PrivateSendMessageResponse {
    pub messages: Vec<PrivateMessageInfo>,
    pub session: Option<PrivateMessageSession>,
    pub tips: String,
    pub identify_url: String,
    pub msg: String,
    pub reason: i64,
    pub end_msg_seq: i64,
    pub update_time: i64,
}

/// 私信写操作响应.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PrivateOperationResponse {
    pub msg: String,
    pub subcode: i64,
    pub tips: String,
}

/// 私信配置读取响应.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PrivateConfigResponse {
    pub config_value: i64,
    pub config_value_str: String,
    pub msg: String,
}

/// 音乐人卡片响应.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PrivateMusicianCardResponse {
    #[serde(default)]
    pub data: Value,
}

/// 聊天页入口项.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PrivateEntryItem {
    #[serde(alias = "EntryType")]
    pub entry_type: i64,
    #[serde(alias = "Icon")]
    pub icon: String,
    #[serde(alias = "Title")]
    pub title: String,
    #[serde(alias = "SkipScheme")]
    pub skip_scheme: String,
    #[serde(alias = "RightTopTag")]
    pub right_top_tag: String,
    #[serde(alias = "Ext")]
    pub ext: std::collections::HashMap<String, String>,
}

/// 聊天页入口响应.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PrivateChatEntriesResponse {
    #[serde(alias = "RetCode")]
    pub ret_code: i64,
    #[serde(alias = "RetMsg")]
    pub ret_msg: String,
    #[serde(alias = "Entries")]
    pub entries: std::collections::HashMap<String, Vec<PrivateEntryItem>>,
    #[serde(alias = "CanBeDazi")]
    pub can_be_dazi: Option<bool>,
    #[serde(alias = "DzData")]
    pub dz_data: Value,
}

/// 图片和视频消息详情响应.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PrivateMediaMessageDetailsResponse {
    #[serde(alias = "MsgIDs")]
    pub msg_ids: std::collections::HashMap<String, PrivateMessageInfo>,
}

/// 私信安全提示响应.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PrivateSafetyHintResponse {
    pub hint: String,
}
