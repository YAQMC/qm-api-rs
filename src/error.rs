//! 错误类型定义.

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

    /// CGI 子请求错误 (`code != 0` 且不在允许集合中).
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
