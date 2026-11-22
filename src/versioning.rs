//! 请求平台版本策略 (对应 Python 端 `core/versioning.py`).

use serde_json::{Map, Value};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::models::Credential;
use crate::utils::hash33;

/// 请求平台枚举.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Platform {
    Android,
    Desktop,
    Web,
}

impl Platform {
    /// 平台字符串标识.
    pub fn as_str(&self) -> &'static str {
        match self {
            Platform::Android => "android",
            Platform::Desktop => "desktop",
            Platform::Web => "web",
        }
    }
}

/// 平台版本档案.
#[derive(Debug, Clone)]
pub struct VersionProfile {
    pub ct: i64,
    pub cv: i64,
    pub v: Option<i64>,
    pub platform: Option<String>,
    pub ua_version: Option<i64>,
    pub qimei_app_version: Option<String>,
    pub qimei_sdk_version: Option<String>,
}

impl VersionProfile {
    pub fn new(ct: i64, cv: i64) -> Self {
        VersionProfile {
            ct,
            cv,
            v: None,
            platform: None,
            ua_version: None,
            qimei_app_version: None,
            qimei_sdk_version: None,
        }
    }
}

/// 请求版本策略.
#[derive(Debug, Clone)]
pub struct VersionPolicy {
    pub android: VersionProfile,
    pub desktop: VersionProfile,
    pub web: VersionProfile,
}

impl VersionPolicy {
    pub fn get_profile(&self, platform: Platform) -> &VersionProfile {
        match platform {
            Platform::Android => &self.android,
            Platform::Desktop => &self.desktop,
            Platform::Web => &self.web,
        }
    }

    /// 构建统一 `comm` 参数.
    ///
    /// `session` 为 Android 平台该账号的不可变会话快照 (`uid`/`sid`),
    /// 与 `credential` 一起原子使用, 避免多账号并发时 session 串号.
    pub(crate) fn build_comm(
        &self,
        platform: Platform,
        credential: &Credential,
        device: &crate::Device,
        qimei: Option<&(String, String)>,
        session: Option<&crate::context::AndroidSession>,
    ) -> Value {
        let profile = self.get_profile(platform);
        let mut comm = Map::new();
        let mut insert = |key: &str, value: String| {
            comm.insert(key.to_string(), Value::String(value));
        };

        match platform {
            Platform::Android => {
                let guid = device.open_udid.clone();
                let uid_part = if credential.musicid != 0 {
                    credential.musicid.to_string()
                } else {
                    guid.clone()
                };
                let trace_timestamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();

                insert("ct", profile.ct.to_string());
                insert("cv", profile.cv.to_string());
                insert("v", profile.v.unwrap_or(profile.cv).to_string());
                insert("chid", "10003505".into());
                if credential.musicid != 0 {
                    insert("qq", credential.musicid.to_string());
                }
                if !credential.musickey.is_empty() {
                    insert("authst", credential.musickey.clone());
                }
                insert("tmeAppID", "qqmusic".into());
                if credential.login_type != 0 {
                    insert("tmeLoginType", credential.login_type.to_string());
                }
                insert("traceid", format!("10002_{uid_part}_{trace_timestamp}"));
                insert(
                    "QIMEI36",
                    qimei.map(|value| value.1.clone()).unwrap_or_default(),
                );
                insert("OpenUDID", guid.clone());
                insert("udid", guid);
                if let Some(session) = session {
                    insert("uid", session.uid.clone());
                }
                insert("OpenUDID2", device.open_udid2.clone());
                if let Some(session) = session {
                    insert("sid", session.sid.clone());
                }
                insert("aid", device.android_id.clone());
                insert("os_ver", device.version.release.clone());
                insert("phonetype", xml_escape_attr(&device.model));
            }
            Platform::Desktop => {
                insert("ct", profile.ct.to_string());
                insert("cv", profile.cv.to_string());
                if let Some(value) = profile.platform.as_ref() {
                    insert("platform", value.clone());
                }
                insert("chid", "0".into());
                if credential.musicid != 0 {
                    insert("uin", credential.musicid.to_string());
                }
                insert("g_tk", Self::get_g_tk(credential).to_string());
                insert("guid", device.open_udid.to_uppercase());
            }
            Platform::Web => {
                let g_tk = Self::get_g_tk(credential);
                insert("ct", profile.ct.to_string());
                insert("cv", profile.cv.to_string());
                if let Some(value) = profile.platform.as_ref() {
                    insert("platform", value.clone());
                }
                insert("chid", "0".into());
                insert("uin", credential.musicid.to_string());
                insert("g_tk", g_tk.to_string());
                insert("g_tk_new_20200303", g_tk.to_string());
                insert("format", "json".into());
                insert("inCharset", "utf-8".into());
                insert("outCharset", "utf-8".into());
                insert("notice", "0".into());
                insert("need_new_code", "1".into());
            }
        }
        Value::Object(comm)
    }

