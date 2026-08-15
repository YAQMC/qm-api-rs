//! 登录流程工具 (对应 Python 端 `modules/login_utils.py`).

use std::time::{Duration, Instant};

use super::login::LoginApi;
use crate::error::{QmError, Result};
use crate::models::login::{
    PhoneAuthCodeResult, QRCodeLoginEvents, QRLoginResult, QRLoginType, QR,
};
use crate::models::Credential;

/// 指数退避上限 (2^6 = 64), 避免 `1u32 << retries` 在 32 次后溢出/panic.
const MAX_BACKOFF_EXP: u32 = 6;
const MAX_DURATION_SECS: f64 = 86_400.0 * 7.0;

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
        self.api
            .phone_authorize(&self.phone, self.is_encrypted, auth_code)
            .await
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
///
/// - [`QRCodeLoginSession::next_event`] 每次返回一个新事件, 供 GUI 实时更新;
/// - [`QRCodeLoginSession::iter_events`] 是便利 API, 收集直到终端状态后一次性返回.
#[derive(Debug, Clone)]
pub struct QRCodeLoginSession {
    pub api: LoginApi,
    pub login_type: QRLoginType,
    pub interval: PollInterval,
    pub timeout_seconds: f64,
    pub emit_repeat: bool,
    pub qrcode: Option<QR>,
    deadline: Option<Instant>,
    last_event: Option<QRCodeLoginEvents>,
    error_retries: u32,
    need_interval: bool,
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
            deadline: None,
            last_event: None,
            error_retries: 0,
            need_interval: false,
        }
    }

    pub fn with_interval(mut self, interval: PollInterval) -> Self {
        self.interval = interval;
        self
    }

    /// 设置总超时 (秒). 非法值不会 panic, 但会在开始轮询时返回 `ValueError`.
    ///
    /// 需要立即校验时请使用 [`QRCodeLoginSession::try_with_timeout`].
    pub fn with_timeout(mut self, timeout_seconds: f64) -> Self {
        self.timeout_seconds = timeout_seconds;
        self
    }

    /// 设置总超时 (秒). `0` / 负数 / NaN / Inf 返回错误, 不 panic.
    pub fn try_with_timeout(self, timeout_seconds: f64) -> Result<Self> {
        validate_positive_secs(timeout_seconds, "timeout_seconds")?;
        Ok(self.with_timeout(timeout_seconds))
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
        self.qrcode
            .clone()
            .ok_or_else(|| QmError::ApiData("二维码未就绪".into()))
    }

    fn reset_poll_state(&mut self) {
        self.deadline = None;
        self.last_event = None;
        self.error_retries = 0;
        self.need_interval = false;
    }

    fn ensure_deadline(&mut self) -> Result<Instant> {
        validate_positive_secs(self.timeout_seconds, "timeout_seconds")?;
        if self.deadline.is_none() {
            self.deadline = Some(
                Instant::now() + duration_from_secs_f64_checked(self.timeout_seconds, "timeout")?,
            );
        }
        Ok(self.deadline.unwrap())
    }

    /// 轮询下一个二维码事件 (QQ / 微信 HTTP 轮询).
    ///
    /// 每次调用最多产出一个新事件, GUI 可据此实时更新
    /// `Scan → Conf → Done/Refuse/Timeout`.
    ///
    /// 手机客户端 MQTT 登录请使用 [`QRCodeLoginSession::wait_qrcode_login`]
    /// 或 `LoginApi::checking_mobile_qrcode` (需要维持长连接).
    pub async fn next_event(&mut self) -> Result<QRLoginResult> {
        let qrcode = self.get_qrcode().await?;
        if qrcode.qr_type == QRLoginType::Mobile {
            return Err(QmError::ValueError(
                "手机客户端二维码请使用 wait_qrcode_login 或 checking_mobile_qrcode".into(),
            ));
        }
        let deadline = self.ensure_deadline()?;
        loop {
            if self.need_interval {
                let delay = match self.last_event {
                    Some(QRCodeLoginEvents::Conf) => self.interval.scanned_interval(),
                    _ => self.interval.default,
                };
                sleep_until(&deadline, delay).await;
                self.need_interval = false;
            }

            let timeout_left = deadline.saturating_duration_since(Instant::now());
            if timeout_left.is_zero() {
                return Ok(timeout_event());
            }

            let result = tokio::time::timeout(timeout_left, self.api.check_qrcode(&qrcode)).await;
            match result {
                Err(_) => return Ok(timeout_event()),
                Ok(Err(e)) if e.is_retryable() => {
                    let backoff = error_backoff_secs(self.error_retries, &self.interval);
                    self.error_retries = self.error_retries.saturating_add(1);
                    sleep_until(&deadline, backoff).await;
                    continue;
                }
                Ok(Err(e)) => return Err(e),
                Ok(Ok(item)) => {
                    self.error_retries = 0;
                    if !self.emit_repeat && self.last_event == Some(item.event) {
                        if is_terminal(&item.event) {
                            return Ok(item);
                        }
                        self.need_interval = true;
                        continue;
                    }
                    self.last_event = Some(item.event);
                    if !is_terminal(&item.event) {
                        self.need_interval = true;
                    }
                    return Ok(item);
                }
            }
        }
    }

    /// 轮询二维码状态直到终端事件, 返回已收集的事件列表.
    ///
    /// 这是便利 API, **不是**实时流: 会等到登录结束 (Done / Refuse / Timeout)
    /// 后一次性返回. GUI 实时更新请循环调用 [`QRCodeLoginSession::next_event`].
    ///
    /// 手机客户端二维码走 MQTT 推送; QQ/微信走 HTTP 轮询.
    /// 不会产出与上一次相同的事件 (除非设置了 `emit_repeat`).
    pub async fn iter_events(&mut self) -> Result<Vec<QRLoginResult>> {
        self.reset_poll_state();
        let qrcode = self.get_qrcode().await?;

        if qrcode.qr_type == QRLoginType::Mobile {
            validate_positive_secs(self.timeout_seconds, "timeout_seconds")?;
            let timeout = duration_from_secs_f64_checked(self.timeout_seconds, "timeout")?;
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

        let mut events = Vec::new();
        loop {
            let item = self.next_event().await?;
            let terminal = is_terminal(&item.event);
            events.push(item);
            if terminal {
                return Ok(events);
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
                    return result.credential.ok_or_else(|| QmError::Login {
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
    matches!(
        event,
        QRCodeLoginEvents::Done | QRCodeLoginEvents::Refuse | QRCodeLoginEvents::Timeout
    )
}

fn timeout_event() -> QRLoginResult {
    QRLoginResult {
        event: QRCodeLoginEvents::Timeout,
        credential: None,
    }
}

fn validate_positive_secs(v: f64, name: &str) -> Result<f64> {
    if v.is_finite() && v > 0.0 && v <= MAX_DURATION_SECS {
        Ok(v)
    } else {
        Err(QmError::ValueError(format!(
            "{name} 必须是有限正数 (秒), 得到 {v}"
        )))
    }
}

fn duration_from_secs_f64_checked(v: f64, name: &str) -> Result<Duration> {
    let v = validate_positive_secs(v, name)?;
    Ok(Duration::from_secs_f64(v))
}

/// 有界指数退避 (秒). `error_retries` 任意大也不会 shift overflow.
pub(crate) fn error_backoff_secs(error_retries: u32, interval: &PollInterval) -> f64 {
    let exponent = error_retries.min(MAX_BACKOFF_EXP);
    let multiplier = 2f64.powi(exponent as i32);
    let base = if interval.default.is_finite() && interval.default > 0.0 {
        interval.default
    } else {
        1.5
    };
    let cap = {
        let e = interval.error_interval();
        if e.is_finite() && e > 0.0 {
            e
        } else {
            base * 2.0
        }
    };
    let v = (multiplier * base).min(cap);
    if v.is_finite() && v > 0.0 {
        v.min(86_400.0)
    } else {
        base
    }
}

async fn sleep_until(deadline: &Instant, delay: f64) {
    let timeout_left = deadline.saturating_duration_since(Instant::now());
    if timeout_left.is_zero() {
        return;
    }
    let Some(sleep) = duration_from_secs_clamped(delay) else {
        return;
    };
    let sleep = sleep.min(timeout_left);
    tokio::time::sleep(sleep).await;
}

fn duration_from_secs_clamped(secs: f64) -> Option<Duration> {
    if !secs.is_finite() || secs <= 0.0 {
        return None;
    }
    Some(Duration::from_secs_f64(secs.min(86_400.0)))
}

/// HTTP 二维码轮询内核 (供单元测试注入 check, 不打真实 ptlogin).
#[cfg(test)]
async fn poll_http_qr_events<F, Fut>(
    interval: PollInterval,
    timeout_seconds: f64,
    emit_repeat: bool,
    mut check: F,
) -> Result<Vec<QRLoginResult>>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<QRLoginResult>>,
{
    validate_positive_secs(timeout_seconds, "timeout_seconds")?;
    let deadline = Instant::now() + duration_from_secs_f64_checked(timeout_seconds, "timeout")?;
    let mut last_event: Option<QRCodeLoginEvents> = None;
    let mut events = Vec::new();
    let mut error_retries: u32 = 0;

    loop {
        let loop_start = Instant::now();
        let timeout_left = deadline.saturating_duration_since(Instant::now());
        if timeout_left.is_zero() {
            events.push(timeout_event());
            return Ok(events);
        }

        let result = tokio::time::timeout(timeout_left, check()).await;
        match result {
            Err(_) => {
                events.push(timeout_event());
                return Ok(events);
            }
            Ok(Err(e)) if e.is_retryable() => {
                let backoff = error_backoff_secs(error_retries, &interval);
                error_retries = error_retries.saturating_add(1);
                sleep_until(&deadline, backoff).await;
                continue;
            }
            Ok(Err(e)) => return Err(e),
            Ok(Ok(item)) => {
                error_retries = 0;
                if !emit_repeat && last_event == Some(item.event) {
                    if is_terminal(&item.event) {
                        return Ok(events);
                    }
                    let elapsed = loop_start.elapsed().as_secs_f64();
                    sleep_until(
                        &deadline,
                        interval.default.max(0.0) - elapsed.min(interval.default),
                    )
                    .await;
                    continue;
                }
                last_event = Some(item.event);
                events.push(item.clone());
                if is_terminal(&item.event) {
                    return Ok(events);
                }
                let sleep_time = match item.event {
                    QRCodeLoginEvents::Conf => interval.scanned_interval(),
                    _ => interval.default,
                };
                let elapsed = loop_start.elapsed().as_secs_f64();
                sleep_until(
                    &deadline,
                    sleep_time.max(0.0) - elapsed.min(sleep_time.max(0.0)),
                )
                .await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::ApiContext;
    use crate::error::QmError;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    fn dummy_api() -> LoginApi {
        let ctx = ApiContext::new(None, None).unwrap();
        LoginApi::new(Arc::new(ctx))
    }

    #[test]
    fn error_backoff_never_overflows_on_large_retry_count() {
        let interval = PollInterval::default();
        for r in 0..64u32 {
            let v = error_backoff_secs(r, &interval);
            assert!(v.is_finite() && v > 0.0, "retries={r} backoff={v}");
            assert!(v <= interval.error_interval() + f64::EPSILON);
        }
        let huge = error_backoff_secs(u32::MAX, &interval);
        assert!(huge.is_finite() && huge > 0.0);
    }

    #[test]
    fn with_timeout_invalid_inputs_do_not_panic() {
        let session = QRCodeLoginSession::new(dummy_api(), QRLoginType::Qq)
            .with_timeout(f64::NAN)
            .with_timeout(f64::INFINITY)
            .with_timeout(f64::NEG_INFINITY)
            .with_timeout(0.0)
            .with_timeout(-3.5);
        assert!(
            session.timeout_seconds.is_nan()
                || session.timeout_seconds <= 0.0
                || !session.timeout_seconds.is_finite()
        );
    }

    #[test]
    fn try_with_timeout_rejects_non_positive() {
        let api = dummy_api();
        for v in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let err = QRCodeLoginSession::new(api.clone(), QRLoginType::Qq)
                .try_with_timeout(v)
                .unwrap_err();
            assert!(matches!(err, QmError::ValueError(_)), "{v} -> {err:?}");
        }
        QRCodeLoginSession::new(api, QRLoginType::Qq)
            .try_with_timeout(12.0)
            .unwrap();
    }

    #[tokio::test]
    async fn iter_events_rejects_invalid_timeout_without_panic() {
        let mut session = QRCodeLoginSession::new(dummy_api(), QRLoginType::Qq).with_timeout(-1.0);
        session.qrcode = Some(QR {
            data: vec![1, 2, 3],
            qr_type: QRLoginType::Qq,
            mimetype: "image/png".into(),
            identifier: "qrsig".into(),
        });
        let err = session.iter_events().await.unwrap_err();
        assert!(matches!(err, QmError::ValueError(_)));
    }

    #[tokio::test]
    async fn poll_does_not_retry_protocol_errors() {
        let interval = PollInterval {
            default: 0.01,
            scanned: Some(0.01),
            error: Some(0.01),
        };
        let err = poll_http_qr_events(interval, 2.0, false, || async {
            Err(QmError::Protocol {
                stage: "qr",
                message: "malformed".into(),
            })
        })
        .await
        .unwrap_err();
        assert!(matches!(err, QmError::Protocol { stage: "qr", .. }));
    }

    #[tokio::test]
    async fn poll_does_not_retry_auth_errors() {
        let interval = PollInterval {
            default: 0.01,
            scanned: Some(0.01),
            error: Some(0.01),
        };
        let err = poll_http_qr_events(interval, 2.0, false, || async {
            Err(QmError::CredentialExpired("expired".into()))
        })
        .await
        .unwrap_err();
        assert!(matches!(err, QmError::CredentialExpired(_)));
    }

    #[tokio::test]
    async fn poll_retries_retryable_then_succeeds() {
        let interval = PollInterval {
            default: 0.01,
            scanned: Some(0.01),
            error: Some(0.05),
        };
        let n = Arc::new(AtomicU32::new(0));
        let events = poll_http_qr_events(interval, 5.0, false, || {
            let n = n.clone();
            async move {
                let i = n.fetch_add(1, Ordering::SeqCst);
                if i < 2 {
                    Err(QmError::network("flaky"))
                } else {
                    Ok(QRLoginResult {
                        event: QRCodeLoginEvents::Done,
                        credential: Some(Credential {
                            musicid: 1,
                            musickey: "k".into(),
                            ..Default::default()
                        }),
                    })
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, QRCodeLoginEvents::Done);
        assert!(n.load(Ordering::SeqCst) >= 3);
    }

    #[tokio::test]
    async fn poll_overall_deadline_is_not_extended_by_retries() {
        let interval = PollInterval {
            default: 0.05,
            scanned: Some(0.05),
            error: Some(0.05),
        };
        let start = Instant::now();
        let events = poll_http_qr_events(interval, 0.2, false, || async {
            Err(QmError::network("always"))
        })
        .await
        .unwrap();
        let elapsed = start.elapsed();
        assert_eq!(
            events.last().map(|e| e.event),
            Some(QRCodeLoginEvents::Timeout)
        );
        assert!(
            elapsed < Duration::from_millis(1500),
            "overall deadline must bound retries, elapsed={elapsed:?}"
        );
    }
}
