//! API 请求上下文 (对应 Python 端 `core/api_context.py`).

use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
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
#[derive(Debug, Clone, Default)]
pub struct RequestOptions {
    pub comm: Option<Value>,
    pub override_comm: bool,
    pub preserve_bool: bool,
    pub credential: Option<Credential>,
    pub platform: Option<Platform>,
    pub sign: bool,
    pub require_login: bool,
}

/// Android 平台会话 (账号级运行态, 与设备身份分离).
///
/// `Device` 只代表设备身份 (android_id/imei/qimei 等); session 归属于账号,
/// 保存在 `ApiContext` 的 per-account 缓存中, 避免多账号并发时的 TOCTOU 竞态.
#[derive(Debug, Clone)]
pub(crate) struct AndroidSession {
    pub uid: String,
    pub sid: String,
    pub acquired_at: i64,
    /// 申请时对应的设备 epoch (设备身份更换后缓存自动失效).
    pub device_epoch: u64,
}

impl AndroidSession {
    fn valid(&self, current_epoch: u64) -> bool {
        !self.uid.is_empty()
            && !self.sid.is_empty()
            && now() - self.acquired_at < 86_400
            && self.device_epoch == current_epoch
    }
}

/// 请求上下文: 持有 HTTP 客户端、平台、版本策略、凭证与设备状态.
///
/// `Device` 是设备指纹 (QIMEI 等) 的**唯一状态源**; session 是账号运行态,
/// 按账号缓存于 `sessions`, 运行时获取的新 QIMEI 写回 `Device`.
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
    /// QIMEI 申请地址 (默认官方接口), 测试时可指向 mock 服务器.
    pub qimei_url: String,
    credential: Mutex<Credential>,
    device: Mutex<Device>,
    /// 设备身份 epoch (每次 `set_device` 递增, 使既有 session 缓存失效).
    device_epoch: std::sync::atomic::AtomicU64,
    /// 按账号缓存的 Android session (`musicid → AndroidSession`).
    sessions: tokio::sync::Mutex<std::collections::HashMap<i64, AndroidSession>>,
    /// 会话 / QIMEI 申请时的 singleflight 锁 (避免并发 stale 请求重复申请).
    state_lock: tokio::sync::Mutex<()>,
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
            let proxy = reqwest::Proxy::all(p).map_err(QmError::from)?;
            builder = builder.proxy(proxy);
        }
        let http = builder.build().map_err(QmError::from)?;
        Ok(ApiContext {
            http,
            platform: platform.unwrap_or(Platform::Android),
            version_policy: VersionPolicy::default(),
            cgi_base_url: "https://u.y.qq.com/cgi-bin".to_string(),
            qimei_url: "https://api.tencentmusic.com/tme/trpc/proxy".to_string(),
            credential: Mutex::new(credential.unwrap_or_default()),
            device: Mutex::new(Device::random()),
            device_epoch: std::sync::atomic::AtomicU64::new(0),
            sessions: tokio::sync::Mutex::new(std::collections::HashMap::new()),
            state_lock: tokio::sync::Mutex::new(()),
            limiter: TokenBucket::default(),
        })
    }

    /// 读取当前全局凭证.
    pub fn credential(&self) -> Credential {
        self.credential
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// 设置全局凭证.
    pub fn set_credential(&self, credential: Credential) {
        *self.credential.lock().unwrap() = credential;
    }

    /// 读取设备.
    pub fn device(&self) -> Device {
        self.device
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// 替换设备 (例如从持久化文件加载).
    ///
    /// 更换设备身份会让此前申请的 Android session 失效 (通过 device epoch
    /// 使缓存全部过期, 下次请求按需重新申请).
    pub fn set_device(&self, device: Device) {
        *self.device.lock().unwrap() = device;
        self.device_epoch.fetch_add(1, Ordering::Relaxed);
    }

    /// 使指定账号的 Android session 失效 (登出/凭证刷新后调用).
    pub(crate) async fn invalidate_session(&self, musicid: i64) {
        self.sessions.lock().await.remove(&musicid);
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

    /// 获取 Android 会话的**不可变快照** (归属指定账号, 按账号缓存).
    ///
    /// - 命中本账号未过期的缓存 → 直接返回 `Arc<AndroidSession>`;
    /// - 否则在 `state_lock` 单飞下申请, 写入 per-account 缓存后返回.
    ///
    /// 返回的是不可变快照: 调用方在 `build_comm` 中使用该快照的 `uid`/`sid`,
    /// 与 `credential` 原子一致, 不会因其他账号并发请求而读到别人的 session.
    pub(crate) async fn session_for(
        &self,
        platform: Platform,
        credential: &Credential,
    ) -> Result<Arc<AndroidSession>> {
        if platform != Platform::Android {
            return Err(QmError::ValueError(
                "session_for 仅适用于 Android 平台".into(),
            ));
        }
        let epoch = self.device_epoch.load(std::sync::atomic::Ordering::Relaxed);
        {
            let sessions = self.sessions.lock().await;
            if let Some(s) = sessions.get(&credential.musicid) {
                if s.valid(epoch) {
                    return Ok(Arc::new(s.clone()));
                }
            }
        }
        let _guard = self.state_lock.lock().await;
        {
            let sessions = self.sessions.lock().await;
            if let Some(s) = sessions.get(&credential.musicid) {
                if s.valid(epoch) {
                    return Ok(Arc::new(s.clone()));
                }
            }
        }
        let session = self.fetch_session(credential).await?;
        let mut sessions = self.sessions.lock().await;
        sessions.insert(credential.musicid, session.clone());
        Ok(Arc::new(session))
    }

    /// 向服务器申请新 session (调用方须已持有 `state_lock`).
    async fn fetch_session(&self, credential: &Credential) -> Result<AndroidSession> {
        let device = self.device();
        let qimei = self.qimei_locked().await?;
        let comm = self.version_policy.build_comm(
            Platform::Android,
            credential,
            &device,
            qimei.as_ref(),
            None,
        );
        let payload = json!({
            "comm": comm,
            "req_0": {
                "module": "music.getSession.session",
                "method": "GetSession",
                "param": {
                    "uid": "",
                    "vkey": 0,
                    "caller": 0,
                },
            },
        });
        let user_agent = self.get_user_agent(Platform::Android);
        let resp = self
            .http
            .post(format!("{}/musicu.fcg", self.cgi_base_url))
            .json(&payload)
            .header("User-Agent", user_agent)
            .send()
            .await
            .map_err(QmError::from)?;
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
        Ok(AndroidSession {
            uid,
            sid,
            acquired_at: now(),
            device_epoch: self.device_epoch.load(std::sync::atomic::Ordering::Relaxed),
        })
    }

    /// 从 Device 读取未过期的 QIMEI 缓存 (不申请锁).
    fn qimei_from_cache(&self) -> Option<(String, String)> {
        let device = self.device();
        if let (Some(q16), Some(q36)) = (device.qimei.as_ref(), device.qimei36.as_ref()) {
            let fresh = device
                .qimei_save_time
                .map(|t| now() - t < 86_400)
                .unwrap_or(false);
            if fresh && !q16.is_empty() && !q36.is_empty() {
                return Some((q16.clone(), q36.clone()));
            }
        }
        None
    }

    /// 获取缓存的 QIMEI, 过期时重新申请.
    ///
    /// 从 `Device` 读取缓存 (过期时间 24 小时); 重新申请成功后写回 `Device`.
    /// 并发 stale 请求通过 singleflight 只触发一次申请.
    pub async fn get_cached_qimei(&self) -> Result<Option<(String, String)>> {
        if let Some(q) = self.qimei_from_cache() {
            return Ok(Some(q));
        }
        let _guard = self.state_lock.lock().await;
        self.qimei_locked().await
    }

    /// 申请 QIMEI 的完整流程 (调用方须已持有 `state_lock`).
    async fn qimei_locked(&self) -> Result<Option<(String, String)>> {
        if let Some(q) = self.qimei_from_cache() {
            return Ok(Some(q));
        }
        let profile = self.version_policy.get_profile(Platform::Android);
        let app_version = profile
            .qimei_app_version
            .clone()
            .unwrap_or_else(|| "14.9.0.8".into());
        let sdk_version = profile
            .qimei_sdk_version
            .clone()
            .unwrap_or_else(|| "1.2.13.6".into());

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
            .post(&self.qimei_url)
            .headers(header_map)
            .json(&body)
            .send()
            .await
            .map_err(QmError::from)?;
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
            headers.insert(
                "User-Agent",
                HeaderValue::from_str(&self.get_user_agent(Platform::Web)).unwrap(),
            );
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
        // 先确定本次请求生效的账号, 并取得与该账号原子一致的 session 快照.
        let cred = credential.cloned().unwrap_or_else(|| self.credential());
        let android_session = if target_platform == Platform::Android {
            Some(self.session_for(target_platform, &cred).await?)
        } else {
            None
        };

        let device = self.device();

        let final_comm = if override_comm {
            comm.clone().unwrap_or_else(|| json!({}))
        } else {
            let qimei = if target_platform == Platform::Android {
                self.get_cached_qimei().await?
            } else {
                None
            };
            let mut base = self.version_policy.build_comm(
                target_platform,
                &cred,
                &device,
                qimei.as_ref(),
                android_session.as_deref(),
            );
            if let Some(Value::Object(map)) = comm {
                for (k, v) in map {
                    base[k] = v;
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
                return Err(QmError::CredentialInvalid(
                    "请求需要登录, 未提供有效的登录凭证".into(),
                ));
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
        let resp = request.send().await.map_err(QmError::from)?;
        let status = resp.status().as_u16();
        let text = resp.text().await.map_err(QmError::from)?;
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
                return Err(QmError::CredentialInvalid(
                    "请求需要登录, 未提供有效的登录凭证".into(),
                ));
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
        let resp = request.send().await.map_err(QmError::from)?;
        let status = resp.status().as_u16();
        let text = resp.text().await.map_err(QmError::from)?;
        if status != 200 {
            return Err(QmError::http(status, text));
        }
        // 只解析一次整个 envelope, 再逐个提取 req_N (避免 N 次全量 parse).
        let env: Value = serde_json::from_str(&text)?;
        let env_code =
            env.get("code")
                .and_then(Value::as_i64)
                .ok_or_else(|| QmError::Protocol {
                    stage: "cgi-envelope",
                    message: "missing or invalid global code".into(),
                })?;
        if env_code != 0 {
            return Err(QmError::GlobalApi {
                code: env_code,
                data: crate::error::redact_payload(&text, 400),
            });
        }
        let mut out = Vec::with_capacity(requests.len());
        for i in 0..requests.len() {
            let req0 = env
                .get(format!("req_{i}"))
                .cloned()
                .ok_or_else(|| QmError::Protocol {
                    stage: "cgi-envelope",
                    message: format!("missing req_{i}"),
                })?;
            let code =
                req0.get("code")
                    .and_then(Value::as_i64)
                    .ok_or_else(|| QmError::Protocol {
                        stage: "cgi-req",
                        message: format!("missing or invalid req_{i}.code"),
                    })?;
            let data = req0.get("data").cloned().unwrap_or(Value::Null);
            out.push(CgiReply::new(code, data));
        }
        Ok(out)
    }

    /// 下载原始字节 (用于音频文件下载).
    pub async fn request_http_bytes(
        &self,
        url: &str,
        credential: Option<&Credential>,
    ) -> Result<Vec<u8>> {
        self.limiter.acquire().await;
        let (headers, cookies) = self.prepare_http_kwargs(credential, HeaderMap::new(), Vec::new());
        let mut request = self.http.get(url).headers(headers);
        for (k, v) in &cookies {
            request = request.header("Cookie", format!("{k}={v}"));
        }
        let resp = request.send().await.map_err(QmError::from)?;
        let status = resp.status().as_u16();
        if status != 200 {
            let text = resp.text().await.unwrap_or_default();
            return Err(QmError::http(status, text));
        }
        let bytes = resp.bytes().await.map_err(QmError::from)?;
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
        let resp = request.send().await.map_err(QmError::from)?;
        let status = resp.status().as_u16();
        let text = resp.text().await.map_err(QmError::from)?;
        if status != 200 {
            return Err(QmError::http(status, text));
        }
        Ok(text)
    }
}

/// 解析 CGI 全局信封并提取 `req_{index}` 的固定响应 `{ code, data }`.
///
/// 协议解析 fail-closed:
/// - HTTP 层已确认状态码为 200;
/// - 全局信封 `code != 0` 视为 transport 级错误 (`GlobalApi`);
/// - `code` 缺失或类型错误 (非数字) → `Protocol` 错误, 不当作 0 成功;
/// - `req_{index}` 缺失 → `Protocol` 错误;
/// - 其余情况返回 `CgiReply { code, data }`, 不解释业务错误码.
pub(crate) fn parse_cgi_envelope(text: &str, index: usize) -> Result<CgiReply<Value>> {
    let env: Value = serde_json::from_str(text)?;
    let env_code = env
        .get("code")
        .and_then(Value::as_i64)
        .ok_or_else(|| QmError::Protocol {
            stage: "cgi-envelope",
            message: "missing or invalid global code".into(),
        })?;
    if env_code != 0 {
        return Err(QmError::GlobalApi {
            code: env_code,
            data: crate::error::redact_payload(text, 400),
        });
    }
    let req0 = env
        .get(format!("req_{index}"))
        .cloned()
        .ok_or_else(|| QmError::Protocol {
            stage: "cgi-envelope",
            message: format!("missing req_{index}"),
        })?;
    let code = req0
        .get("code")
        .and_then(Value::as_i64)
        .ok_or_else(|| QmError::Protocol {
            stage: "cgi-req",
            message: format!("missing or invalid req_{index}.code"),
        })?;
    let data = req0.get("data").cloned().unwrap_or(Value::Null);
    Ok(CgiReply::new(code, data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_envelope_success() {
        let text =
            r#"{"code":0,"req_0":{"code":0,"data":{"songmid":"001X3HEN1oK0Jr","name":"晴天"}}}"#;
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
            Err(QmError::Protocol { stage, .. }) => assert_eq!(stage, "cgi-envelope"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parse_envelope_missing_global_code_fails_closed() {
        let text = r#"{"req_0":{"code":0,"data":{}}}"#;
        assert!(matches!(
            parse_cgi_envelope(text, 0),
            Err(QmError::Protocol {
                stage: "cgi-envelope",
                ..
            })
        ));
    }

    #[test]
    fn parse_envelope_string_global_code_fails_closed() {
        let text = r#"{"code":"ok","req_0":{"code":0,"data":{}}}"#;
        assert!(matches!(
            parse_cgi_envelope(text, 0),
            Err(QmError::Protocol {
                stage: "cgi-envelope",
                ..
            })
        ));
    }

    #[test]
    fn parse_envelope_missing_req_code_fails_closed() {
        let text = r#"{"code":0,"req_0":{"data":{}}}"#;
        assert!(matches!(
            parse_cgi_envelope(text, 0),
            Err(QmError::Protocol {
                stage: "cgi-req",
                ..
            })
        ));
    }

    #[test]
    fn parse_envelope_string_req_code_fails_closed() {
        let text = r#"{"code":0,"req_0":{"code":"broken","data":{}}}"#;
        assert!(matches!(
            parse_cgi_envelope(text, 0),
            Err(QmError::Protocol {
                stage: "cgi-req",
                ..
            })
        ));
    }

    #[test]
    fn parse_envelope_null_req_code_fails_closed() {
        let text = r#"{"code":0,"req_0":{"code":null,"data":{}}}"#;
        assert!(matches!(
            parse_cgi_envelope(text, 0),
            Err(QmError::Protocol {
                stage: "cgi-req",
                ..
            })
        ));
    }

    #[test]
    fn parse_envelope_batch_multiple() {
        let text =
            r#"{"code":0,"req_0":{"code":0,"data":{"a":1}},"req_1":{"code":2001,"data":{}}}"#;
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
            .request_cgi(
                "music.UserInfo.userInfoServer",
                "GetLoginUserInfo",
                json!({}),
                &RequestOptions::default(),
            )
            .await
            .unwrap();
        // transport 不吞掉业务错误码.
        assert_eq!(reply.code, 104400);
        assert!(matches!(
            reply.require_success(),
            Err(QmError::CredentialExpired(_))
        ));
    }

    #[tokio::test]
    async fn request_cgi_batch_partial_failure_from_mock() {
        use axum::routing::post;
        let body =
            r#"{"code":0,"req_0":{"code":0,"data":{"ok":1}},"req_1":{"code":2001,"data":{}}}"#;
        let base = spawn_mock("/cgi-bin/musicu.fcg", post(move || async move { body })).await;

        let mut ctx = ApiContext::new_with_proxy(None, Some(Platform::Web), None).unwrap();
        ctx.cgi_base_url = format!("{base}/cgi-bin");
        let reqs = [("music.a", "A", json!({})), ("music.b", "B", json!({}))];
        let replies = ctx
            .request_cgi_batch(&reqs, &RequestOptions::default())
            .await
            .unwrap();
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
        let parsed: crate::models::song::GetSheetResponse = serde_json::from_value(drift).unwrap();
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
    async fn session_reused_within_same_account() {
        use axum::routing::post;
        let base = spawn_mock(
            "/cgi-bin/musicu.fcg",
            post(|| async {
                r#"{"code":0,"req_0":{"code":0,"data":{"session":{"uid":"u1","sid":"s1"}}}}"#
            }),
        )
        .await;
        let qimei_body = r#"{"data":"{\"data\":{\"q16\":\"q16\",\"q36\":\"q36\"}}"}"#;
        let base2 = spawn_mock("/tme/trpc/proxy", post(move || async move { qimei_body })).await;

        let mut ctx = ApiContext::new_with_proxy(None, Some(Platform::Android), None).unwrap();
        ctx.cgi_base_url = format!("{base}/cgi-bin");
        ctx.qimei_url = format!("{base2}/tme/trpc/proxy");

        let mut cred = Credential::default();
        cred.musicid = 42;
        cred.str_musicid = "42".into();

        let s1 = ctx.session_for(Platform::Android, &cred).await.unwrap();
        // 同一账号二次获取 → 命中 per-account 缓存, 仍返回同一 uid/sid.
        let s2 = ctx.session_for(Platform::Android, &cred).await.unwrap();
        assert_eq!(s1.uid, s2.uid);
        assert_eq!(s1.uid, "u1");
    }

    #[tokio::test]
    async fn session_cached_per_account() {
        use axum::routing::post;
        // mock 按账号返回不同 session.
        let base = spawn_mock(
            "/cgi-bin/musicu.fcg",
            post(|| async {
                r#"{"code":0,"req_0":{"code":0,"data":{"session":{"uid":"new-uid","sid":"new-sid"}}}}"#
            }),
        )
        .await;
        let qimei_body = r#"{"data":"{\"data\":{\"q16\":\"q16\",\"q36\":\"q36\"}}"}"#;
        let base2 = spawn_mock("/tme/trpc/proxy", post(move || async move { qimei_body })).await;

        let mut ctx = ApiContext::new_with_proxy(None, Some(Platform::Android), None).unwrap();
        ctx.cgi_base_url = format!("{base}/cgi-bin");
        ctx.qimei_url = format!("{base2}/tme/trpc/proxy");

        // 账号 A.
        let mut cred_a = Credential::default();
        cred_a.musicid = 111;
        cred_a.str_musicid = "111".into();
        let a = ctx.session_for(Platform::Android, &cred_a).await.unwrap();
        assert_eq!(a.uid, "new-uid");

        // 账号 B (不同 musicid) → 必须各自持有自己的 session, 不共享单例.
        let mut cred_b = Credential::default();
        cred_b.musicid = 222;
        cred_b.str_musicid = "222".into();
        let b = ctx.session_for(Platform::Android, &cred_b).await.unwrap();
        assert_eq!(b.uid, "new-uid");

        // 再取 A → 缓存命中, 仍返回 A 的 session (不会读到 B).
        let a2 = ctx.session_for(Platform::Android, &cred_a).await.unwrap();
        assert_eq!(a2.uid, a.uid);
    }

    #[tokio::test]
    async fn concurrent_cached_qimei_reads_are_consistent() {
        let ctx = std::sync::Arc::new(
            ApiContext::new_with_proxy(None, Some(Platform::Android), None).unwrap(),
        );
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

    #[tokio::test]
    async fn session_for_singleflight_does_not_deadlock() {
        use axum::routing::post;
        use tokio::time::Duration;
        // 两个 mock: session (cgi) 与 qimei, 均为空缓存 → 走完整 singleflight 路径.
        let base = spawn_mock(
            "/cgi-bin/musicu.fcg",
            post(|| async {
                r#"{"code":0,"req_0":{"code":0,"data":{"session":{"uid":"u1","sid":"s1"}}}}"#
            }),
        )
        .await;
        let qimei_body = r#"{"data":"{\"data\":{\"q16\":\"q16\",\"q36\":\"q36\"}}"}"#;
        let base2 = spawn_mock("/tme/trpc/proxy", post(move || async move { qimei_body })).await;

        let mut ctx = ApiContext::new_with_proxy(None, Some(Platform::Android), None).unwrap();
        ctx.cgi_base_url = format!("{base}/cgi-bin");
        ctx.qimei_url = format!("{base2}/tme/trpc/proxy");

        let mut cred = Credential::default();
        cred.musicid = 7;
        cred.str_musicid = "7".into();

        // 若 session_for 内重复加锁会死锁, 此处 5s 超时会失败.
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            ctx.session_for(Platform::Android, &cred),
        )
        .await;
        let session = result
            .expect("session_for 应在超时前完成 (单飞锁不可重入)")
            .unwrap();
        assert_eq!(session.uid, "u1");
        assert_eq!(ctx.qimei(), Some(("q16".into(), "q36".into())));
    }

    #[tokio::test]
    async fn set_device_invalidates_cached_session_via_epoch() {
        use axum::routing::post;
        use std::sync::atomic::Ordering as AOrdering;
        let base = spawn_mock(
            "/cgi-bin/musicu.fcg",
            post(|| async {
                r#"{"code":0,"req_0":{"code":0,"data":{"session":{"uid":"u1","sid":"s1"}}}}"#
            }),
        )
        .await;
        let qimei_body = r#"{"data":"{\"data\":{\"q16\":\"q16\",\"q36\":\"q36\"}}"}"#;
        let base2 = spawn_mock("/tme/trpc/proxy", post(move || async move { qimei_body })).await;

        let mut ctx = ApiContext::new_with_proxy(None, Some(Platform::Android), None).unwrap();
        ctx.cgi_base_url = format!("{base}/cgi-bin");
        ctx.qimei_url = format!("{base2}/tme/trpc/proxy");

        let mut cred = Credential::default();
        cred.musicid = 5;
        cred.str_musicid = "5".into();

        let s1 = ctx.session_for(Platform::Android, &cred).await.unwrap();
        assert_eq!(s1.uid, "u1");

        // 更换设备身份 → epoch 递增 → 原缓存失效, 下次重新申请.
        let epoch_before = ctx.device_epoch.load(AOrdering::Relaxed);
        ctx.set_device(Device::random());
        assert_eq!(ctx.device_epoch.load(AOrdering::Relaxed), epoch_before + 1);

        let s2 = ctx.session_for(Platform::Android, &cred).await.unwrap();
        assert_eq!(s2.uid, "u1"); // mock 恒定返回, 但确实重新申请 (epoch 已变).
        assert_ne!(s1.device_epoch, s2.device_epoch);
    }
}
