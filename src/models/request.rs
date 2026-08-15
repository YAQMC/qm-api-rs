//! 凭据模型 (对应 Python 端 `models/request.py`).

use serde::{Deserialize, Serialize};

/// 登录凭证.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Credential {
    /// OpenID.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub openid: String,
    /// RefreshToken.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub refresh_token: String,
    /// AccessToken.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub access_token: String,
    /// 到期时间.
    #[serde(default)]
    pub expired_at: i64,
    /// QQMusicID.
    #[serde(default)]
    pub musicid: i64,
    /// QQMusicKey.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub musickey: String,
    /// UnionID.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub unionid: String,
    /// 字符串形式的 QQMusicID.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub str_musicid: String,
    /// RefreshKey.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub refresh_key: String,
    #[serde(default, alias = "musickeyCreateTime")]
    pub musickey_create_time: i64,
    #[serde(default, alias = "keyExpiresIn")]
    pub key_expires_in: i64,
    #[serde(default)]
    pub first_login: i64,
    #[serde(default, alias = "bindAccountType")]
    pub bind_account_type: i64,
    #[serde(default, alias = "needRefreshKeyIn")]
    pub need_refresh_key_in: i64,
    #[serde(default, alias = "encryptUin")]
    pub encrypt_uin: String,
    /// 登录类型 (1=微信, 2=QQ).
    #[serde(default, alias = "loginType")]
    pub login_type: i64,
}

impl Credential {
    /// 字符串形式的 musicid, 优先使用 `str_musicid`.
    pub fn str_musicid(&self) -> String {
        if !self.str_musicid.is_empty() {
            self.str_musicid.clone()
        } else if self.musicid != 0 {
            self.musicid.to_string()
        } else {
            String::new()
        }
    }

    /// 检查凭据是否已过期.
    pub fn is_expired(&self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        self.musickey_create_time != 0
            && self.key_expires_in != 0
            && now >= self.musickey_create_time + self.key_expires_in
    }
}
