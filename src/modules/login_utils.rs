//! 登录流程工具 (对应 Python 端 `modules/login_utils.py`).

use std::time::{Duration, Instant};

use super::login::LoginApi;
use crate::error::{QmError, Result};
use crate::models::login::{
    PhoneAuthCodeResult, QR, QRCodeLoginEvents, QRLoginResult, QRLoginType,
};
use crate::models::Credential;

/// 封装手机验证码登录流程的会话对象.
#[derive(Debug, Clone)]
pub struct PhoneLoginSession {
    pub api: LoginApi,
    /// 手机号 (明文) 或加密手机号.
    pub phone: String,
    /// 手机号是否已加密.
    pub is_encrypted: bool,
    pub country_code: i64,
    pub last_result: Option<PhoneAuthCodeResult>,
}

impl PhoneLoginSession {
    pub fn new(api: LoginApi, phone: &str, is_encrypted: bool, country_code: i64) -> Self {
        PhoneLoginSession {
            api,
            phone: phone.to_string(),
            is_encrypted,
            country_code,
            last_result: None,
        }
    }

    /// 发送当前会话手机号对应的验证码.
    pub async fn send_authcode(&mut self) -> Result<PhoneAuthCodeResult> {
        let result = self
            .api
            .send_authcode(&self.phone, self.is_encrypted, self.country_code)
            .await?;
        self.last_result = Some(result.clone());
        Ok(result)
    }

    /// 使用验证码完成当前会话的登录鉴权.
    pub async fn authorize(&self, auth_code: &str) -> Result<Credential> {
        self.api.phone_authorize(&self.phone, self.is_encrypted, auth_code).await
    }
}

/// 二维码登录轮询间隔控制策略 (单位: 秒).
#[derive(Debug, Clone, Copy)]
pub struct PollInterval {
    pub default: f64,
    pub scanned: Option<f64>,
    pub error: Option<f64>,
}

impl PollInterval {
    /// 已扫码状态下的轮询间隔 (计算值).
    pub fn scanned_interval(&self) -> f64 {
        self.scanned.unwrap_or(self.default / 2.0)
    }

    /// 异常退避、网络错误时的最大退避间隔.
    pub fn error_interval(&self) -> f64 {
        self.error.unwrap_or(self.default * 2.0)
    }
}

impl Default for PollInterval {
    fn default() -> Self {
        PollInterval {
            default: 1.5,
            scanned: None,
            error: None,
        }
    }
}

/// 封装二维码登录轮询与会话的对象.
#[derive(Debug, Clone)]
pub struct QRCodeLoginSession {
    pub api: LoginApi,
    pub login_type: QRLoginType,
    pub interval: PollInterval,
    pub timeout_seconds: f64,
    pub emit_repeat: bool,
    pub qrcode: Option<QR>,
}

impl QRCodeLoginSession {
    pub fn new(api: LoginApi, login_type: QRLoginType) -> Self {
        QRCodeLoginSession {
            api,
            login_type,
            interval: PollInterval::default(),
            timeout_seconds: 180.0,
            emit_repeat: false,
            qrcode: None,
        }
    }

    pub fn with_interval(mut self, interval: PollInterval) -> Self {
        self.interval = interval;
        self
    }

    pub fn with_timeout(mut self, timeout_seconds: f64) -> Self {
        assert!(timeout_seconds > 0.0, "timeout_seconds 必须大于 0");
        self.timeout_seconds = timeout_seconds;
        self
    }

    pub fn with_emit_repeat(mut self, emit_repeat: bool) -> Self {
        self.emit_repeat = emit_repeat;
        self
    }

    /// 获取并缓存当前会话的二维码对象.
    pub async fn get_qrcode(&mut self) -> Result<QR> {
        if self.qrcode.is_none() {
            self.qrcode = Some(self.api.get_qrcode(self.login_type).await?);
        }
        Ok(self.qrcode.clone().expect("qrcode"))
    }

