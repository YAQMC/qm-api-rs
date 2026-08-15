//! 凭据模型 (对应 Python 端 `models/request.py`).

use serde::{Deserialize, Serialize};

/// 登录凭证.
///
/// `Debug` 实现为 secret-safe: 所有令牌/密钥字段均以 `[redacted]` 输出,
/// 不会进入日志. 序列化 (登录/刷新结果解析) 不受影响.
#[derive(Clone, Default, Serialize, Deserialize)]
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

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 仅暴露非敏感标识字段; 令牌/密钥一律 redacted.
        f.debug_struct("Credential")
            .field("musicid", &self.musicid)
            .field("str_musicid", &self.str_musicid)
            .field("login_type", &self.login_type)
            .field("expired_at", &self.expired_at)
            .field("bind_account_type", &self.bind_account_type)
            .field("musickey", &"[redacted]")
            .field("refresh_token", &"[redacted]")
            .field("access_token", &"[redacted]")
            .field("refresh_key", &"[redacted]")
            .field("openid", &"[redacted]")
            .field("unionid", &"[redacted]")
            .field("encrypt_uin", &"[redacted]")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts_secrets() {
        let cred = Credential {
            musicid: 10001,
            str_musicid: "10001".into(),
            musickey: "super-secret-musickey".into(),
            access_token: "super-secret-token".into(),
            refresh_token: "super-secret-refresh".into(),
            refresh_key: "super-secret-refreshkey".into(),
            ..Default::default()
        };
        let dbg = format!("{cred:?}");
        assert!(dbg.contains("10001"), "应保留可读的 musicid");
        assert!(!dbg.contains("super-secret-musickey"));
        assert!(!dbg.contains("super-secret-token"));
        assert!(!dbg.contains("super-secret-refresh"));
        assert!(dbg.contains("[redacted]"));
    }

    #[test]
    fn serde_roundtrip_preserves_secrets() {
        let cred = Credential {
            musicid: 42,
            musickey: "k".into(),
            refresh_token: "rt".into(),
            ..Default::default()
        };
        let json = serde_json::to_string(&cred).unwrap();
        let back: Credential = serde_json::from_str(&json).unwrap();
        assert_eq!(back.musickey, "k");
        assert_eq!(back.refresh_token, "rt");
    }
}
