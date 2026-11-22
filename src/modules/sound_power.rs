//! 音响力等级、成长特权与好友听歌日榜 API.

use serde_json::{json, Value};

use super::ApiModule;
use crate::context::RequestOptions;
use crate::error::{QmError, Result};
use crate::models::sound_power::*;
use crate::models::Credential;

/// 音响力相关 API.
#[derive(Clone, Debug)]
pub struct SoundPowerApi {
    pub(crate) base: ApiModule,
}

impl SoundPowerApi {
    pub(crate) fn new(context: std::sync::Arc<crate::context::ApiContext>) -> Self {
        Self {
            base: ApiModule::new(context),
        }
    }

    fn login_options(credential: Option<&Credential>) -> RequestOptions {
        let mut options = RequestOptions::default();
        options.require_login = true;
        options.credential = credential.cloned();
        options
    }

    /// 获取当前账号的音响力主页、今日有效听歌时长与权益.
    pub async fn get_detail(
        &self,
        credential: Option<&Credential>,
    ) -> Result<SoundPowerDetailResponse> {
        let data = self
            .base
            .cgi(
                "music.soundPower.SoundPowerSvr",
                "QueryLevelDetailPage",
                json!({}),
                Self::login_options(credential),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取超级会员音响力加速与等级规则.
    pub async fn get_hugevip_rule(
        &self,
        credential: Option<&Credential>,
    ) -> Result<HugevipLevelRuleResponse> {
        let data = self
            .base
            .cgi(
                "music.soundPower.SoundPowerSvr",
                "QueryHugevipLevelRule",
                json!({}),
                Self::login_options(credential),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取好友听歌日榜的一页.
    pub async fn get_friend_rank(
        &self,
        offset: i64,
        limit: i64,
        last_uin: &str,
        rankno: i64,
        credential: Option<&Credential>,
    ) -> Result<FriendRankResponse> {
        if offset < 0 || limit <= 0 || rankno < 0 {
            return Err(QmError::ValueError("invalid friend rank pagination".into()));
        }
        let data = self
            .base
            .cgi(
                "music.activeCenter.FriendRankSvr",
                "GetRank",
                json!({
                    "rank_type": 1,
                    "offset": offset,
                    "limit": limit,
                    "last_uin": last_uin,
                    "rankno": rankno,
                }),
                Self::login_options(credential),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 为好友听歌榜条目点赞或取消点赞.
    pub async fn like_friend(
        &self,
        uin: &str,
        cancel: bool,
        credential: Option<&Credential>,
    ) -> Result<LikeFriendResponse> {
        if uin.trim().is_empty() {
            return Err(QmError::ValueError("friend uin must not be empty".into()));
        }
        let data = self
            .base
            .cgi(
                "music.activeCenter.FriendRankSvr",
                "Like",
                json!({
                    "rank_type": 1,
                    "uin": uin,
                    "cancel": if cancel { 1 } else { 0 },
                }),
                Self::login_options(credential),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 设置好友排行榜隐私状态.
    pub async fn set_rank_privacy(
        &self,
        status: i64,
        credential: Option<&Credential>,
    ) -> Result<SetRankPrivacyResponse> {
        let data = self
            .base
            .cgi(
                "music.activeCenter.FriendRankSvr",
                "SetPrivacy",
                json!({ "status": status }),
                Self::login_options(credential),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取音响力勋章馆入口及佩戴勋章信息.
    pub async fn get_medal_entry(
        &self,
        enc_uin: &str,
        credential: Option<&Credential>,
    ) -> Result<SoundPowerMedalEntryResponse> {
        let data = self
            .base
            .cgi(
                "music.medalHall.MedalHallEntrySrv",
                "GetSoundPowerEntry",
                json!({ "EncUin": enc_uin }),
                Self::login_options(credential),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 使用参考协议默认值获取音响力加速任务.
    pub async fn get_tasks(
        &self,
        credential: Option<&Credential>,
    ) -> Result<ActTaskModulesResponse> {
        self.get_tasks_with("1nsAQf", None, credential).await
    }

    /// 获取指定活动与任务模块的音响力加速任务.
    pub async fn get_tasks_with(
        &self,
        act_id: &str,
        task_module_ids: Option<&[String]>,
        credential: Option<&Credential>,
    ) -> Result<ActTaskModulesResponse> {
        if act_id.trim().is_empty() {
            return Err(QmError::ValueError(
                "task activity id must not be empty".into(),
            ));
        }
        let default_ids = [String::from("Z1jtHy7")];
        let task_module_ids: Value = match task_module_ids {
            Some(ids) if !ids.is_empty() => json!(ids),
            Some(_) => {
                return Err(QmError::ValueError(
                    "task module ids must not be empty".into(),
                ));
            }
            None => json!(default_ids),
        };
        let data = self
            .base
            .cgi(
                "music.activeCenter.ActTaskNewSvr",
                "GetTaskModules",
                json!({
                    "actID": act_id,
                    "taskModuleIDs": task_module_ids,
                }),
                Self::login_options(credential),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }
}
