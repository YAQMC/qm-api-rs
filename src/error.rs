//! 错误类型定义.

/// 错误大类 (供宿主做展示 / 重试 / 鉴权失效处理).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCategory {
    /// 网络 / 连接 / 超时.
    Network,
    /// 凭证无效或过期.
    Auth,
    /// 权限不足 (需要登录等).
    Permission,
    /// 请求参数 / 数据错误.
    BadRequest,
    /// 服务端限流.
    RateLimit,
    /// 资源不存在.
    NotFound,
    /// 服务端错误.
    Server,
    /// 其他.
    Other,
}

/// 敏感键名 (JSON 键 / form·query 键, 不含 `=`).
const SENSITIVE_KEYS: [&str; 8] = [
    "qm_keyst",
    "musickey",
    "access_token",
    "refresh_token",
    "refresh_key",
    "p_skey",
    "qrsig",
    "uin",
];

/// 递归对 JSON 值做 key-based 脱敏: 敏感键的值替换为 `[redacted]`.
fn redact_json_value(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(map) => {
            for (k, val) in map.iter_mut() {
                if SENSITIVE_KEYS.iter().any(|s| k.eq_ignore_ascii_case(s)) {
                    *val = serde_json::Value::String("[redacted]".into());
                } else {
                    redact_json_value(val);
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr.iter_mut() {
                redact_json_value(item);
            }
        }
        _ => {}
    }
}

/// 截断长文本 (保留尾部标记).
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        let mut truncated: String = s.chars().take(max).collect();
        truncated.push_str("…[truncated]");
        truncated
    } else {
        s.to_string()
    }
}

/// 敏感载荷脱敏 (供错误诊断使用, 避免完整响应带进日志).
///
/// 两套策略:
/// - JSON (对象 / 数组) → 递归 key-based 脱敏 (覆盖 `"musickey":"xxx"` 等);
/// - 其他 (URL / form) → `key=value` 键值对脱敏 (覆盖 `qm_keyst=xxx` 等).
///
/// 结果统一截断到 `max` 字符.
pub(crate) fn redact_payload(s: &str, max: usize) -> String {
    if let Ok(mut v) = serde_json::from_str::<serde_json::Value>(s) {
        redact_json_value(&mut v);
        return truncate(&v.to_string(), max);
    }

    // form / query 键值对脱敏.
    let mut out = s.to_string();
    for key in SENSITIVE_KEYS {
        let marker = format!("{key}=");
        let mut start = 0;
        while let Some(idx) = out[start..].find(&marker) {
            let val_start = start + idx + marker.len();
            let end = out[val_start..]
                .find(|c: char| c.is_whitespace() || c == ';' || c == '&' || c == '"')
                .map(|e| val_start + e)
                .unwrap_or(out.len());
            out.replace_range(val_start..end, "[redacted]");
            start = val_start;
        }
    }
    truncate(&out, max)
}

/// QQ 音乐 API 统一错误类型.
#[derive(Debug, thiserror::Error)]
pub enum QmError {
    /// 网络层错误 (连接失败、超时等).
    #[error("network error: {0}")]
    Network(String),

    /// HTTP 状态码异常.
    #[error("http error: status {status}, body: {body}")]
    Http { status: u16, body: String },

    /// CGI 全局信封错误 (`req_0` 外层 `code != 0`).
    #[error("global api error: code {code}, data: {data}")]
    GlobalApi { code: i64, data: String },

    /// CGI 子请求错误 (`code != 0`).
    #[error("cgi api error: code {code}, data: {data}")]
    CgiApi { code: i64, data: String },

    /// 需要签名但未提供签名.
    #[error("signature required (code 2000)")]
    SignatureRequired,

    /// 请求被限流.
    #[error("rate limited (code 2001)")]
    RateLimited,

    /// 登录凭证无效或过期.
    #[error("credential expired: {0}")]
    CredentialExpired(String),

    /// 需要登录但未提供有效凭证.
    #[error("credential invalid: {0}")]
    CredentialInvalid(String),

    /// 登录业务错误.
    #[error("login error: {message} (code {code})")]
    Login { message: String, code: i64 },

    /// 凭证刷新失败.
    #[error("credential refresh failed: {0}")]
    CredentialRefresh(String),

    /// 响应 JSON 反序列化失败.
    #[error("deserialize error: {0}")]
    Deserialize(String),

    /// 响应内容解析失败.
    #[error("api data error: {0}")]
    ApiData(String),

    /// JSONPath 提取失败.
    #[error("jsonpath error: {0}")]
    JsonPath(String),

    /// 参数校验错误.
    #[error("value error: {0}")]
    ValueError(String),

    /// I/O 错误.
    #[error("io error: {0}")]
    Io(String),

    /// 其他错误.
    #[error("{0}")]
    Other(String),
}

impl QmError {
    /// 构造 HTTP 错误 (响应体自动脱敏, 避免敏感载荷进日志).
    pub(crate) fn http(status: u16, body: String) -> Self {
        QmError::Http {
            status,
            body: redact_payload(&body, 400),
        }
    }

