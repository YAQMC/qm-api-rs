//! 私信相关 API 模块 (对应 Python 端 `modules/private_message.py`).
//!
//! 所有接口固定使用 Android 平台, 需要登录凭证.

use serde_json::{json, Value};

use super::ApiModule;
use crate::context::RequestOptions;
use crate::error::Result;
use crate::models::private_message::*;
use crate::models::Credential;
use crate::versioning::Platform;

const PRIVATE_MSG_READ_MODULE: &str = "music.privateMsg.PrivateMsgRead";
const PRIVATE_MSG_WRITE_MODULE: &str = "music.privateMsg.PrivateMsgWrite";

/// 私信相关 API.
#[derive(Clone, Debug)]
pub struct PrivateMessageApi {
    pub(crate) base: ApiModule,
}

impl PrivateMessageApi {
    pub(crate) fn new(context: std::sync::Arc<crate::context::ApiContext>) -> Self {
        PrivateMessageApi {
            base: ApiModule::new(context),
        }
    }

    fn opts(&self, credential: Option<&Credential>) -> RequestOptions {
        let mut opts = RequestOptions::default();
        opts.require_login = true;
        opts.credential = credential.cloned();
        opts.platform = Some(Platform::Android);
        opts
    }

    /// 获取私信会话列表.
    #[allow(clippy::too_many_arguments)]
    pub async fn get_sessions(
        &self,
        last_id: &str,
        order: i64,
        size: i64,
        last_time: i64,
        from: i64,
        fans_flag: Option<i64>,
        encrypt_from_uin: Option<&str>,
        credential: Option<&Credential>,
    ) -> Result<PrivateSessionListResponse> {
        let mut params = json!({
            "last_id": last_id,
            "order": order,
            "size": size,
            "last_time": last_time,
            "from": from,
        });
        if let Some(eu) = encrypt_from_uin {
            params["EncryptFromUin"] = json!(eu);
        } else if let Some(flag) = fans_flag {
            params["FansFlag"] = json!(flag);
        }
        let data = self
            .base
            .cgi(PRIVATE_MSG_READ_MODULE, "GetSessionList", params, self.opts(credential))
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 删除私信会话.
    pub async fn delete_session(
        &self,
        session_id: &str,
        super_msg_flag: i64,
        credential: Option<&Credential>,
    ) -> Result<PrivateOperationResponse> {
        let data = self
            .base
            .cgi(
                PRIVATE_MSG_WRITE_MODULE,
                "DeleteSession",
                json!({ "session_id": session_id, "super_msg_flag": super_msg_flag }),
                self.opts(credential),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取私信聊天消息列表.
    #[allow(clippy::too_many_arguments)]
    pub async fn get_messages(
        &self,
        session_id: &str,
        user_id: &str,
        last_id: &str,
        wns_id: &str,
        order: i64,
        size: i64,
        flag: i64,
        location_id: Option<&str>,
        update_time: Option<i64>,
        credential: Option<&Credential>,
    ) -> Result<PrivateMessageListResponse> {
        let mut params = json!({ "order": order, "size": size, "flag": flag });
        let mut optional = serde_json::Map::new();
        if !session_id.is_empty() {
            optional.insert("session_id".into(), json!(session_id));
        }
        if !last_id.is_empty() {
            optional.insert("last_id".into(), json!(last_id));
        }
        if !wns_id.is_empty() {
            optional.insert("wns_id".into(), json!(wns_id));
        }
        if !user_id.is_empty() {
            optional.insert("user_id".into(), json!(user_id));
        }
        if let Some(loc) = location_id {
            optional.insert("location_id".into(), json!(loc));
        }
        if let Some(ut) = update_time {
            optional.insert("update_time".into(), json!(ut));
        }
        if let Some(obj) = params.as_object_mut() {
            for (k, v) in optional {
                obj.insert(k, v);
            }
        }
        let data = self
            .base
            .cgi(PRIVATE_MSG_READ_MODULE, "GetMessage", params, self.opts(credential))
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 发送私信消息.
    #[allow(clippy::too_many_arguments)]
    pub async fn send_message(
        &self,
        user_id: &str,
        msg_type: i64,
        session_id: &str,
        last_id: &str,
        last_msg_seq: i64,
        meta_data: Option<Value>,
        entrance: i64,
        client_key: &str,
        source_flag: Option<i64>,
        msg_id: Option<&str>,
        user_input: Option<&str>,
        super_msg_flag: Option<i64>,
        star_send: bool,
        credential: Option<&Credential>,
    ) -> Result<PrivateSendMessageResponse> {
        let mut params = json!({
            "last_msg_seq": last_msg_seq,
            "user_id": user_id,
            "entrance": entrance,
            "client_key": client_key,
            "msg_type": msg_type,
        });
        if !session_id.is_empty() {
            params["session_id"] = json!(session_id);
        }
        if !last_id.is_empty() {
            params["last_id"] = json!(last_id);
        }
        if let Some(md) = meta_data {
            params["meta_data"] = md;
        }
        if let Some(sf) = source_flag {
            params["source_flag"] = json!(sf);
        }
        if let Some(mid) = msg_id {
            params["msg_id"] = json!(mid);
        }
        if let Some(ui) = user_input {
            params["user_input"] = json!(ui);
        }
        if let Some(smf) = super_msg_flag {
            params["super_msg_flag"] = json!(smf);
        }
        let method = if star_send { "StarSendSuperMsg" } else { "SendMessageAsync" };
        let data = self
            .base
            .cgi(PRIVATE_MSG_WRITE_MODULE, method, params, self.opts(credential))
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 删除单条私信消息.
    pub async fn delete_message(
        &self,
        session_id: &str,
        msg_id: &str,
        super_msg_flag: i64,
        credential: Option<&Credential>,
    ) -> Result<PrivateOperationResponse> {
        let data = self
            .base
            .cgi(
                PRIVATE_MSG_WRITE_MODULE,
                "DeleteMessage",
                json!({ "session_id": session_id, "msg_id": msg_id, "super_msg_flag": super_msg_flag }),
                self.opts(credential),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 清空私信会话消息.
    pub async fn clear_session(
        &self,
        session_id: &str,
        super_msg_flag: i64,
        credential: Option<&Credential>,
    ) -> Result<PrivateOperationResponse> {
        let data = self
            .base
            .cgi(
                PRIVATE_MSG_WRITE_MODULE,
                "ClearSession",
                json!({ "session_id": session_id, "super_msg_flag": super_msg_flag }),
                self.opts(credential),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 写入私信配置.
    pub async fn set_config(
        &self,
        config_type: i64,
        config_value: &str,
        credential: Option<&Credential>,
    ) -> Result<PrivateOperationResponse> {
        let data = self
            .base
            .cgi(
                PRIVATE_MSG_WRITE_MODULE,
                "SetConfig",
                json!({ "config_type": config_type, "config_value_str": config_value }),
                self.opts(credential),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 读取私信配置.
    pub async fn get_config(&self, config_type: i64, config_value: &str, credential: Option<&Credential>) -> Result<PrivateConfigResponse> {
        let data = self
            .base
            .cgi(
                PRIVATE_MSG_READ_MODULE,
                "GetConfig",
                json!({ "config_type": config_type, "config_value_str": config_value }),
                self.opts(credential),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取音乐人私信卡片.
    pub async fn get_musician_message_card(
        &self,
        enc_uin: &str,
        credential: Option<&Credential>,
    ) -> Result<PrivateMusicianCardResponse> {
        let data = self
            .base
            .cgi(
                "music.privateMsg.MusicianMsgCardSvr",
                "GetMusicianCard",
                json!({ "EncUin": enc_uin }),
                self.opts(credential),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 上报卡片消息操作回调.
    pub async fn report_card_message_action(
        &self,
        target_user_id: &str,
        msg_type: i64,
        confirm: i64,
        msg_id: &str,
        ext: Option<Value>,
        credential: Option<&Credential>,
    ) -> Result<PrivateOperationResponse> {
        let mut params = json!({
            "target_user_id": target_user_id,
            "msg_type": msg_type,
            "confirm": confirm,
            "msg_id": msg_id,
        });
        if let Some(e) = ext {
            params["ext"] = e;
        }
        let data = self
            .base
            .cgi(PRIVATE_MSG_WRITE_MODULE, "ActCardMsgCallBack", params, self.opts(credential))
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取聊天页功能入口.
    pub async fn get_chat_entries(
        &self,
        scenes: &[i64],
        from_user_type: Option<i64>,
        user_id: Option<&str>,
        ext: Option<Value>,
        credential: Option<&Credential>,
    ) -> Result<PrivateChatEntriesResponse> {
        let mut params = serde_json::Map::new();
        params.insert("Scence".into(), json!(scenes));
        if let Some(f) = from_user_type {
            params.insert("FromUserType".into(), json!(f));
        }
        if let Some(u) = user_id {
            params.insert("UserID".into(), json!(u));
        }
        if let Some(e) = ext {
            params.insert("Ext".into(), e);
        }
        let data = self
            .base
            .cgi(PRIVATE_MSG_READ_MODULE, "GetEntries", Value::Object(params), self.opts(credential))
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取图片或视频消息详情.
    pub async fn get_media_message_details(
        &self,
        session_id: &str,
        msg_ids: &[String],
        credential: Option<&Credential>,
    ) -> Result<PrivateMediaMessageDetailsResponse> {
        let data = self
            .base
            .cgi(
                PRIVATE_MSG_READ_MODULE,
                "GetMsgDetails",
                json!({ "SessionID": session_id, "MsgIDs": msg_ids }),
                self.opts(credential),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 设置私信全部已读.
    pub async fn mark_all_messages_read(
        &self,
        cmd_flag: i64,
        encrypt_uin: &str,
        credential: Option<&Credential>,
    ) -> Result<PrivateOperationResponse> {
        let data = self
            .base
            .cgi(
                PRIVATE_MSG_WRITE_MODULE,
                "SetAllMsgMardRead",
                json!({ "CmdFlag": cmd_flag, "EncryptUin": encrypt_uin }),
                self.opts(credential),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取私信安全提示.
    pub async fn get_safety_hint(
        &self,
        enc_uin: &str,
        close: i64,
        credential: Option<&Credential>,
    ) -> Result<PrivateSafetyHintResponse> {
        let data = self
            .base
            .cgi(
                PRIVATE_MSG_READ_MODULE,
                "GetSafetyHint",
                json!({ "encUin": enc_uin, "close": close }),
                self.opts(credential),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// ⚠️ **Raw 透传** — 获取聊天页好友浮标.
    ///
    /// 响应 schema 未经 live 验证, 仅提供透传能力.
    pub async fn raw_get_friendship_badge(&self, target_enc_uin: &str, credential: Option<&Credential>) -> Result<Value> {
        self.base
            .cgi(
                "music.dazi.DzEntrySrv",
                "GetFriendFloatingIcon",
                json!({ "TargetEncuin": target_enc_uin }),
                self.opts(credential),
            )
            .await
    }
}
