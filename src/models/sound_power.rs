//! 音响力与好友听歌榜响应模型.

use serde::Deserialize;
use serde_json::Value;

use crate::models::de::null_as_default;

/// 音响力等级特权项.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SoundPowerEquity {
    #[serde(alias = "eType")]
    pub etype: i64,
    pub name: String,
    pub icon: String,
    pub scheme: String,
}

/// 音响力基础等级信息.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SoundPowerInfo {
    pub level: i64,
    pub value: i64,
    pub name: String,
    pub icon: String,
    #[serde(alias = "currentValue")]
    pub current_value: i64,
    #[serde(alias = "nextValue")]
    pub next_value: i64,
    #[serde(alias = "reachMaxLevel")]
    pub reach_max_level: i64,
    #[serde(alias = "isHugeVip")]
    pub is_huge_vip: bool,
    #[serde(alias = "nextLevelNums")]
    pub next_level_nums: i64,
    #[serde(alias = "nextLevelIcon")]
    pub next_level_icon: String,
}

/// 音响力等级主页响应.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SoundPowerDetailResponse {
    #[serde(alias = "powerInfo")]
    pub power_info: SoundPowerInfo,
    #[serde(alias = "medalNum")]
    pub medal_num: i64,
    pub nick: String,
    #[serde(alias = "headPic")]
    pub head_pic: String,
    #[serde(alias = "todayLT")]
    pub today_listen_time: i64,
    #[serde(alias = "equityText")]
    pub equity_text: String,
    #[serde(deserialize_with = "null_as_default")]
    pub equities: Vec<SoundPowerEquity>,
    #[serde(alias = "isHugevip")]
    pub is_hugevip: i64,
    #[serde(alias = "hugevipPowerInfo")]
    pub hugevip_power_info: Option<Value>,
    #[serde(alias = "medalInfo")]
    pub medal_info: Option<Value>,
}

/// 超级会员音响力等级规则条目.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct HugevipLevelRuleItem {
    pub level: i64,
    pub name: String,
    pub icon: String,
    #[serde(alias = "needDuration")]
    pub need_duration: i64,
    pub percent: String,
}

/// 超级会员音响力加速与等级规则.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct HugevipLevelRuleResponse {
    #[serde(alias = "currentDuration")]
    pub current_duration: i64,
    #[serde(alias = "nextLevelDuration")]
    pub next_level_duration: i64,
    #[serde(deserialize_with = "null_as_default")]
    pub rules: Vec<HugevipLevelRuleItem>,
}

/// 好友听歌日榜条目.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct FriendRankItem {
    pub uin: String,
    pub nick: String,
    pub pic: String,
    #[serde(alias = "playTime")]
    pub play_time: i64,
    #[serde(alias = "likeNum")]
    pub like_num: i64,
    #[serde(alias = "likeStatus")]
    pub like_status: i64,
    pub no: i64,
    #[serde(alias = "spInfo")]
    pub sp_info: Value,
    #[serde(alias = "mInfo")]
    pub medal_info: Value,
}

/// 好友听歌日榜响应.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct FriendRankResponse {
    #[serde(deserialize_with = "null_as_default")]
    pub ranks: Vec<FriendRankItem>,
    #[serde(alias = "myRankNo")]
    pub my_rank_no: i64,
    #[serde(alias = "myPlayTime")]
    pub my_play_time: i64,
    #[serde(alias = "myLikeNum")]
    pub my_like_num: i64,
    #[serde(alias = "hasMore")]
    pub has_more: i64,
    pub rankno: i64,
}

/// 好友听歌榜点赞操作响应.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct LikeFriendResponse {
    #[serde(alias = "NOUSE")]
    pub no_use: i64,
}

/// 好友排行榜隐私设置响应.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SetRankPrivacyResponse {
    pub status: i64,
}

/// 听歌勋章条目.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SoundPowerMedalItem {
    #[serde(alias = "PicURL")]
    pub pic_url: String,
    #[serde(alias = "Scheme")]
    pub scheme: String,
}

/// 音响力勋章馆入口响应.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SoundPowerMedalEntryResponse {
    #[serde(alias = "Title")]
    pub title: String,
    #[serde(alias = "MedalCnt")]
    pub medal_cnt: i64,
    #[serde(alias = "MedalList", deserialize_with = "null_as_default")]
    pub medal_list: Vec<SoundPowerMedalItem>,
}

/// 音响力加速任务响应.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ActTaskModulesResponse {
    #[serde(alias = "retCode")]
    pub ret_code: i64,
    #[serde(alias = "retMsg")]
    pub ret_msg: String,
    #[serde(alias = "taskModules")]
    pub task_modules: Option<Vec<Value>>,
    #[serde(alias = "actInfo")]
    pub act_info: Value,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn null_lists_are_decoded_as_empty_and_aliases_are_preserved() {
        let detail: SoundPowerDetailResponse = serde_json::from_value(json!({
            "powerInfo":{"level":3,"currentValue":12,"isHugeVip":true},
            "equities":null,
            "todayLT":99,
            "hugevipPowerInfo":null
        }))
        .unwrap();
        assert_eq!(detail.power_info.level, 3);
        assert_eq!(detail.power_info.current_value, 12);
        assert!(detail.power_info.is_huge_vip);
        assert_eq!(detail.today_listen_time, 99);
        assert!(detail.equities.is_empty());

        let medals: SoundPowerMedalEntryResponse = serde_json::from_value(json!({
            "Title":"听歌勋章", "MedalCnt":2, "MedalList":null
        }))
        .unwrap();
        assert_eq!(medals.title, "听歌勋章");
        assert!(medals.medal_list.is_empty());
    }
}