    /// 根据平台生成 User-Agent.
    pub fn get_user_agent(&self, platform: Platform, device: &crate::Device) -> String {
        match platform {
            Platform::Android => {
                let profile = self.get_profile(Platform::Android);
                let ua_version = profile.ua_version.unwrap_or(profile.cv);
                format!("QQMusic {ua_version}(android {})", device.version.release)
            }
            _ => "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36"
                .to_string(),
        }
    }

    /// 计算 g_tk.
    pub fn get_g_tk(credential: &Credential) -> i64 {
        if credential.musickey.is_empty() {
            5381
        } else {
            hash33(&credential.musickey, 5381)
        }
    }
}

impl Default for VersionPolicy {
    fn default() -> Self {
        let mut android = VersionProfile::new(11, 20_090_008);
        android.v = Some(20_090_008);
        android.ua_version = Some(20_090_008);
        android.qimei_app_version = Some("20.9.0.8".into());
        android.qimei_sdk_version = Some("1.2.13.6".into());

        let mut desktop = VersionProfile::new(19, 2201);
        desktop.platform = Some("yqq".into());

        let mut web = VersionProfile::new(24, 4747474);
        web.platform = Some("yqq.json".into());

        VersionPolicy {
            android,
            desktop,
            web,
        }
    }
}

fn xml_escape_attr(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device() -> crate::Device {
        let mut device = crate::Device::random();
        device.open_udid = "primary-udid".into();
        device.open_udid2 = "secondary-udid".into();
        device.android_id = "android-id".into();
        device.model = "A\"&B".into();
        device
    }

    #[test]
    fn android_comm_matches_current_protocol_and_trace_identity() {
        let policy = VersionPolicy::default();
        let credential = Credential {
            musicid: 42,
            musickey: "key".into(),
            login_type: 1,
            ..Default::default()
        };
        let qimei = ("ignored-q16".into(), "q36".into());
        let comm = policy.build_comm(
            Platform::Android,
            &credential,
            &device(),
            Some(&qimei),
            None,
        );

        assert_eq!(comm["ct"], "11");
        assert_eq!(comm["cv"], "20090008");
        assert_eq!(comm["v"], "20090008");
        assert_eq!(comm["QIMEI36"], "q36");
        assert!(comm.get("QIMEI").is_none());
        assert_eq!(comm["OpenUDID"], "primary-udid");
        assert_eq!(comm["OpenUDID2"], "secondary-udid");
        assert_eq!(comm["phonetype"], "A&quot;&amp;B");
        assert!(comm["traceid"].as_str().unwrap().starts_with("10002_42_"));
        assert!(comm["traceid"]
            .as_str()
            .unwrap()
            .rsplit_once('_')
            .and_then(|(_, timestamp)| timestamp.parse::<u64>().ok())
            .is_some());
    }

    #[test]
    fn guest_trace_uses_open_udid() {
        let comm = VersionPolicy::default().build_comm(
            Platform::Android,
            &Credential::default(),
            &device(),
            None,
            None,
        );
        assert!(comm["traceid"]
            .as_str()
            .unwrap()
            .starts_with("10002_primary-udid_"));
        assert!(comm.get("qq").is_none());
        assert!(comm.get("authst").is_none());
    }
}