    /// 错误大类 (供展示 / 重试策略参考).
    pub fn category(&self) -> ErrorCategory {
        match self {
            QmError::Network(_) => ErrorCategory::Network,
            QmError::Http { status, .. } if *status >= 500 => ErrorCategory::Server,
            QmError::Http { .. } => ErrorCategory::BadRequest,
            QmError::GlobalApi { .. } => ErrorCategory::Server,
            QmError::CgiApi { code, .. } => classify_cgi_code(*code),
            QmError::SignatureRequired => ErrorCategory::BadRequest,
            QmError::RateLimited => ErrorCategory::RateLimit,
            QmError::CredentialExpired(_) => ErrorCategory::Auth,
            QmError::CredentialInvalid(_) => ErrorCategory::Auth,
            QmError::Login { .. } => ErrorCategory::Auth,
            QmError::CredentialRefresh(_) => ErrorCategory::Auth,
            QmError::Deserialize(_) => ErrorCategory::BadRequest,
            QmError::ApiData(_) => ErrorCategory::Server,
            QmError::JsonPath(_) => ErrorCategory::BadRequest,
            QmError::ValueError(_) => ErrorCategory::BadRequest,
            QmError::Io(_) => ErrorCategory::Other,
            QmError::Other(_) => ErrorCategory::Other,
        }
    }

    /// 该错误是否可安全重试 (网络抖动 / 限流 / 服务端 5xx).
    pub fn is_retryable(&self) -> bool {
        match self {
            QmError::Network(_) => true,
            QmError::RateLimited => true,
            QmError::Http { status, .. } => *status == 429 || *status >= 500,
            QmError::CgiApi { code, .. } => *code == 2001 || *code == 104604,
            QmError::GlobalApi { .. } => true,
            _ => false,
        }
    }
}

fn classify_cgi_code(code: i64) -> ErrorCategory {
    match code {
        2000 => ErrorCategory::BadRequest,
        2001 | 104604 => ErrorCategory::RateLimit,
        1000 | 104401 | 104400 => ErrorCategory::Auth,
        104003 => ErrorCategory::Permission,
        20261 | 20271 | 20272 | 20274 => ErrorCategory::BadRequest,
        10007 => ErrorCategory::NotFound,
        _ => ErrorCategory::Other,
    }
}

impl From<reqwest::Error> for QmError {
    fn from(e: reqwest::Error) -> Self {
        QmError::Network(e.to_string())
    }
}

impl From<serde_json::Error> for QmError {
    fn from(e: serde_json::Error) -> Self {
        QmError::Deserialize(e.to_string())
    }
}

impl From<std::io::Error> for QmError {
    fn from(e: std::io::Error) -> Self {
        QmError::Io(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, QmError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_form_style_tokens() {
        let payload = r#"qm_keyst=SECRETKEY&uin=123&foo=bar; musickey=SECRET2"#;
        let out = redact_payload(payload, 10_000);
        assert!(!out.contains("SECRETKEY"));
        assert!(!out.contains("SECRET2"));
        assert!(!out.contains("uin=123"), "uin 的值必须被掩码");
        assert!(out.contains("uin=[redacted]"));
        assert!(out.contains("foo=bar"));
    }

    #[test]
    fn redacts_json_values_by_key() {
        let payload = r#"{"ret":0,"musickey":"SECRET1","data":{"user":{"uin":"12345","name":"x"},"access_token":"SECRET2"}}"#;
        let out = redact_payload(payload, 10_000);
        assert!(!out.contains("SECRET1"));
        assert!(!out.contains("SECRET2"));
        assert!(!out.contains("12345"));
        assert!(!out.contains("\"musickey\":\"SECRET1\""));
        assert!(out.contains("name"), "非敏感字段应保留");
        assert!(out.contains("redacted"));
    }

    #[test]
    fn truncates_long_payload() {
        let payload = "a".repeat(2000);
        let out = redact_payload(&payload, 300);
        assert!(out.len() < 400);
        assert!(out.contains("truncated"));
    }

    #[test]
    fn categories_and_retryable() {
        assert_eq!(QmError::Network("x".into()).category(), ErrorCategory::Network);
        assert_eq!(QmError::RateLimited.category(), ErrorCategory::RateLimit);
        assert_eq!(QmError::CgiApi { code: 1000, data: String::new() }.category(), ErrorCategory::Auth);
        assert_eq!(
            QmError::Http { status: 500, body: String::new() }.category(),
            ErrorCategory::Server
        );

        assert!(QmError::Network("x".into()).is_retryable());
        assert!(QmError::RateLimited.is_retryable());
        assert!(QmError::CgiApi { code: 104604, data: String::new() }.is_retryable());
        assert!(!QmError::CredentialExpired("x".into()).is_retryable());
        assert!(!QmError::ValueError("x".into()).is_retryable());
    }
}
