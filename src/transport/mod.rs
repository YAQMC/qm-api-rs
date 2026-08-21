//! 可注入的 HTTP 传输 (`ApiTransport`).
//!
//! 公开请求/响应类型不暴露 `reqwest`. 默认实现 `ReqwestApiTransport` 使用
//! reqwest **0.12** (cookie_store / gzip / brotli), 仅存在于本模块的私有子模块.
//!
//! ## 取消
//!
//! 每个请求携带 [`CancellationToken`] (`tokio-util::sync::CancellationToken`).
//! 默认 transport 在发送与读取响应体时 `tokio::select!` 该令牌; 取消后返回
//! [`QmError::Network`] 且 `kind = NetworkErrorKind::Cancelled` (不可重试).
//! 未取消的默认令牌 (`CancellationToken::new()`) 不会自行触发.
//!
//! 不要只在调用方包一层 `tokio::time::timeout` 来代替这个机制.

mod default_http;

use std::fmt;
use std::time::Duration;

use crate::error::Result;

pub use default_http::ReqwestApiTransport;
pub use tokio_util::sync::CancellationToken;

/// 库内 HTTP 方法 (不暴露 `reqwest::Method`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Head,
    Delete,
    Patch,
}

/// 请求体.
#[derive(Clone, Default)]
pub enum HttpBody {
    #[default]
    Empty,
    Json(serde_json::Value),
    /// `application/x-www-form-urlencoded`, 对象的键值会被展平为字符串.
    Form(serde_json::Value),
    Bytes(Vec<u8>),
}

impl fmt::Debug for HttpBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HttpBody::Empty => write!(f, "Empty"),
            HttpBody::Json(_) => write!(f, "Json(..)"),
            HttpBody::Form(_) => write!(f, "Form(..)"),
            HttpBody::Bytes(b) => f.debug_tuple("Bytes").field(&b.len()).finish(),
        }
    }
}

/// 传输层重试类别.
///
/// 与 [`crate::QmError::is_retryable`] 对齐: 仅 [`RetryClass::SafeRead`] 会在
/// 网络抖动 / HTTP 429 / 5xx 时按 [`TransportConfig::retry_max`] 重试.
/// 登录写与状态改变必须使用 [`RetryClass::Write`], 默认不重试.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RetryClass {
    /// 幂等读. 可重试网络抖动与 5xx/429.
    #[default]
    SafeRead,
    /// 登录轮询 (含微信长轮询). 超时是有意义的信号, 不自动重试.
    AuthPoll,
    /// 状态改变 / 登录写. 默认不重试.
    Write,
}

/// 重定向策略.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RedirectMode {
    /// 校验 allowlist 后跟随, 最多 [`TransportConfig::max_redirects`] 跳 (默认 3).
    #[default]
    FollowValidated,
    /// 不跟随, 把 30x 当作最终响应返回 (二维码 / cookie 交换).
    None,
}

/// 默认 transport 的可配置项.
#[derive(Debug, Clone)]
pub struct TransportConfig {
    /// 连接超时. 默认 5s.
    pub connect_timeout: Duration,
    /// 单次请求总超时 (可被请求级 `timeout` 覆盖). 默认 15s.
    pub total_timeout: Duration,
    /// [`RedirectMode::FollowValidated`] 最多跟随跳数. 默认 3.
    pub max_redirects: usize,
    /// [`RetryClass::SafeRead`] 的额外尝试次数 (不含首次). 默认 1.
    pub retry_max: u32,
    /// 重试间隔. 默认 250ms.
    pub retry_delay: Duration,
    /// HTTP 代理, 如 `http://127.0.0.1:7890`.
    pub proxy: Option<String>,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(5),
            total_timeout: Duration::from_secs(15),
            max_redirects: 3,
            retry_max: 1,
            retry_delay: Duration::from_millis(250),
            proxy: None,
        }
    }
}

/// 一次 HTTP 请求 (库内类型, 不含 reqwest).
pub struct TransportRequest {
    pub method: HttpMethod,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub query: Vec<(String, String)>,
    pub body: HttpBody,
    /// 覆盖 [`TransportConfig::total_timeout`]; `None` 使用配置默认值.
    pub timeout: Option<Duration>,
    pub retry: RetryClass,
    pub redirects: RedirectMode,
    /// 见模块文档: `tokio-util::sync::CancellationToken`.
    pub cancellation: CancellationToken,
}

