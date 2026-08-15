//! API 请求上下文 (对应 Python 端 `core/api_context.py`).

use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::{json, Value};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::device::Device;
use crate::error::{QmError, Result};
use crate::qimei;
use crate::rate_limiter::TokenBucket;
use crate::reply::CgiReply;
use crate::sign::zzc_sign;
use crate::versioning::{Platform, VersionPolicy};
use crate::Credential;

/// CGI 请求选项 (轻量拷贝, 与 `crate::client::CgiOptions` 同构).
#[derive(Debug, Clone)]
pub struct RequestOptions {
    pub comm: Option<Value>,
    pub override_comm: bool,
    pub preserve_bool: bool,
    pub credential: Option<Credential>,
    pub platform: Option<Platform>,
    pub sign: bool,
    pub require_login: bool,
}

impl Default for RequestOptions {
    fn default() -> Self {
        RequestOptions {
            comm: None,
            override_comm: false,
            preserve_bool: false,
            credential: None,
            platform: None,
            sign: false,
            require_login: false,
        }
    }
}

/// 请求上下文: 持有 HTTP 客户端、平台、版本策略、凭证与设备状态.
///
/// `Device` 是设备指纹 (含 QIMEI / Android session) 的**唯一状态源**;
/// 运行时获取的新 QIMEI / session 会写回 `Device`, 以便
/// `Client::save_device` / `load_device` 持久化后能跨进程复用.
#[derive(Debug)]
pub struct ApiContext {
    /// 底层 HTTP 客户端.
    pub http: reqwest::Client,
    /// 默认请求平台.
    pub platform: Platform,
    /// 版本策略.
    pub version_policy: VersionPolicy,
    /// CGI 基础地址 (默认 `https://u.y.qq.com/cgi-bin`), 测试时可指向 mock 服务器.
    pub cgi_base_url: String,
    credential: Mutex<Credential>,
    device: Mutex<Device>,
    /// 请求限流器.
    pub limiter: TokenBucket,
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// 将 JSON 值转为字符串 (兼容数字/字符串).
fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

impl ApiContext {
    /// 创建上下文.
    pub fn new(credential: Option<Credential>, platform: Option<Platform>) -> Result<Self> {
        Self::new_with_proxy(credential, platform, None)
    }

    /// 创建上下文 (可指定 HTTP 代理).
    pub fn new_with_proxy(
        credential: Option<Credential>,
        platform: Option<Platform>,
        proxy: Option<&str>,
    ) -> Result<Self> {
        let mut builder = reqwest::Client::builder()
            .gzip(true)
            .brotli(true)
            .cookie_store(true);
        if let Some(p) = proxy {
            let proxy = reqwest::Proxy::all(p).map_err(|e| QmError::Network(e.to_string()))?;
            builder = builder.proxy(proxy);
        }
        let http = builder.build().map_err(|e| QmError::Network(e.to_string()))?;
        Ok(ApiContext {
            http,
            platform: platform.unwrap_or(Platform::Android),
            version_policy: VersionPolicy::default(),
            cgi_base_url: "https://u.y.qq.com/cgi-bin".to_string(),
            credential: Mutex::new(credential.unwrap_or_default()),
            device: Mutex::new(Device::random()),
            limiter: TokenBucket::default(),
        })
    }

