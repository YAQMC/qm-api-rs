//! 登录相关数据模型与状态枚举 (对应 Python 端 `models/login.py`).

use super::Credential;

/// 二维码登录类型.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QRLoginType {
    Qq,
    Wx,
    Mobile,
}

impl QRLoginType {
    pub fn as_str(&self) -> &'static str {
        match self {
            QRLoginType::Qq => "qq",
            QRLoginType::Wx => "wx",
            QRLoginType::Mobile => "mobile",
        }
    }
}

/// 二维码登录流程中的状态事件.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QRCodeLoginEvents {
    /// 登录完成, 携带凭证信息.
    Done,
    /// 二维码未被扫描, 等待扫描中.
    Scan,
    /// 二维码已被扫描, 等待确认中.
    Conf,
    /// 二维码过期或登录超时.
    Timeout,
    /// 用户拒绝登录.
    Refuse,
}

impl QRCodeLoginEvents {
    /// 根据状态码获取事件.
    pub fn get_by_value(value: i64) -> Option<QRCodeLoginEvents> {
        match value {
            0 | 405 => Some(QRCodeLoginEvents::Done),
            66 | 408 => Some(QRCodeLoginEvents::Scan),
            67 | 404 => Some(QRCodeLoginEvents::Conf),
            65 | 402 => Some(QRCodeLoginEvents::Timeout),
            68 | 403 => Some(QRCodeLoginEvents::Refuse),
            _ => None,
        }
    }
}

/// 手机验证码登录状态.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhoneLoginEvents {
    /// 验证码已发送.
    Send,
    /// 需要滑块验证.
    Captcha,
    /// 请求过于频繁.
    Frequency,
}

impl PhoneLoginEvents {
    pub fn value(&self) -> i64 {
        match self {
            PhoneLoginEvents::Send => 0,
            PhoneLoginEvents::Captcha => 20276,
            PhoneLoginEvents::Frequency => 100001,
        }
    }
}

/// 手机验证码发送接口的结果对象.
#[derive(Debug, Clone)]
pub struct PhoneAuthCodeResult {
    pub event: PhoneLoginEvents,
    pub info: Option<String>,
}

/// 二维码信息.
#[derive(Debug, Clone)]
pub struct QR {
    /// 二维码二进制数据.
    pub data: Vec<u8>,
    pub qr_type: QRLoginType,
    pub mimetype: String,
    /// 标识符 (QQ 为 qrsig, WX 为 uuid, Mobile 为 qrcodeID).
    pub identifier: String,
}

/// 二维码登录流程中的单次结果对象.
#[derive(Debug, Clone)]
pub struct QRLoginResult {
    pub event: QRCodeLoginEvents,
    pub credential: Option<Credential>,
}

impl QRLoginResult {
    pub fn done(&self) -> bool {
        self.event == QRCodeLoginEvents::Done
    }
}

/// 登录接口错误码 (需要特别处理).
pub const LOGIN_ERROR_CODES: [i64; 12] = [
    1000, 104401, 104400, 20261, 20271, 20272, 20274, 20277, 20278, 20279, 20450, 104604,
];
