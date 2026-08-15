//! API 请求上下文 (对应 Python 端 `core/api_context.py`).

use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::device::Device;
use crate::error::{QmError, Result};
use crate::qimei;
use crate::rate_limiter::TokenBucket;
use crate::sign::zzc_sign;
use crate::versioning::{Platform, VersionPolicy};
use crate::Credential;

/// CGI 请求选项 (轻量拷贝, 与 `crate::client::CgiOptions` 同构).
#[derive(Debug, Clone)]
pub struct RequestOptions {
    pub comm: Option<Value>,
    pub override_comm: bool,
    pub preserve_bool: bool,
    pub allow_error_codes: Option<Vec<i64>>,
    pub parse_on_allow: bool,
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
            allow_error_codes: None,
            parse_on_allow: false,
            credential: None,
            platform: None,
            sign: false,
            require_login: false,
        }
    }
}

/// Android 平台会话缓存 (来自 `music.getSession.session`).
#[derive(Debug, Clone, Default)]
struct Session {
    uid: String,
    sid: String,
    save_time: i64,
}

/// 请求上下文: 持有 HTTP 客户端、平台、版本策略、凭证与设备状态.
#[derive(Debug)]
pub struct ApiContext {
    /// 底层 HTTP 客户端.
    pub http: reqwest::Client,
    /// 默认请求平台.
    pub platform: Platform,
    /// 版本策略.
    pub version_policy: VersionPolicy,
    credential: Mutex<Credential>,
    device: Mutex<Device>,
    qimei: Mutex<Option<(String, String)>>,
    qimei_time: Mutex<i64>,
    session: Mutex<Option<Session>>,
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
            credential: Mutex::new(credential.unwrap_or_default()),
            device: Mutex::new(Device::random()),
            qimei: Mutex::new(None),
            qimei_time: Mutex::new(0),
            session: Mutex::new(None),
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

    /// 读取当前 QIMEI 缓存.
    pub fn qimei(&self) -> Option<(String, String)> {
        self.qimei.lock().unwrap().clone()
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
            let session = self.session.lock().unwrap();
            if let Some(s) = session.as_ref() {
                if now() - s.save_time < 86_400 && !s.uid.is_empty() && !s.sid.is_empty() {
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
            return Err(QmError::Http {
                status,
                body: resp.text().await.unwrap_or_default(),
            });
        }
        let value: Value = resp.json().await?;
        let session_data = &value["req_0"]["data"]["session"];
        let uid = value_to_string(&session_data["uid"]);
        let sid = value_to_string(&session_data["sid"]);
        if uid.is_empty() || sid.is_empty() {
            return Err(QmError::ApiData("获取 session 失败".into()));
        }
        let mut session = self.session.lock().unwrap();
        *session = Some(Session {
            uid,
            sid,
            save_time: now(),
        });
        Ok(())
    }

    /// 获取缓存的 QIMEI, 过期时重新申请.
    pub async fn get_cached_qimei(&self) -> Result<Option<(String, String)>> {
        let profile = self.version_policy.get_profile(Platform::Android);
        let app_version = profile.qimei_app_version.clone().unwrap_or_else(|| "14.9.0.8".into());
        let sdk_version = profile.qimei_sdk_version.clone().unwrap_or_else(|| "1.2.13.6".into());

        {
            let time = self.qimei_time.lock().unwrap();
            if now() - *time < 86_400 {
                if let Some(q) = self.qimei.lock().unwrap().as_ref() {
                    return Ok(Some(q.clone()));
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
            let mut qimei = self.qimei.lock().unwrap();
            *qimei = Some(q.clone());
            *self.qimei_time.lock().unwrap() = now();
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
            "https://u.y.qq.com/cgi-bin/musics.fcg"
        } else {
            "https://u.y.qq.com/cgi-bin/musicu.fcg"
        };
        let user_agent = self.get_user_agent(target_platform);
        Ok((url.to_string(), payload, params, user_agent))
    }

    /// 执行一个 CGI 请求并返回 `req_0.data` 原始值.
    pub async fn request_cgi(
        &self,
        module: &str,
        method: &str,
        param: Value,
        opts: &RequestOptions,
    ) -> Result<Value> {
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
            return Err(QmError::Http { status, body: text });
        }
        let mut env: Value = serde_json::from_str(&text)?;
        let env_code = env.get("code").and_then(Value::as_i64).unwrap_or(0);
        if env_code != 0 {
            return Err(QmError::GlobalApi { code: env_code, data: text });
        }
        let req0 = env
            .get_mut("req_0")
            .cloned()
            .ok_or_else(|| QmError::ApiData("CGI 响应缺少 req_0".into()))?;
        let code = req0.get("code").and_then(Value::as_i64).unwrap_or(0);
        let data = req0.get("data").cloned().unwrap_or(Value::Null);

        let allow: HashSet<i64> = opts
            .allow_error_codes
            .as_ref()
            .map(|codes| codes.iter().copied().collect())
            .unwrap_or_default();
        if allow.contains(&code) {
            if opts.parse_on_allow {
                return Ok(data);
            }
            return Ok(req0);
        }
        match code {
            2000 => return Err(QmError::SignatureRequired),
            2001 => return Err(QmError::RateLimited),
            1000 | 104401 | 104400 => {
                return Err(QmError::CredentialExpired(format!("code {code}")));
            }
            c if c != 0 => {
                return Err(QmError::CgiApi {
                    code: c,
                    data: data.to_string(),
                });
            }
            _ => {}
        }
        Ok(data)
    }

    /// 批量执行多个 CGI 请求 (合并为一次 `req_0..req_N` 调用).
    ///
    /// `requests` 为 `(module, method, param)` 三元组列表, 返回每个子请求的 `data`.
    pub async fn request_cgi_batch(
        &self,
        requests: &[(&str, &str, Value)],
        opts: &RequestOptions,
    ) -> Result<Vec<Value>> {
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
            return Err(QmError::Http { status, body: text });
        }
        let mut env: Value = serde_json::from_str(&text)?;
        let env_code = env.get("code").and_then(Value::as_i64).unwrap_or(0);
        if env_code != 0 {
            return Err(QmError::GlobalApi { code: env_code, data: text });
        }

        let allow: HashSet<i64> = opts
            .allow_error_codes
            .as_ref()
            .map(|codes| codes.iter().copied().collect())
            .unwrap_or_default();
        let mut out = Vec::with_capacity(requests.len());
        for i in 0..requests.len() {
            let req0 = env
                .get_mut(&format!("req_{i}"))
                .cloned()
                .ok_or_else(|| QmError::ApiData(format!("CGI 响应缺少 req_{i}")))?;
            let code = req0.get("code").and_then(Value::as_i64).unwrap_or(0);
            let data = req0.get("data").cloned().unwrap_or(Value::Null);
            if allow.contains(&code) {
                if opts.parse_on_allow {
                    out.push(data);
                } else {
                    out.push(req0);
                }
                continue;
            }
            match code {
                2000 => return Err(QmError::SignatureRequired),
                2001 => return Err(QmError::RateLimited),
                1000 | 104401 | 104400 => {
                    return Err(QmError::CredentialExpired(format!("code {code}")));
                }
                c if c != 0 => {
                    return Err(QmError::CgiApi {
                        code: c,
                        data: data.to_string(),
                    });
                }
                _ => out.push(data),
            }
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
            return Err(QmError::Http { status, body: text });
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
            return Err(QmError::Http { status, body: text });
        }
        Ok(text)
    }
}