    /// 读取当前全局凭证.
    pub fn credential(&self) -> Credential {
        self.credential.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// 设置全局凭证.
    pub fn set_credential(&self, credential: Credential) {
        *self.credential.lock().unwrap() = credential;
    }

    /// 读取设备.
    pub fn device(&self) -> Device {
        self.device.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// 替换设备 (例如从持久化文件加载).
    pub fn set_device(&self, device: Device) {
        *self.device.lock().unwrap() = device;
    }

    /// 读取当前缓存的 QIMEI (从 `Device`, 单一状态源).
    pub fn qimei(&self) -> Option<(String, String)> {
        let device = self.device();
        match (device.qimei, device.qimei36) {
            (Some(q16), Some(q36)) if !q16.is_empty() && !q36.is_empty() => Some((q16, q36)),
            _ => None,
        }
    }

    /// 获取 User-Agent.
    pub fn get_user_agent(&self, platform: Platform) -> String {
        let device = self.device();
        self.version_policy.get_user_agent(platform, &device)
    }

    /// 确保 Android 平台会话有效, 否则申请新的 session.
    pub async fn ensure_session(&self, platform: Platform) -> Result<()> {
        if platform != Platform::Android {
            return Ok(());
        }
        {
            let device = self.device();
            if let (Some(uid), Some(sid)) = (device.session_uid.as_ref(), device.session_sid.as_ref()) {
                let fresh = device
                    .session_save_time
                    .map(|t| now() - t < 86_400)
                    .unwrap_or(false);
                if fresh && !uid.is_empty() && !sid.is_empty() {
                    return Ok(());
                }
            }
        }
        let credential = self.credential();
        let device = self.device();
        let qimei = self.get_cached_qimei().await?;
        let comm = self.version_policy.build_comm(Platform::Android, &credential, &device, qimei.as_ref());
        let payload = json!({
            "comm": comm,
            "req_0": {
                "module": "music.getSession.session",
                "method": "GetSession",
                "param": {
                    "uid": device.session_uid.clone().unwrap_or_default(),
                    "vkey": 0,
                    "caller": 0,
                },
            },
        });
        let user_agent = self.get_user_agent(Platform::Android);
        let resp = self
            .http
            .post("https://u.y.qq.com/cgi-bin/musicu.fcg")
            .json(&payload)
            .header("User-Agent", user_agent)
            .send()
            .await
            .map_err(|e| QmError::Network(e.to_string()))?;
        let status = resp.status().as_u16();
        if status != 200 {
            return Err(QmError::http(status, resp.text().await.unwrap_or_default()));
        }
        let value: Value = resp.json().await?;
        let session_data = &value["req_0"]["data"]["session"];
        let uid = value_to_string(&session_data["uid"]);
        let sid = value_to_string(&session_data["sid"]);
        if uid.is_empty() || sid.is_empty() {
            return Err(QmError::ApiData("获取 session 失败".into()));
        }
        // 写回 Device (单一状态源), 使 save_device 能持久化 session.
        let mut device = self.device.lock().unwrap();
        device.session_uid = Some(uid);
        device.session_sid = Some(sid);
        device.session_save_time = Some(now());
        Ok(())
    }

    /// 获取缓存的 QIMEI, 过期时重新申请.
    ///
    /// 从 `Device` 读取缓存 (过期时间 24 小时); 重新申请成功后写回 `Device`.
    pub async fn get_cached_qimei(&self) -> Result<Option<(String, String)>> {
        let profile = self.version_policy.get_profile(Platform::Android);
        let app_version = profile.qimei_app_version.clone().unwrap_or_else(|| "14.9.0.8".into());
        let sdk_version = profile.qimei_sdk_version.clone().unwrap_or_else(|| "1.2.13.6".into());

        {
            let device = self.device();
            if let (Some(q16), Some(q36)) = (device.qimei.as_ref(), device.qimei36.as_ref()) {
                let fresh = device
                    .qimei_save_time
                    .map(|t| now() - t < 86_400)
                    .unwrap_or(false);
                if fresh && !q16.is_empty() && !q36.is_empty() {
                    return Ok(Some((q16.clone(), q36.clone())));
                }
            }
        }

        let device = self.device();
        let (_, headers, body) = qimei::build_qimei_request(&device, &app_version, &sdk_version);
        let mut header_map = HeaderMap::new();
        for (k, v) in headers {
            if let Ok(v) = HeaderValue::from_str(&v) {
                if let Ok(name) = reqwest::header::HeaderName::from_bytes(k.as_bytes()) {
                    header_map.insert(name, v);
                }
            }
        }
        let resp = self
            .http
            .post("https://api.tencentmusic.com/tme/trpc/proxy")
            .headers(header_map)
            .json(&body)
            .send()
            .await
            .map_err(|e| QmError::Network(e.to_string()))?;
        let text = resp.text().await?;
        if let Some(q) = qimei::parse_qimei_response(&text) {
            // 写回 Device (单一状态源), 使 save_device 能持久化 QIMEI.
            let mut device = self.device.lock().unwrap();
            device.qimei = Some(q.0.clone());
            device.qimei36 = Some(q.1.clone());
            device.qimei_save_time = Some(now());
            return Ok(Some(q));
        }
        Ok(None)
    }

    /// 为 HTTP 请求准备 kwargs (注入 Cookies 与 User-Agent).
    pub fn prepare_http_kwargs(
        &self,
        credential: Option<&Credential>,
        mut headers: HeaderMap,
        mut cookies: Vec<(String, String)>,
    ) -> (HeaderMap, Vec<(String, String)>) {
        let cred = credential.cloned().unwrap_or_else(|| self.credential());
        let str_musicid = cred.str_musicid();
        if !str_musicid.is_empty() {
            cookies.push(("uin".into(), str_musicid.clone()));
            cookies.push(("qqmusic_uin".into(), str_musicid));
        }
        if !cred.musickey.is_empty() {
            cookies.push(("qm_keyst".into(), cred.musickey.clone()));
            cookies.push(("qqmusic_key".into(), cred.musickey));
        }
        if !headers.contains_key("User-Agent") {
            headers.insert("User-Agent", HeaderValue::from_str(&self.get_user_agent(Platform::Web)).unwrap());
        }
        (headers, cookies)
    }

    /// 构建 CGI 请求的 (url, payload, params, headers).
    #[allow(clippy::too_many_arguments)]
    pub async fn build_api_kwargs(
        &self,
        data: &[Value],
        comm: Option<Value>,
        credential: Option<&Credential>,
        platform: Option<Platform>,
        override_comm: bool,
        sign: bool,
    ) -> Result<(String, Value, Vec<(String, String)>, String)> {
        let target_platform = platform.unwrap_or(self.platform);
        if target_platform == Platform::Android {
            self.ensure_session(target_platform).await?;
        }

        let cred = credential.cloned().unwrap_or_else(|| self.credential());
        let device = self.device();

        let final_comm = if override_comm {
            comm.clone().unwrap_or_else(|| json!({}))
        } else {
            let qimei = if target_platform == Platform::Android {
                self.get_cached_qimei().await?
            } else {
                None
            };
            let mut base = self
                .version_policy
                .build_comm(target_platform, &cred, &device, qimei.as_ref());
            if let Some(c) = comm {
                if let Value::Object(map) = c {
                    for (k, v) in map {
                        base[k] = v;
                    }
                }
            }
            base
        };

        let mut payload = json!({ "comm": final_comm });
        for (idx, req) in data.iter().enumerate() {
            payload[format!("req_{idx}")] = req.clone();
        }

        let mut params = Vec::new();
        if sign {
            params.push(("_".to_string(), format!("{}", now() * 1000)));
            let sign_value = zzc_sign(payload.to_string().as_bytes());
            params.push(("sign".to_string(), sign_value));
        }

        let url = if sign {
            format!("{}/musics.fcg", self.cgi_base_url)
        } else {
            format!("{}/musicu.fcg", self.cgi_base_url)
        };
        let user_agent = self.get_user_agent(target_platform);
        Ok((url.to_string(), payload, params, user_agent))
    }

    /// 执行一个 CGI 请求, 返回固定形状的响应 `CgiReply { code, data }`.
    ///
    /// transport 层不解释业务错误码: 无论 `req_0.code` 是否为 0, 均以
    /// `CgiReply` 返回, 由调用方决定如何处理 (参见 `CgiReply::require_success`).
    /// 仅在 HTTP 状态异常或全局信封 (`code != 0`) 时返回错误.
    pub async fn request_cgi(
        &self,
        module: &str,
        method: &str,
        param: Value,
        opts: &RequestOptions,
    ) -> Result<CgiReply<Value>> {
        self.limiter.acquire().await;
        if opts.require_login {
            let cred = opts.credential.clone().unwrap_or_else(|| self.credential());
            if cred.musicid == 0 || cred.musickey.is_empty() {
                return Err(QmError::CredentialInvalid("请求需要登录, 未提供有效的登录凭证".into()));
            }
        }

        let param = if opts.preserve_bool {
            param
        } else {
            crate::utils::bool_to_int(&param)
        };

        let req = json!({ "module": module, "method": method, "param": param });
        let (url, payload, query_params, user_agent) = self
            .build_api_kwargs(
                &[req],
                opts.comm.clone(),
                opts.credential.as_ref(),
                opts.platform,
                opts.override_comm,
                opts.sign,
            )
            .await?;

        let mut request = self
            .http
            .post(&url)
            .json(&payload)
            .header("User-Agent", user_agent);
        for (k, v) in &query_params {
            request = request.query(&[(k, v)]);
        }
        let resp = request.send().await.map_err(|e| QmError::Network(e.to_string()))?;
        let status = resp.status().as_u16();
        let text = resp.text().await.map_err(|e| QmError::Network(e.to_string()))?;
        if status != 200 {
            return Err(QmError::http(status, text));
        }
        parse_cgi_envelope(&text, 0)
    }

    /// 批量执行多个 CGI 请求 (合并为一次 `req_0..req_N` 调用).
    ///
    /// `requests` 为 `(module, method, param)` 三元组列表, 返回与输入顺序一致
    /// 的每个子请求 `CgiReply { code, data }`. 单个子请求的业务错误码不会导致
    /// 整个批量请求失败, 由调用方决定如何处理部分失败.
    pub async fn request_cgi_batch(
        &self,
        requests: &[(&str, &str, Value)],
        opts: &RequestOptions,
    ) -> Result<Vec<CgiReply<Value>>> {
        if requests.is_empty() {
            return Ok(Vec::new());
        }
        self.limiter.acquire().await;
        if opts.require_login {
            let cred = opts.credential.clone().unwrap_or_else(|| self.credential());
            if cred.musicid == 0 || cred.musickey.is_empty() {
                return Err(QmError::CredentialInvalid("请求需要登录, 未提供有效的登录凭证".into()));
            }
        }

        let mut data = Vec::with_capacity(requests.len());
        for (module, method, param) in requests {
            let param = if opts.preserve_bool {
                param.clone()
            } else {
                crate::utils::bool_to_int(param)
            };
            data.push(json!({ "module": module, "method": method, "param": param }));
        }

        let (url, payload, query_params, user_agent) = self
            .build_api_kwargs(
                &data,
                opts.comm.clone(),
                opts.credential.as_ref(),
                opts.platform,
                opts.override_comm,
                opts.sign,
            )
            .await?;

        let mut request = self
            .http
            .post(&url)
            .json(&payload)
            .header("User-Agent", user_agent);
        for (k, v) in &query_params {
            request = request.query(&[(k, v)]);
        }
        let resp = request.send().await.map_err(|e| QmError::Network(e.to_string()))?;
        let status = resp.status().as_u16();
        let text = resp.text().await.map_err(|e| QmError::Network(e.to_string()))?;
        if status != 200 {
            return Err(QmError::http(status, text));
        }
        let mut out = Vec::with_capacity(requests.len());
        for i in 0..requests.len() {
            out.push(parse_cgi_envelope(&text, i)?);
        }
        Ok(out)
    }

        /// 下载原始字节 (用于音频文件下载).
    pub async fn request_http_bytes(&self, url: &str, credential: Option<&Credential>) -> Result<Vec<u8>> {
        self.limiter.acquire().await;
        let (headers, cookies) = self.prepare_http_kwargs(credential, HeaderMap::new(), Vec::new());
        let mut request = self.http.get(url).headers(headers);
        for (k, v) in &cookies {
            request = request.header("Cookie", format!("{k}={v}"));
        }
        let resp = request.send().await.map_err(|e| QmError::Network(e.to_string()))?;
        let status = resp.status().as_u16();
        if status != 200 {
            let text = resp.text().await.unwrap_or_default();
            return Err(QmError::http(status, text));
        }
        let bytes = resp.bytes().await.map_err(|e| QmError::Network(e.to_string()))?;
        Ok(bytes.to_vec())
    }

    /// 执行标准 HTTP 请求, 返回原始响应文本.
    pub async fn request_http(
        &self,
        method: reqwest::Method,
        url: &str,
        opts: &crate::client::HttpOptions,
    ) -> Result<String> {
        self.limiter.acquire().await;
        let (headers, cookies) = self.prepare_http_kwargs(
            opts.credential.as_ref(),
            opts.headers.clone(),
            opts.cookies.clone(),
        );
        let mut request = self.http.request(method, url).headers(headers);
        for (k, v) in &opts.params {
            request = request.query(&[(k, v)]);
        }
        for (k, v) in &cookies {
            request = request.header("Cookie", format!("{k}={v}"));
        }
        if let Some(json) = &opts.json {
            request = request.json(json);
        }
        if let Some(data) = &opts.data {
            request = request.form(&data);
        }
        if let Some(t) = opts.timeout {
            request = request.timeout(t);
        }
        let resp = request.send().await.map_err(|e| QmError::Network(e.to_string()))?;
        let status = resp.status().as_u16();
        let text = resp.text().await.map_err(|e| QmError::Network(e.to_string()))?;
        if status != 200 {
            return Err(QmError::http(status, text));
        }
        Ok(text)
    }
}

/// 解析 CGI 全局信封并提取 `req_{index}` 的固定响应 `{ code, data }`.
///
/// - HTTP 层已确认状态码为 200;
/// - 全局信封 `code != 0` 视为 transport 级错误 (`GlobalApi`);
/// - `req_{index}` 缺失视为 `ApiData` 错误;
/// - 其余情况返回 `CgiReply { code, data }`, 不解释业务错误码.
pub(crate) fn parse_cgi_envelope(text: &str, index: usize) -> Result<CgiReply<Value>> {
    let env: Value = serde_json::from_str(text)?;
    let env_code = env.get("code").and_then(Value::as_i64).unwrap_or(0);
    if env_code != 0 {
        return Err(QmError::GlobalApi {
            code: env_code,
            data: crate::error::redact_payload(text, 400),
        });
    }
    let req0 = env
        .get(&format!("req_{index}"))
        .cloned()
        .ok_or_else(|| QmError::ApiData(format!("CGI 响应缺少 req_{index}")))?;
    let code = req0.get("code").and_then(Value::as_i64).unwrap_or(0);
    let data = req0.get("data").cloned().unwrap_or(Value::Null);
    Ok(CgiReply::new(code, data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_envelope_success() {
        let text = r#"{"code":0,"req_0":{"code":0,"data":{"songmid":"001X3HEN1oK0Jr","name":"晴天"}}}"#;
        let reply = parse_cgi_envelope(text, 0).unwrap();
        assert_eq!(reply.code, 0);
        assert_eq!(reply.data["name"], "晴天");
    }

    #[test]
    fn parse_envelope_preserves_business_error_code() {
        // 登录错误码 20271 必须原样保留, 不能吞掉.
        let text = r#"{"code":0,"req_0":{"code":20271,"data":{"message":"验证码错误"}}}"#;
        let reply = parse_cgi_envelope(text, 0).unwrap();
        assert_eq!(reply.code, 20271);
        assert_eq!(reply.data["message"], "验证码错误");
    }

    #[test]
    fn parse_envelope_preserves_credential_expired_code() {
        let text = r#"{"code":0,"req_0":{"code":104400,"data":{}}}"#;
        let reply = parse_cgi_envelope(text, 0).unwrap();
        assert_eq!(reply.code, 104400);
    }

    #[test]
    fn parse_envelope_global_code_errors() {
        let text = r#"{"code":-1,"message":"error","req_0":{"code":0,"data":{}}}"#;
        match parse_cgi_envelope(text, 0) {
            Err(QmError::GlobalApi { code, .. }) => assert_eq!(code, -1),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parse_envelope_missing_req_errors() {
        let text = r#"{"code":0,"req_1":{"code":0,"data":{}}}"#;
        match parse_cgi_envelope(text, 0) {
            Err(QmError::ApiData(_)) => {}
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parse_envelope_batch_multiple() {
        let text = r#"{"code":0,"req_0":{"code":0,"data":{"a":1}},"req_1":{"code":2001,"data":{}}}"#;
        let first = parse_cgi_envelope(text, 0).unwrap();
        let second = parse_cgi_envelope(text, 1).unwrap();
        assert_eq!(first.code, 0);
        assert_eq!(first.data["a"], 1);
        assert_eq!(second.code, 2001);
    }

    #[test]
    fn parse_envelope_null_data() {
        let text = r#"{"code":0,"req_0":{"code":0}}"#;
        let reply = parse_cgi_envelope(text, 0).unwrap();
        assert_eq!(reply.code, 0);
        assert!(reply.data.is_null());
    }

    // ------------------------------------------------------------------
    // contract test harness: 本地 mock MusicU 服务器 + 状态缓存并发.
    // ------------------------------------------------------------------

    /// 启动一个本地 mock 服务器, 返回其地址.
    async fn spawn_mock(base_route: &'static str, handler: axum::routing::MethodRouter) -> String {
        use axum::Router;
        let app = Router::new().route(base_route, handler);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn request_cgi_against_mock_server() {
        use axum::routing::post;
        let base = spawn_mock(
            "/cgi-bin/musicu.fcg",
            post(|| async { r#"{"code":0,"req_0":{"code":0,"data":{"name":"晴天"}}}"# }),
        )
        .await;

        let mut ctx = ApiContext::new_with_proxy(None, Some(Platform::Web), None).unwrap();
        ctx.cgi_base_url = format!("{base}/cgi-bin");
        let reply = ctx
            .request_cgi(
                "music.adaptor.SearchAdaptor",
                "do_search_v2",
                json!({ "query": "周杰伦" }),
                &RequestOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(reply.code, 0);
        assert_eq!(reply.data["name"], "晴天");
    }

    #[tokio::test]
    async fn request_cgi_preserves_business_error_from_mock() {
        use axum::routing::post;
        let base = spawn_mock(
            "/cgi-bin/musicu.fcg",
            post(|| async { r#"{"code":0,"req_0":{"code":104400,"data":{"message":"expired"}}}"# }),
        )
        .await;

        let mut ctx = ApiContext::new_with_proxy(None, Some(Platform::Web), None).unwrap();
        ctx.cgi_base_url = format!("{base}/cgi-bin");
        let reply = ctx
            .request_cgi("music.UserInfo.userInfoServer", "GetLoginUserInfo", json!({}), &RequestOptions::default())
            .await
            .unwrap();
        // transport 不吞掉业务错误码.
        assert_eq!(reply.code, 104400);
        assert!(matches!(reply.require_success(), Err(QmError::CredentialExpired(_))));
    }

    #[tokio::test]
    async fn request_cgi_batch_partial_failure_from_mock() {
        use axum::routing::post;
        let body = r#"{"code":0,"req_0":{"code":0,"data":{"ok":1}},"req_1":{"code":2001,"data":{}}}"#;
        let base = spawn_mock("/cgi-bin/musicu.fcg", post(move || async move { body })).await;

        let mut ctx = ApiContext::new_with_proxy(None, Some(Platform::Web), None).unwrap();
        ctx.cgi_base_url = format!("{base}/cgi-bin");
        let reqs = [("music.a", "A", json!({})), ("music.b", "B", json!({}))];
        let replies = ctx.request_cgi_batch(&reqs, &RequestOptions::default()).await.unwrap();
        assert_eq!(replies.len(), 2);
        assert!(replies[0].succeeded());
        assert_eq!(replies[1].code, 2001);
        let report = CgiReply::report(&replies);
        assert_eq!(report.succeeded, 1);
        assert_eq!(report.failures, vec![(1, 2001)]);
    }

    #[test]
    fn model_schema_drift_uses_defaults() {
        // 曲谱接口字段名变化 / 缺失时, 模型使用默认值而非报错或静默错位.
        let drift = serde_json::json!({ "result": null, "totalMap": {} });
        let parsed: crate::models::song::GetSheetResponse =
            serde_json::from_value(drift).unwrap();
        assert!(parsed.result.is_empty());
    }

    #[tokio::test]
    async fn cached_qimei_reused_from_device_without_network() {
        let ctx = ApiContext::new_with_proxy(None, Some(Platform::Android), None).unwrap();
        let mut device = ctx.device();
        device.qimei = Some("q16".into());
        device.qimei36 = Some("q36".into());
        device.qimei_save_time = Some(now());
        ctx.set_device(device);

        let q = ctx.get_cached_qimei().await.unwrap();
        assert_eq!(q, Some(("q16".into(), "q36".into())));
        // 缓存未变, 不应重新申请.
        assert_eq!(ctx.qimei(), Some(("q16".into(), "q36".into())));
    }

    #[tokio::test]
    async fn session_reused_from_device_without_network() {
        let ctx = ApiContext::new_with_proxy(None, Some(Platform::Android), None).unwrap();
        let mut device = ctx.device();
        device.session_uid = Some("uid-1".into());
        device.session_sid = Some("sid-1".into());
        device.session_save_time = Some(now());
        ctx.set_device(device);

        ctx.ensure_session(Platform::Android).await.unwrap();
        assert_eq!(ctx.device().session_uid.as_deref(), Some("uid-1"));
    }

    #[tokio::test]
    async fn concurrent_cached_qimei_reads_are_consistent() {
        let ctx = std::sync::Arc::new(ApiContext::new_with_proxy(None, Some(Platform::Android), None).unwrap());
        let mut device = ctx.device();
        device.qimei = Some("q16".into());
        device.qimei36 = Some("q36".into());
        device.qimei_save_time = Some(now());
        ctx.set_device(device);

        let mut handles = Vec::new();
        for _ in 0..16 {
            let ctx = ctx.clone();
            handles.push(tokio::spawn(async move { ctx.get_cached_qimei().await }));
        }
        for h in handles {
            let q = h.await.unwrap().unwrap();
            assert_eq!(q, Some(("q16".into(), "q36".into())));
        }
    }
}