    /// 轮询二维码状态, 逐个产出事件.
    ///
    /// 手机客户端二维码走 MQTT 推送; QQ/微信走 HTTP 轮询.
    /// 不会产出与上一次相同的事件 (除非设置了 `emit_repeat`).
    pub async fn iter_events(&mut self) -> Result<Vec<QRLoginResult>> {
        let qrcode = self.get_qrcode().await?;

        // 手机客户端二维码: 通过 MQTT 接收推送.
        if qrcode.qr_type == QRLoginType::Mobile {
            let timeout = std::time::Duration::from_secs_f64(self.timeout_seconds);
            let events = self.api.checking_mobile_qrcode(&qrcode, timeout).await?;
            let mut out = Vec::new();
            let mut last_event: Option<QRCodeLoginEvents> = None;
            for item in events {
                if !self.emit_repeat && last_event == Some(item.event) {
                    continue;
                }
                last_event = Some(item.event);
                out.push(item);
            }
            return Ok(out);
        }

        let deadline = Instant::now() + Duration::from_secs_f64(self.timeout_seconds);
        let mut last_event: Option<QRCodeLoginEvents> = None;
        let mut events = Vec::new();
        let mut error_retries: u32 = 0;

        loop {
            let loop_start = Instant::now();
            let timeout_left = deadline.saturating_duration_since(Instant::now());
            if timeout_left.is_zero() {
                events.push(QRLoginResult {
                    event: QRCodeLoginEvents::Timeout,
                    credential: None,
                });
                return Ok(events);
            }

            let result = tokio::time::timeout(timeout_left, self.api.check_qrcode(&qrcode)).await;
            match result {
                Err(_) => {
                    events.push(QRLoginResult {
                        event: QRCodeLoginEvents::Timeout,
                        credential: None,
                    });
                    return Ok(events);
                }
                Ok(Err(_)) => {
                    // 网络错误: 指数退避后重试
                    let backoff =
                        self.interval.error_interval().min(((1u32 << error_retries) as f64) * self.interval.default);
                    error_retries += 1;
                    tokio::time::sleep(Duration::from_secs_f64(backoff)).await;
                    continue;
                }
                Ok(Ok(item)) => {
                    error_retries = 0;
                    if !self.emit_repeat && last_event == Some(item.event) {
                        // 忽略重复事件, 但仍检查是否终端状态
                        if is_terminal(&item.event) {
                            return Ok(events);
                        }
                        let elapsed = loop_start.elapsed().as_secs_f64();
                        sleep_until(&deadline, self.interval.default.max(1.0 - elapsed)).await;
                        continue;
                    }
                    last_event = Some(item.event);
                    events.push(item.clone());
                    if is_terminal(&item.event) {
                        return Ok(events);
                    }
                    let sleep_time = match item.event {
                        QRCodeLoginEvents::Conf => self.interval.scanned_interval(),
                        _ => self.interval.default,
                    };
                    let elapsed = loop_start.elapsed().as_secs_f64();
                    sleep_until(&deadline, sleep_time.max(1.0 - elapsed)).await;
                }
            }
        }
    }

    /// 等待二维码登录完成并返回凭证.
    ///
    /// 用户拒绝 (`Refuse`) 或二维码超时 (`Timeout`) 时返回错误.
    pub async fn wait_qrcode_login(&mut self) -> Result<Credential> {
        let events = self.iter_events().await?;
        for result in events {
            match result.event {
                QRCodeLoginEvents::Done => {
                    return result
                        .credential
                        .ok_or_else(|| QmError::Login {
                            message: "登录结果缺少凭证".into(),
                            code: -1,
                        });
                }
                QRCodeLoginEvents::Refuse => {
                    return Err(QmError::Login {
                        message: "用户拒绝了登录请求".into(),
                        code: -1,
                    });
                }
                QRCodeLoginEvents::Timeout => {
                    return Err(QmError::Login {
                        message: "登录二维码已超时".into(),
                        code: -1,
                    });
                }
                _ => {}
            }
        }
        Err(QmError::Login {
            message: "登录流程异常结束".into(),
            code: -1,
        })
    }
}

fn is_terminal(event: &QRCodeLoginEvents) -> bool {
    matches!(event, QRCodeLoginEvents::Done | QRCodeLoginEvents::Refuse | QRCodeLoginEvents::Timeout)
}

async fn sleep_until(deadline: &Instant, delay: f64) {
    let timeout_left = deadline.saturating_duration_since(Instant::now());
    if timeout_left.is_zero() {
        return;
    }
    let sleep = Duration::from_secs_f64(delay.min(timeout_left.as_secs_f64()));
    tokio::time::sleep(sleep).await;
}