impl TransportRequest {
    pub fn new(method: HttpMethod, url: impl Into<String>) -> Self {
        Self {
            method,
            url: url.into(),
            headers: Vec::new(),
            query: Vec::new(),
            body: HttpBody::Empty,
            timeout: None,
            retry: RetryClass::SafeRead,
            redirects: RedirectMode::FollowValidated,
            cancellation: CancellationToken::new(),
        }
    }
}

impl fmt::Debug for TransportRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let header_names: Vec<&str> = self.headers.iter().map(|(k, _)| k.as_str()).collect();
        f.debug_struct("TransportRequest")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("header_names", &header_names)
            .field(
                "query_keys",
                &self.query.iter().map(|(k, _)| k).collect::<Vec<_>>(),
            )
            .field("body", &self.body)
            .field("timeout", &self.timeout)
            .field("retry", &self.retry)
            .field("redirects", &self.redirects)
            .finish_non_exhaustive()
    }
}

/// 一次 HTTP 响应 (库内类型, 不含 `reqwest::Response`).
#[derive(Clone)]
pub struct TransportResponse {
    pub status: u16,
    pub final_url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl fmt::Debug for TransportResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let header_names: Vec<&str> = self.headers.iter().map(|(k, _)| k.as_str()).collect();
        f.debug_struct("TransportResponse")
            .field("status", &self.status)
            .field("final_url", &self.final_url)
            .field("header_names", &header_names)
            .field("body_len", &self.body.len())
            .finish()
    }
}

impl TransportResponse {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// 可注入的 HTTP 传输.
///
/// `Client::new` / `ApiContext::new` 使用 [`ReqwestApiTransport`].
/// 下游可通过 `new_with_transport` 注入 `Arc<dyn ApiTransport>`.
#[async_trait::async_trait]
pub trait ApiTransport: Send + Sync {
    async fn execute(&self, request: TransportRequest) -> Result<TransportResponse>;

