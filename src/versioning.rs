//! 请求平台版本策略 (对应 Python 端 `core/versioning.py`).

use serde_json::{json, Value};

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
    pub fn build_comm(
        &self,
        platform: Platform,
        credential: &Credential,
        device: &crate::Device,
        qimei: Option<&(String, String)>,
    ) -> Value {
        let profile = self.get_profile(platform);
        match platform {
            Platform::Android => {
                let (q16, q36) = qimei.map(|q| (q.0.clone(), q.1.clone())).unwrap_or_default();
                let guid = device.open_udid.clone();
                json!({
                    "ct": profile.ct,
                    "cv": profile.cv,
                    "v": profile.v.unwrap_or(profile.cv),
                    "chid": "10003505",
                    "qq": credential.musicid.to_string(),
                    "authst": credential.musickey,
                    "tmeAppID": "qqmusic",
                    "tmeLoginType": credential.login_type,
                    "QIMEI": q16,
                    "QIMEI36": q36,
                    "OpenUDID": guid,
                    "udid": guid,
                    "uid": device.session_uid.clone().unwrap_or_default(),
                    "OpenUDID2": guid,
                    "sid": device.session_sid.clone().unwrap_or_default(),
                    "aid": device.android_id.clone(),
                    "os_ver": device.version.release.clone(),
                    "phonetype": device.model.clone(),
                    "devicelevel": device.version.sdk.to_string(),
                    "newdevicelevel": device.version.sdk.to_string(),
                    "rom": device.fingerprint.clone(),
                })
            }
            Platform::Desktop => json!({
                "ct": profile.ct,
                "cv": profile.cv,
                "platform": profile.platform,
                "chid": "0",
                "uin": credential.musicid,
                "g_tk": Self::get_g_tk(credential),
                "guid": device.open_udid.to_uppercase(),
            }),
            Platform::Web => {
                let g_tk = Self::get_g_tk(credential);
                json!({
                    "ct": profile.ct,
                    "cv": profile.cv,
                    "platform": profile.platform,
                    "chid": "0",
                    "uin": credential.musicid,
                    "g_tk": g_tk,
                    "g_tk_new_20200303": g_tk,
                    "format": "json",
                    "inCharset": "utf-8",
                    "outCharset": "utf-8",
                    "notice": 0,
                    "need_new_code": 1,
                })
            }
        }
    }

    /// 根据平台生成 User-Agent.
    pub fn get_user_agent(&self, platform: Platform, device: &crate::Device) -> String {
        match platform {
            Platform::Android => {
                let profile = self.get_profile(Platform::Android);
                let ua_version = profile.ua_version.unwrap_or(profile.cv);
                format!("QQMusic {ua_version}(android {})", device.version.release)
            }
            _ => {
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36"
                    .to_string()
            }
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
        let mut android = VersionProfile::new(11, 14_090_008);
        android.v = Some(14_090_008);
        android.ua_version = Some(14_090_008);
        android.qimei_app_version = Some("14.9.0.8".into());
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