    /// 放行额外 origin (完整 URL 或 `scheme://host[:port]`).
    ///
    /// 默认 transport 用它放行测试里指向 mock 的 `cgi_base_url` / `qimei_url`.
    /// 自定义实现可忽略.
    fn allow_origin(&self, origin: &str) {
        let _ = origin;
    }
}

/// 生产 HTTPS 主机 (精确匹配).
const PRODUCTION_HOSTS: &[&str] = &[
    "u.y.qq.com",
    "c.y.qq.com",
    "c6.y.qq.com",
    "api.tencentmusic.com",
    "ssl.ptlogin2.qq.com",
    "ssl.ptlogin2.graph.qq.com",
    "xui.ptlogin2.qq.com",
    "graph.qq.com",
    "y.qq.com",
    "open.weixin.qq.com",
    "lp.open.weixin.qq.com",
];

pub(crate) fn origin_of(url: &url::Url) -> String {
    let host = match url.host_str() {
        Some(h) if h.contains(':') => format!("[{h}]"),
        Some(h) => h.to_string(),
        None => String::new(),
    };
    match url.port() {
        Some(p) => format!("{}://{}:{p}", url.scheme(), host),
        None => format!("{}://{}", url.scheme(), host),
    }
}

pub(crate) fn parse_origin_input(s: &str) -> Option<String> {
    let url = url::Url::parse(s).ok()?;
    url.host_str()?;
    Some(origin_of(&url))
}

pub(crate) fn is_allowed_host(host: &str) -> bool {
    if PRODUCTION_HOSTS.contains(&host) {
        return true;
    }
    host.ends_with(".stream.qqmusic.qq.com")
        || host.ends_with(".music.tc.qq.com")
        || host.ends_with(".gtimg.cn")
        || (host.ends_with(".myqcloud.com") && host.contains(".cos."))
}

pub(crate) fn validate_url(url: &url::Url, extra_origins: &[String]) -> Result<()> {
    if !url.username().is_empty() || url.password().is_some() {
        return Err(crate::QmError::allowlist_denied("<userinfo>"));
    }
    let host = url.host_str().unwrap_or("");
    if host.is_empty() {
        return Err(crate::QmError::allowlist_denied("<missing-host>"));
    }
    if url.scheme() == "https" && is_allowed_host(host) {
        return Ok(());
    }
    let origin = origin_of(url);
    if extra_origins.iter().any(|e| e == &origin) {
        return Ok(());
    }
    Err(crate::QmError::allowlist_denied(host))
}

pub(crate) fn form_pairs(value: &serde_json::Value) -> Vec<(String, String)> {
    match value {
        serde_json::Value::Object(map) => map
            .iter()
            .map(|(k, v)| {
                let val = match v {
                    serde_json::Value::String(s) => s.clone(),
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::Bool(b) => b.to_string(),
                    other => other.to_string(),
                };
                (k.clone(), val)
            })
            .collect(),
        _ => Vec::new(),
    }
}

pub(crate) fn is_redirect_status(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

pub(crate) fn redirects_as_get(status: u16, method: HttpMethod) -> bool {
    matches!(status, 301..=303) && !matches!(method, HttpMethod::Get | HttpMethod::Head)
}

pub(crate) fn same_origin(a: &url::Url, b: &url::Url) -> bool {
    a.scheme() == b.scheme()
        && a.host() == b.host()
        && a.port_or_known_default() == b.port_or_known_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{NetworkErrorKind, QmError};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use tokio::time::Duration as TokioDuration;

    async fn spawn_router(app: axum::Router) -> (String, std::net::SocketAddr) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{addr}"), addr)
    }

    fn transport_for(base: &str, config: TransportConfig) -> ReqwestApiTransport {
        let t = ReqwestApiTransport::new(config).unwrap();
        t.allow_origin(base);
        t
    }

    #[tokio::test]
    async fn mock_base_url_is_allowed() {
        use axum::routing::get;
        let (base, _) =
            spawn_router(axum::Router::new().route("/ok", get(|| async { "hi" }))).await;
        let t = transport_for(&base, TransportConfig::default());
        let resp = t
            .execute(TransportRequest::new(HttpMethod::Get, format!("{base}/ok")))
            .await
            .unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(resp.text(), "hi");
    }

    #[tokio::test]
    async fn unknown_host_is_rejected() {
        let t = ReqwestApiTransport::new(TransportConfig::default()).unwrap();
        let err = t
            .execute(TransportRequest::new(
                HttpMethod::Get,
                "https://evil.example/nope",
            ))
            .await
            .unwrap_err();
        match err {
            QmError::Protocol { stage, message } => {
                assert_eq!(stage, "allowlist");
                assert!(message.contains("evil.example"));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[tokio::test]
    async fn timeout_is_enforced() {
        use axum::routing::get;
        let (base, _) = spawn_router(axum::Router::new().route(
            "/slow",
            get(|| async {
                tokio::time::sleep(TokioDuration::from_secs(2)).await;
                "late"
            }),
        ))
        .await;
        let mut config = TransportConfig::default();
        config.total_timeout = TokioDuration::from_millis(200);
        config.connect_timeout = TokioDuration::from_millis(200);
        let t = transport_for(&base, config);
        let err = t
            .execute(TransportRequest::new(
                HttpMethod::Get,
                format!("{base}/slow"),
            ))
            .await
            .unwrap_err();
        match err {
            QmError::Network(n) => assert_eq!(n.kind, NetworkErrorKind::Timeout),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[tokio::test]
    async fn redirect_none_returns_30x() {
        use axum::http::header::LOCATION;
        use axum::http::StatusCode;
        use axum::response::IntoResponse;
        use axum::routing::get;
        let (base, _) = spawn_router(
            axum::Router::new()
                .route(
                    "/from",
                    get(|| async { (StatusCode::FOUND, [(LOCATION, "/to")]).into_response() }),
                )
                .route("/to", get(|| async { "landed" })),
        )
        .await;
        let t = transport_for(&base, TransportConfig::default());
        let mut req = TransportRequest::new(HttpMethod::Get, format!("{base}/from"));
        req.redirects = RedirectMode::None;
        let resp = t.execute(req).await.unwrap();
        assert_eq!(resp.status, 302);
        assert_ne!(resp.text(), "landed");
    }

    #[tokio::test]
    async fn redirect_follow_three_hops() {
        use axum::http::header::LOCATION;
        use axum::http::StatusCode;
        use axum::response::IntoResponse;
        use axum::routing::get;
        let (base, _) = spawn_router(
            axum::Router::new()
                .route(
                    "/0",
                    get(|| async { (StatusCode::FOUND, [(LOCATION, "/1")]).into_response() }),
                )
                .route(
                    "/1",
                    get(|| async { (StatusCode::FOUND, [(LOCATION, "/2")]).into_response() }),
                )
                .route(
                    "/2",
                    get(|| async { (StatusCode::FOUND, [(LOCATION, "/3")]).into_response() }),
                )
                .route("/3", get(|| async { "done" })),
        )
        .await;
        let t = transport_for(&base, TransportConfig::default());
        let resp = t
            .execute(TransportRequest::new(HttpMethod::Get, format!("{base}/0")))
            .await
            .unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(resp.text(), "done");
        assert!(resp.final_url.ends_with("/3"));
    }

    #[tokio::test]
    async fn redirect_four_hops_fails() {
        use axum::http::header::LOCATION;
        use axum::http::StatusCode;
        use axum::response::IntoResponse;
        use axum::routing::get;
        let (base, _) = spawn_router(
            axum::Router::new()
                .route(
                    "/0",
                    get(|| async { (StatusCode::FOUND, [(LOCATION, "/1")]).into_response() }),
                )
                .route(
                    "/1",
                    get(|| async { (StatusCode::FOUND, [(LOCATION, "/2")]).into_response() }),
                )
                .route(
                    "/2",
                    get(|| async { (StatusCode::FOUND, [(LOCATION, "/3")]).into_response() }),
                )
                .route(
                    "/3",
                    get(|| async { (StatusCode::FOUND, [(LOCATION, "/4")]).into_response() }),
                )
                .route("/4", get(|| async { "too-far" })),
        )
        .await;
        let t = transport_for(&base, TransportConfig::default());
        let err = t
            .execute(TransportRequest::new(HttpMethod::Get, format!("{base}/0")))
            .await
            .unwrap_err();
        match err {
            QmError::Network(n) => assert_eq!(n.kind, NetworkErrorKind::Redirect),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[tokio::test]
    async fn redirect_to_unknown_host_is_rejected() {
        use axum::http::header::LOCATION;
        use axum::http::StatusCode;
        use axum::response::IntoResponse;
        use axum::routing::get;
        let (base, _) = spawn_router(axum::Router::new().route(
            "/from",
            get(|| async {
                (StatusCode::FOUND, [(LOCATION, "https://evil.example/x")]).into_response()
            }),
        ))
        .await;
        let t = transport_for(&base, TransportConfig::default());
        let err = t
            .execute(TransportRequest::new(
                HttpMethod::Get,
                format!("{base}/from"),
            ))
            .await
            .unwrap_err();
        match err {
            QmError::Protocol { stage, .. } => assert_eq!(stage, "allowlist"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[tokio::test]
    async fn cancellation_aborts_in_flight_request() {
        use axum::routing::get;
        let (base, _) = spawn_router(axum::Router::new().route(
            "/hang",
            get(|| async {
                tokio::time::sleep(TokioDuration::from_secs(30)).await;
                "nope"
            }),
        ))
        .await;
        let t = transport_for(&base, TransportConfig::default());
        let token = CancellationToken::new();
        let mut req = TransportRequest::new(HttpMethod::Get, format!("{base}/hang"));
        req.cancellation = token.clone();
        let handle = tokio::spawn(async move { t.execute(req).await });
        tokio::time::sleep(TokioDuration::from_millis(50)).await;
        token.cancel();
        let err = handle.await.unwrap().unwrap_err();
        match err {
            QmError::Network(n) => assert_eq!(n.kind, NetworkErrorKind::Cancelled),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[tokio::test]
    async fn write_requests_are_not_retried() {
        use axum::http::StatusCode;
        use axum::routing::get;
        async fn run(retry: RetryClass) -> u32 {
            let hits = Arc::new(AtomicU32::new(0));
            let hits2 = hits.clone();
            let (base, _) = spawn_router(axum::Router::new().route(
                "/flaky",
                get(move || {
                    let hits2 = hits2.clone();
                    async move {
                        hits2.fetch_add(1, Ordering::SeqCst);
                        StatusCode::INTERNAL_SERVER_ERROR
                    }
                }),
            ))
            .await;
            let t = transport_for(&base, TransportConfig::default());
            let mut req = TransportRequest::new(HttpMethod::Get, format!("{base}/flaky"));
            req.retry = retry;
            let _ = t.execute(req).await;
            hits.load(Ordering::SeqCst)
        }
        assert_eq!(run(RetryClass::SafeRead).await, 2);
        assert_eq!(run(RetryClass::Write).await, 1);
        assert_eq!(run(RetryClass::AuthPoll).await, 1);
    }

    #[test]
    fn production_hosts_are_listed() {
        assert!(is_allowed_host("u.y.qq.com"));
        assert!(is_allowed_host("api.tencentmusic.com"));
        assert!(is_allowed_host("lp.open.weixin.qq.com"));
        assert!(is_allowed_host("isure.stream.qqmusic.qq.com"));
        assert!(is_allowed_host("bucket.cos.ap-guangzhou.myqcloud.com"));
        assert!(!is_allowed_host("evil.example"));
    }
}
