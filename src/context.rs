//! API 请求上下文 (对应 Python 端 `core/api_context.py`).

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
use crate::transport::{
    ApiTransport, CancellationToken, HttpBody, HttpMethod, RedirectMode, ReqwestApiTransport,
    RetryClass, TransportConfig, TransportRequest, TransportResponse,
};
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
    pub retry: RetryClass,
    pub cancellation: CancellationToken,
}

impl Default for RequestOptions {
    fn default() -> Self {
        Self {
            comm: None,
            override_comm: false,
            preserve_bool: false,
            credential: None,
            platform: None,
            sign: false,
            require_login: false,
            retry: RetryClass::SafeRead,
            cancellation: CancellationToken::new(),
        }
    }
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

/// 设备身份的不可变快照. 异步请求必须绑定**开始时**的 epoch,
/// 而不能在响应返回后再读取“当前 epoch”(否则会把 D0 的结果标成 D1).
#[derive(Clone)]
pub(crate) struct DeviceSnapshot {
    pub epoch: u64,
    pub device: Device,
}

enum FetchOutcome<T> {
    Ready(T),
    Stale,
}

const MAX_DEVICE_RETRIES: u32 = 5;

fn qimei_if_fresh(device: &Device) -> Option<(String, String)> {
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

fn device_replaced_error(stage: &'static str) -> QmError {
    QmError::Protocol {
        stage,
        message: "device replaced during in-flight request".into(),
    }
}

/// 请求上下文: 持有 HTTP 传输、平台、版本策略、凭证与设备状态.
///
/// `Device` 是设备指纹 (QIMEI 等) 的**唯一状态源**; session 是账号运行态,
/// 按账号缓存于 `sessions`, 运行时获取的新 QIMEI 写回 `Device`.
pub struct ApiContext {
    transport: Arc<dyn ApiTransport>,
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

impl std::fmt::Debug for ApiContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiContext")
            .field("platform", &self.platform)
            .field("cgi_base_url", &self.cgi_base_url)
            .field("qimei_url", &self.qimei_url)
            .finish_non_exhaustive()
    }
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
        let mut config = TransportConfig::default();
        config.proxy = proxy.map(str::to_string);
        Self::new_with_transport_config(credential, platform, config)
    }

    /// 使用指定的默认 transport 配置创建上下文.
    pub fn new_with_transport_config(
        credential: Option<Credential>,
        platform: Option<Platform>,
        config: TransportConfig,
    ) -> Result<Self> {
        let transport = Arc::new(ReqwestApiTransport::new(config)?);
        Ok(Self::new_with_transport(credential, platform, transport))
    }

    /// 注入自定义 [`ApiTransport`].
    pub fn new_with_transport(
        credential: Option<Credential>,
        platform: Option<Platform>,
        transport: Arc<dyn ApiTransport>,
    ) -> Self {
        ApiContext {
            transport,
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
        }
    }

    fn note_configured_origins(&self) {
        self.transport.allow_origin(&self.cgi_base_url);
        self.transport.allow_origin(&self.qimei_url);
    }

    pub(crate) async fn execute_transport(
        &self,
        request: TransportRequest,
    ) -> Result<TransportResponse> {
        self.note_configured_origins();
        self.transport.execute(request).await
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

    /// 读取设备身份快照 (device + epoch 在同一把锁下一致).
    pub(crate) fn device_snapshot(&self) -> DeviceSnapshot {
        let guard = self.device.lock().unwrap_or_else(|e| e.into_inner());
        let epoch = self.device_epoch.load(Ordering::Acquire);
        DeviceSnapshot {
            epoch,
            device: guard.clone(),
        }
    }

    fn current_epoch(&self) -> u64 {
        self.device_epoch.load(Ordering::Acquire)
    }

    /// 替换设备 (例如从持久化文件加载).
    ///
    /// 更换设备身份会:
    /// - 递增 device epoch, 使既有 Android session 缓存全部失效;
    /// - 使用调用方传入的新 `Device` (不会把旧 Device 的 QIMEI 拷到新 Device);
    /// - 与 epoch 绑定的 in-flight QIMEI / Session 结果不得再写回新 Device.
    pub fn set_device(&self, device: Device) {
        let mut guard = self.device.lock().unwrap();
        *guard = device;
        self.device_epoch.fetch_add(1, Ordering::Release);
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
        {
            let epoch = self.current_epoch();
            let sessions = self.sessions.lock().await;
            if let Some(s) = sessions.get(&credential.musicid) {
                if s.valid(epoch) {
                    return Ok(Arc::new(s.clone()));
                }
            }
        }
        let _guard = self.state_lock.lock().await;
        for _ in 0..MAX_DEVICE_RETRIES {
            let snapshot = self.device_snapshot();
            {
                let sessions = self.sessions.lock().await;
                if let Some(s) = sessions.get(&credential.musicid) {
                    if s.valid(snapshot.epoch) {
                        return Ok(Arc::new(s.clone()));
                    }
                }
            }
            match self.fetch_session(credential, &snapshot).await? {
                FetchOutcome::Ready(session) => {
                    if self.current_epoch() != snapshot.epoch {
                        continue;
                    }
                    let mut sessions = self.sessions.lock().await;
                    sessions.insert(credential.musicid, session.clone());
                    return Ok(Arc::new(session));
                }
                FetchOutcome::Stale => continue,
            }
        }
        Err(device_replaced_error("android-session"))
    }

    /// 向服务器申请新 session (调用方须已持有 `state_lock`).
    ///
    /// Session 绑定 **请求开始时** 的 `snapshot.epoch`, 而不是响应返回时的当前 epoch.
    /// 若等待期间 Device 被替换, 丢弃结果且不写入缓存.
    async fn fetch_session(
        &self,
        credential: &Credential,
        snapshot: &DeviceSnapshot,
    ) -> Result<FetchOutcome<AndroidSession>> {
        let qimei = match self.qimei_for_snapshot(snapshot).await? {
            FetchOutcome::Ready(q) => q,
            FetchOutcome::Stale => return Ok(FetchOutcome::Stale),
        };
        let comm = self.version_policy.build_comm(
            Platform::Android,
            credential,
            &snapshot.device,
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
        let user_agent = self
            .version_policy
            .get_user_agent(Platform::Android, &snapshot.device);
        let url = format!("{}/musicu.fcg", self.cgi_base_url);
        let mut request = TransportRequest::new(HttpMethod::Post, url.clone());
        request.headers = self.cgi_headers(&url, user_agent, Some(credential));
        request.body = HttpBody::Json(payload);
        request.retry = RetryClass::SafeRead;
        request.redirects = RedirectMode::FollowValidated;
        request.cancellation = CancellationToken::new();
        let resp = self.execute_transport(request).await?;
        let status = resp.status;
        if status != 200 {
            return Err(QmError::http(status, resp.text()));
        }
        let value: Value = serde_json::from_slice(&resp.body)?;
        let session_data = &value["req_0"]["data"]["session"];
        let uid = value_to_string(&session_data["uid"]);
        let sid = value_to_string(&session_data["sid"]);
        if uid.is_empty() || sid.is_empty() {
            return Err(QmError::ApiData("获取 session 失败".into()));
        }
        if self.current_epoch() != snapshot.epoch {
            return Ok(FetchOutcome::Stale);
        }
        Ok(FetchOutcome::Ready(AndroidSession {
            uid,
            sid,
            acquired_at: now(),
            device_epoch: snapshot.epoch,
        }))
    }

    /// 从 Device 读取未过期的 QIMEI 缓存 (不申请锁).
    fn qimei_from_cache(&self) -> Option<(String, String)> {
        qimei_if_fresh(&self.device())
    }

    /// 获取缓存的 QIMEI, 过期时重新申请.
    ///
    /// 从 `Device` 读取缓存 (过期时间 24 小时); 重新申请成功后写回 `Device`.
    /// 并发 stale 请求通过 singleflight 只触发一次申请.
    /// 若申请期间 Device 被替换, 丢弃旧结果, 有界重试新 Device.
    pub async fn get_cached_qimei(&self) -> Result<Option<(String, String)>> {
        if let Some(q) = self.qimei_from_cache() {
            return Ok(Some(q));
        }
        let _guard = self.state_lock.lock().await;
        for _ in 0..MAX_DEVICE_RETRIES {
            if let Some(q) = self.qimei_from_cache() {
                return Ok(Some(q));
            }
            let snapshot = self.device_snapshot();
            match self.qimei_for_snapshot(&snapshot).await? {
                FetchOutcome::Ready(q) => return Ok(q),
                FetchOutcome::Stale => continue,
            }
        }
        Err(device_replaced_error("qimei"))
    }

    /// 为指定 Device 快照申请 QIMEI; epoch 变化时不写回当前 Device.
    async fn qimei_for_snapshot(
        &self,
        snapshot: &DeviceSnapshot,
    ) -> Result<FetchOutcome<Option<(String, String)>>> {
        if let Some(q) = qimei_if_fresh(&snapshot.device) {
            return Ok(FetchOutcome::Ready(Some(q)));
        }
        if self.current_epoch() != snapshot.epoch {
            return Ok(FetchOutcome::Stale);
        }
        if let Some(q) = self.qimei_from_cache() {
            return Ok(FetchOutcome::Ready(Some(q)));
        }
        let fetched = self.fetch_qimei_http(&snapshot.device).await?;
        if self.current_epoch() != snapshot.epoch {
            return Ok(FetchOutcome::Stale);
        }
        if let Some(q) = fetched {
            if !self.commit_qimei(snapshot, &q.0, &q.1) {
                return Ok(FetchOutcome::Stale);
            }
            return Ok(FetchOutcome::Ready(Some(q)));
        }
        Ok(FetchOutcome::Ready(None))
    }

    fn commit_qimei(&self, snapshot: &DeviceSnapshot, q16: &str, q36: &str) -> bool {
        let mut guard = self.device.lock().unwrap();
        if self.device_epoch.load(Ordering::Acquire) != snapshot.epoch {
            return false;
        }
        guard.qimei = Some(q16.to_string());
        guard.qimei36 = Some(q36.to_string());
        guard.qimei_save_time = Some(now());
        true
    }

    async fn fetch_qimei_http(&self, device: &Device) -> Result<Option<(String, String)>> {
        let profile = self.version_policy.get_profile(Platform::Android);
        let app_version = profile
            .qimei_app_version
            .clone()
            .unwrap_or_else(|| "14.9.0.8".into());
        let sdk_version = profile
            .qimei_sdk_version
            .clone()
            .unwrap_or_else(|| "1.2.13.6".into());

        let (_, headers, body) = qimei::build_qimei_request(device, &app_version, &sdk_version)?;
        let resp = self
            .execute_transport(TransportRequest {
                method: HttpMethod::Post,
                url: self.qimei_url.clone(),
                headers,
                query: Vec::new(),
                body: HttpBody::Json(body),
                timeout: None,
                retry: RetryClass::SafeRead,
                redirects: RedirectMode::FollowValidated,
                cancellation: CancellationToken::new(),
            })
            .await?;
        let text = resp.text();
        Ok(qimei::parse_qimei_response(&text))
    }

    /// 为 HTTP 请求准备 kwargs (显式注入 Cookies 与 User-Agent).
    ///
    /// 安全语义: `credential == None` 表示匿名请求, **不会**回退到全局账号凭证.
    /// CGI 调用方若需要默认账号, 必须在请求入口先快照全局凭证再显式传入.
    #[allow(clippy::type_complexity)]
    pub fn prepare_http_kwargs(
        &self,
        credential: Option<&Credential>,
        mut headers: Vec<(String, String)>,
        mut cookies: Vec<(String, String)>,
    ) -> (Vec<(String, String)>, Vec<(String, String)>) {
        if let Some(cred) = credential {
            let str_musicid = cred.str_musicid();
            if !str_musicid.is_empty() {
                cookies.push(("uin".into(), str_musicid.clone()));
                cookies.push(("qqmusic_uin".into(), str_musicid));
            }
            if !cred.musickey.is_empty() {
                cookies.push(("qm_keyst".into(), cred.musickey.clone()));
                cookies.push(("qqmusic_key".into(), cred.musickey.clone()));
            }
        }
        if !headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("User-Agent"))
        {
            headers.push(("User-Agent".into(), self.get_user_agent(Platform::Web)));
        }
        (headers, cookies)
    }

    /// CGI 出口统一组头: User-Agent + Credential Cookie + 缺省时的 y.qq.com Referer/Origin.
    ///
    /// 鉴权 Cookie 写在请求上, 不依赖 transport cookie jar.
    fn cgi_headers(
        &self,
        url: &str,
        user_agent: String,
        credential: Option<&Credential>,
    ) -> Vec<(String, String)> {
        let (headers, cookies) = self.prepare_http_kwargs(
            credential,
            vec![("User-Agent".into(), user_agent)],
            Vec::new(),
        );
        let mut headers = merge_cookie_headers(headers, cookies);
        ensure_yqq_cgi_headers(url, &mut headers);
        headers
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
        for _ in 0..MAX_DEVICE_RETRIES {
            let snap = self.device_snapshot();
            // 直接调用 build_api_kwargs 时仍保留 `None => 当前全局凭证` 的 CGI builder
            // 兼容语义; request_cgi/request_cgi_batch 会在入口快照后显式传入凭证.
            let cred = credential.cloned().unwrap_or_else(|| self.credential());
            let android_session = if target_platform == Platform::Android {
                Some(self.session_for(target_platform, &cred).await?)
            } else {
                None
            };

            if let Some(ref s) = android_session {
                if s.device_epoch != self.current_epoch() {
                    continue;
                }
            }
            if self.current_epoch() != snap.epoch {
                continue;
            }

            let qimei = if target_platform == Platform::Android {
                self.get_cached_qimei().await?
            } else {
                None
            };

            let epoch = self.current_epoch();
            if epoch != snap.epoch {
                continue;
            }
            if let Some(ref s) = android_session {
                if s.device_epoch != epoch {
                    continue;
                }
            }

            let device = self.device();
            if self.current_epoch() != epoch {
                continue;
            }
            // 优先使用当前 Device 上已提交的 QIMEI, 保证与 aid 等同属一个 snapshot.
            let qimei = qimei_if_fresh(&device).or(qimei);

            let final_comm = if override_comm {
                comm.clone().unwrap_or_else(|| json!({}))
            } else {
                let mut base = self.version_policy.build_comm(
                    target_platform,
                    &cred,
                    &device,
                    qimei.as_ref(),
                    android_session.as_deref(),
                );
                if let Some(Value::Object(map)) = comm.clone() {
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
            let user_agent = self.version_policy.get_user_agent(target_platform, &device);
            return Ok((url.to_string(), payload, params, user_agent));
        }
        Err(device_replaced_error("build-api-kwargs"))
    }

    /// 执行一个 CGI 请求, 返回固定形状的响应 `CgiReply { code, data }`.
    ///
    /// 对 `u.y.qq.com` / `c.y.qq.com` / `c6.y.qq.com` (以及本地 mock CGI)
    /// 在请求尚未携带时补上 `Referer: https://y.qq.com/` 与
    /// `Origin: https://y.qq.com`, 不覆盖已有 Referer (如 ptlogin).
    /// Cookie 由 [`Credential`] 写入请求, 不依赖 transport cookie jar.
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

        // 每个 CGI 请求只在入口读取一次全局账号. 后续 build_comm/session/Cookie 都绑定
        // 同一不可变凭证快照, 避免并发 set_credential 导致跨账号 TOCTOU.
        let effective_credential = opts.credential.clone().unwrap_or_else(|| self.credential());
        if opts.require_login
            && (effective_credential.musicid == 0 || effective_credential.musickey.is_empty())
        {
            return Err(QmError::CredentialInvalid(
                "请求需要登录, 未提供有效的登录凭证".into(),
            ));
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
                Some(&effective_credential),
                opts.platform,
                opts.override_comm,
                opts.sign,
            )
            .await?;

        let headers = self.cgi_headers(&url, user_agent, Some(&effective_credential));
        let mut request = TransportRequest::new(HttpMethod::Post, url);
        request.headers = headers;
        request.query = query_params;
        request.body = HttpBody::Json(payload);
        request.retry = opts.retry;
        request.redirects = RedirectMode::FollowValidated;
        request.cancellation = opts.cancellation.clone();
        let resp = self.execute_transport(request).await?;
        let status = resp.status;
        let text = resp.text();
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

        let effective_credential = opts.credential.clone().unwrap_or_else(|| self.credential());
        if opts.require_login
            && (effective_credential.musicid == 0 || effective_credential.musickey.is_empty())
        {
            return Err(QmError::CredentialInvalid(
                "请求需要登录, 未提供有效的登录凭证".into(),
            ));
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
                Some(&effective_credential),
                opts.platform,
                opts.override_comm,
                opts.sign,
            )
            .await?;

        let headers = self.cgi_headers(&url, user_agent, Some(&effective_credential));
        let mut request = TransportRequest::new(HttpMethod::Post, url);
        request.headers = headers;
        request.query = query_params;
        request.body = HttpBody::Json(payload);
        request.retry = opts.retry;
        request.redirects = RedirectMode::FollowValidated;
        request.cancellation = opts.cancellation.clone();
        let resp = self.execute_transport(request).await?;
        let status = resp.status;
        let text = resp.text();
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
    ///
    /// `credential == None` 表示匿名下载; 不会读取全局账号凭证.
    pub async fn request_http_bytes(
        &self,
        url: &str,
        credential: Option<&Credential>,
    ) -> Result<Vec<u8>> {
        self.limiter.acquire().await;
        let (headers, cookies) = self.prepare_http_kwargs(credential, Vec::new(), Vec::new());
        let mut request = TransportRequest::new(HttpMethod::Get, url);
        request.headers = merge_cookie_headers(headers, cookies);
        request.retry = RetryClass::SafeRead;
        let resp = self.execute_transport(request).await?;
        if resp.status != 200 {
            return Err(QmError::http(resp.status, resp.text()));
        }
        Ok(resp.body)
    }

    /// 执行标准 HTTP 请求, 返回完整响应 (含状态码 / 最终 URL / 头 / 体).
    ///
    /// `opts.credential == None` 表示匿名请求; 不会读取全局账号凭证.
    pub async fn request_http_raw(
        &self,
        method: HttpMethod,
        url: &str,
        opts: &crate::client::HttpOptions,
    ) -> Result<TransportResponse> {
        self.limiter.acquire().await;
        let (headers, cookies) = self.prepare_http_kwargs(
            opts.credential.as_ref(),
            opts.headers.clone(),
            opts.cookies.clone(),
        );
        let body = if let Some(json) = &opts.json {
            HttpBody::Json(json.clone())
        } else if let Some(data) = &opts.data {
            HttpBody::Form(data.clone())
        } else if let Some(raw) = &opts.body {
            HttpBody::Bytes(raw.clone())
        } else {
            HttpBody::Empty
        };
        let mut request = TransportRequest::new(method, url);
        request.headers = merge_cookie_headers(headers, cookies);
        request.query = opts.params.clone();
        request.body = body;
        request.timeout = opts.timeout;
        request.retry = opts.retry;
        request.redirects = opts.redirects;
        request.cancellation = opts.cancellation.clone();
        self.execute_transport(request).await
    }

    /// 执行标准 HTTP 请求, 返回原始响应文本.
    pub async fn request_http(
        &self,
        method: HttpMethod,
        url: &str,
        opts: &crate::client::HttpOptions,
    ) -> Result<String> {
        let resp = self.request_http_raw(method, url, opts).await?;
        if resp.status != 200 {
            return Err(QmError::http(resp.status, resp.text()));
        }
        Ok(resp.text())
    }
}

fn merge_cookie_headers(
    mut headers: Vec<(String, String)>,
    cookies: Vec<(String, String)>,
) -> Vec<(String, String)> {
    if !cookies.is_empty() {
        let cookie = cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ");
        headers.push(("Cookie".into(), cookie));
    }
    headers
}

fn has_header(headers: &[(String, String)], name: &str) -> bool {
    headers
        .iter()
        .any(|(key, _)| key.eq_ignore_ascii_case(name))
}

fn is_yqq_cgi_url(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    let host = parsed.host_str().unwrap_or("");
    if matches!(host, "u.y.qq.com" | "c.y.qq.com" | "c6.y.qq.com") {
        return true;
    }
    let loopback = host == "127.0.0.1" || host == "localhost" || host == "::1";
    loopback
        && (parsed.path().contains("/cgi-bin")
            || parsed.path().contains("musicu.fcg")
            || parsed.path().contains("musics.fcg"))
}

/// 对 QQ 音乐 CGI host 补浏览器头. 已有 Referer/Origin (如 ptlogin) 不覆盖.
fn ensure_yqq_cgi_headers(url: &str, headers: &mut Vec<(String, String)>) {
    if !is_yqq_cgi_url(url) {
        return;
    }
    if !has_header(headers, "referer") {
        headers.push(("Referer".into(), "https://y.qq.com/".into()));
    }
    if !has_header(headers, "origin") {
        headers.push(("Origin".into(), "https://y.qq.com".into()));
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

    #[test]
    fn generic_http_none_does_not_inherit_global_credential() {
        let ctx = ApiContext::new_with_proxy(None, Some(Platform::Web), None).unwrap();
        ctx.set_credential(Credential {
            musicid: 10001,
            str_musicid: "10001".into(),
            musickey: "secret-key".into(),
            ..Default::default()
        });
        let (_headers, cookies) = ctx.prepare_http_kwargs(None, Vec::new(), Vec::new());
        assert!(cookies.is_empty());
    }

    #[test]
    fn generic_http_explicit_credential_still_adds_cookies() {
        let ctx = ApiContext::new_with_proxy(None, Some(Platform::Web), None).unwrap();
        let credential = Credential {
            musicid: 10001,
            str_musicid: "10001".into(),
            musickey: "secret-key".into(),
            ..Default::default()
        };
        let (_headers, cookies) =
            ctx.prepare_http_kwargs(Some(&credential), Vec::new(), Vec::new());
        assert!(cookies.iter().any(|(k, v)| k == "uin" && v == "10001"));
        assert!(cookies
            .iter()
            .any(|(k, v)| k == "qm_keyst" && v == "secret-key"));
    }

    // ------------------------------------------------------------------
    // contract test harness: 本地 mock MusicU 服务器 + 状态缓存并发.
    // ------------------------------------------------------------------

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
    fn yqq_cgi_gets_referer_and_origin_when_missing() {
        let mut headers = vec![("User-Agent".into(), "x".into())];
        ensure_yqq_cgi_headers("https://u.y.qq.com/cgi-bin/musicu.fcg", &mut headers);
        assert!(headers.iter().any(
            |(key, value)| key.eq_ignore_ascii_case("referer") && value == "https://y.qq.com/"
        ));
        assert!(headers
            .iter()
            .any(|(key, value)| key.eq_ignore_ascii_case("origin") && value == "https://y.qq.com"));
    }

    #[test]
    fn existing_ptlogin_referer_is_not_replaced() {
        let mut headers = vec![("Referer".into(), "https://xui.ptlogin2.qq.com/".into())];
        ensure_yqq_cgi_headers("https://u.y.qq.com/cgi-bin/musicu.fcg", &mut headers);
        assert_eq!(
            headers
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case("referer"))
                .map(|(_, value)| value.as_str()),
            Some("https://xui.ptlogin2.qq.com/")
        );
        assert!(headers
            .iter()
            .any(|(key, value)| key.eq_ignore_ascii_case("origin") && value == "https://y.qq.com"));
    }

    #[test]
    fn ptlogin_host_does_not_get_yqq_cgi_headers() {
        let mut headers = vec![("User-Agent".into(), "x".into())];
        ensure_yqq_cgi_headers("https://ssl.ptlogin2.qq.com/ptqrshow", &mut headers);
        assert!(!headers
            .iter()
            .any(|(key, _)| key.eq_ignore_ascii_case("referer")));
        assert!(!headers
            .iter()
            .any(|(key, _)| key.eq_ignore_ascii_case("origin")));
    }

    #[derive(Default)]
    struct CapturedCgi {
        referer: Option<String>,
        origin: Option<String>,
        cookie: Option<String>,
        uri: String,
        body: Value,
    }

    async fn spawn_capturing_cgi(cap: std::sync::Arc<std::sync::Mutex<CapturedCgi>>) -> String {
        use axum::{extract::OriginalUri, http::HeaderMap, routing::post, Json, Router};
        let app = Router::new().route(
            "/cgi-bin/musicu.fcg",
            post({
                let cap = cap.clone();
                move |headers: HeaderMap, uri: OriginalUri, Json(body): Json<Value>| {
                    let cap = cap.clone();
                    async move {
                        let mut seen = cap.lock().unwrap();
                        seen.referer = headers
                            .get("referer")
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_string);
                        seen.origin = headers
                            .get("origin")
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_string);
                        seen.cookie = headers
                            .get("cookie")
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_string);
                        seen.uri = uri.0.to_string();
                        seen.body = body;
                        r#"{"code":0,"req_0":{"code":0,"data":{"lyric":"[ti:x]"}}}"#
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn request_cgi_sends_yqq_referer_and_origin() {
        let cap = std::sync::Arc::new(std::sync::Mutex::new(CapturedCgi::default()));
        let base = spawn_capturing_cgi(cap.clone()).await;
        let mut ctx = ApiContext::new_with_proxy(None, Some(Platform::Web), None).unwrap();
        ctx.cgi_base_url = format!("{base}/cgi-bin");
        ctx.request_cgi(
            "music.musichallSong.PlayLyricInfo",
            "GetPlayLyricInfo",
            json!({ "songMID": "001X" }),
            &RequestOptions::default(),
        )
        .await
        .unwrap();
        let seen = cap.lock().unwrap();
        assert_eq!(seen.referer.as_deref(), Some("https://y.qq.com/"));
        assert_eq!(seen.origin.as_deref(), Some("https://y.qq.com"));
    }

    #[tokio::test]
    async fn request_cgi_sends_credential_cookies() {
        let cap = std::sync::Arc::new(std::sync::Mutex::new(CapturedCgi::default()));
        let base = spawn_capturing_cgi(cap.clone()).await;
        let mut ctx = ApiContext::new_with_proxy(None, Some(Platform::Web), None).unwrap();
        ctx.cgi_base_url = format!("{base}/cgi-bin");
        ctx.set_credential(Credential {
            musicid: 10001,
            str_musicid: "10001".into(),
            musickey: "test-key".into(),
            ..Default::default()
        });
        ctx.request_cgi(
            "music.musichallSong.PlayLyricInfo",
            "GetPlayLyricInfo",
            json!({ "songMID": "001X" }),
            &RequestOptions::default(),
        )
        .await
        .unwrap();
        let cookie = cap.lock().unwrap().cookie.clone().expect("Cookie header");
        assert!(cookie.contains("uin=10001"), "{cookie}");
        assert!(cookie.contains("qqmusic_uin=10001"), "{cookie}");
        assert!(cookie.contains("qm_keyst=test-key"), "{cookie}");
        assert!(cookie.contains("qqmusic_key=test-key"), "{cookie}");
    }

    #[test]
    fn model_schema_drift_uses_defaults() {
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
        let s2 = ctx.session_for(Platform::Android, &cred).await.unwrap();
        assert_eq!(s1.uid, s2.uid);
        assert_eq!(s1.uid, "u1");
    }

    #[tokio::test]
    async fn session_cached_per_account() {
        use axum::routing::post;
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

        let mut cred_a = Credential::default();
        cred_a.musicid = 111;
        cred_a.str_musicid = "111".into();
        let a = ctx.session_for(Platform::Android, &cred_a).await.unwrap();
        assert_eq!(a.uid, "new-uid");

        let mut cred_b = Credential::default();
        cred_b.musicid = 222;
        cred_b.str_musicid = "222".into();
        let b = ctx.session_for(Platform::Android, &cred_b).await.unwrap();
        assert_eq!(b.uid, "new-uid");

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

        let epoch_before = ctx.device_epoch.load(AOrdering::Relaxed);
        ctx.set_device(Device::random());
        assert_eq!(ctx.device_epoch.load(AOrdering::Relaxed), epoch_before + 1);

        let s2 = ctx.session_for(Platform::Android, &cred).await.unwrap();
        assert_eq!(s2.uid, "u1");
        assert_ne!(s1.device_epoch, s2.device_epoch);
    }

    fn seed_qimei(ctx: &ApiContext, q16: &str, q36: &str) {
        let mut device = ctx.device();
        device.qimei = Some(q16.into());
        device.qimei36 = Some(q36.into());
        device.qimei_save_time = Some(now());
        ctx.set_device(device);
    }

    async fn spawn_dual(
        cgi: axum::routing::MethodRouter,
        qimei: axum::routing::MethodRouter,
    ) -> String {
        use axum::Router;
        let app = Router::new()
            .route("/cgi-bin/musicu.fcg", cgi)
            .route("/tme/trpc/proxy", qimei);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn stale_qimei_is_not_committed_after_set_device() {
        use axum::routing::post;
        use std::sync::atomic::{AtomicU32, Ordering as AOrd};
        use std::sync::Arc;
        use tokio::sync::Barrier;

        let gate = Arc::new(Barrier::new(2));
        let hits = Arc::new(AtomicU32::new(0));
        let qimei = {
            let gate = gate.clone();
            let hits = hits.clone();
            post(move || {
                let gate = gate.clone();
                let hits = hits.clone();
                async move {
                    let n = hits.fetch_add(1, AOrd::SeqCst);
                    if n == 0 {
                        gate.wait().await;
                        gate.wait().await;
                        r#"{"data":"{\"data\":{\"q16\":\"q16-d0\",\"q36\":\"q36-d0\"}}"}"#
                    } else {
                        r#"{"data":"{\"data\":{\"q16\":\"q16-d1\",\"q36\":\"q36-d1\"}}"}"#
                    }
                }
            })
        };
        let cgi = post(|| async {
            r#"{"code":0,"req_0":{"code":0,"data":{"session":{"uid":"u1","sid":"s1"}}}}"#
        });
        let base = spawn_dual(cgi, qimei).await;

        let ctx = Arc::new({
            let mut ctx = ApiContext::new_with_proxy(None, Some(Platform::Android), None).unwrap();
            ctx.cgi_base_url = format!("{base}/cgi-bin");
            ctx.qimei_url = format!("{base}/tme/trpc/proxy");
            let mut d0 = ctx.device();
            d0.android_id = "aid-d0".into();
            d0.qimei = None;
            d0.qimei36 = None;
            d0.qimei_save_time = None;
            ctx.set_device(d0);
            ctx
        });

        let task = {
            let ctx = ctx.clone();
            tokio::spawn(async move { ctx.get_cached_qimei().await })
        };
        gate.wait().await;
        let mut d1 = Device::random();
        d1.android_id = "aid-d1".into();
        d1.qimei = None;
        d1.qimei36 = None;
        d1.qimei_save_time = None;
        ctx.set_device(d1);
        gate.wait().await;

        let got = task.await.unwrap().unwrap();
        assert_eq!(got, Some(("q16-d1".into(), "q36-d1".into())));
        assert_eq!(ctx.qimei(), Some(("q16-d1".into(), "q36-d1".into())));
        assert_ne!(ctx.qimei(), Some(("q16-d0".into(), "q36-d0".into())));
        assert_eq!(ctx.device().android_id, "aid-d1");
        assert!(
            hits.load(AOrd::SeqCst) >= 2,
            "stale result must trigger retry"
        );
    }

    #[tokio::test]
    async fn stale_session_is_not_cached_for_new_device() {
        use axum::routing::post;
        use std::sync::atomic::{AtomicU32, Ordering as AOrd};
        use std::sync::Arc;
        use tokio::sync::Barrier;

        let gate = Arc::new(Barrier::new(2));
        let hits = Arc::new(AtomicU32::new(0));
        let cgi = {
            let gate = gate.clone();
            let hits = hits.clone();
            post(move || {
                let gate = gate.clone();
                let hits = hits.clone();
                async move {
                    let n = hits.fetch_add(1, AOrd::SeqCst);
                    if n == 0 {
                        gate.wait().await;
                        gate.wait().await;
                        r#"{"code":0,"req_0":{"code":0,"data":{"session":{"uid":"u-d0","sid":"s-d0"}}}}"#
                    } else {
                        r#"{"code":0,"req_0":{"code":0,"data":{"session":{"uid":"u-d1","sid":"s-d1"}}}}"#
                    }
                }
            })
        };
        let qimei = post(|| async { r#"{"data":"{\"data\":{\"q16\":\"q16\",\"q36\":\"q36\"}}"}"# });
        let base = spawn_dual(cgi, qimei).await;

        let ctx = Arc::new({
            let mut ctx = ApiContext::new_with_proxy(None, Some(Platform::Android), None).unwrap();
            ctx.cgi_base_url = format!("{base}/cgi-bin");
            ctx.qimei_url = format!("{base}/tme/trpc/proxy");
            seed_qimei(&ctx, "q16-d0", "q36-d0");
            ctx
        });
        let epoch_d0 = ctx.current_epoch();

        let mut cred = Credential::default();
        cred.musicid = 9;
        cred.str_musicid = "9".into();

        let task = {
            let ctx = ctx.clone();
            let cred = cred.clone();
            tokio::spawn(async move { ctx.session_for(Platform::Android, &cred).await })
        };
        gate.wait().await;
        let mut d1 = Device::random();
        d1.android_id = "aid-d1".into();
        d1.qimei = Some("q16-d1".into());
        d1.qimei36 = Some("q36-d1".into());
        d1.qimei_save_time = Some(now());
        ctx.set_device(d1);
        let epoch_d1 = ctx.current_epoch();
        assert_ne!(epoch_d0, epoch_d1);
        gate.wait().await;

        let session = task.await.unwrap().unwrap();
        assert_eq!(session.uid, "u-d1");
        assert_eq!(session.device_epoch, epoch_d1);
        assert_ne!(session.device_epoch, epoch_d0);
        assert!(hits.load(AOrd::SeqCst) >= 2);
    }

    #[tokio::test]
    async fn build_api_kwargs_retries_for_coherent_device_snapshot() {
        use axum::routing::post;
        use std::sync::atomic::{AtomicU32, Ordering as AOrd};
        use std::sync::Arc;
        use tokio::sync::Barrier;

        let gate = Arc::new(Barrier::new(2));
        let hits = Arc::new(AtomicU32::new(0));
        let cgi = {
            let gate = gate.clone();
            let hits = hits.clone();
            post(move || {
                let gate = gate.clone();
                let hits = hits.clone();
                async move {
                    let n = hits.fetch_add(1, AOrd::SeqCst);
                    if n == 0 {
                        gate.wait().await;
                        gate.wait().await;
                        r#"{"code":0,"req_0":{"code":0,"data":{"session":{"uid":"u-d0","sid":"s-d0"}}}}"#
                    } else {
                        r#"{"code":0,"req_0":{"code":0,"data":{"session":{"uid":"u-d1","sid":"s-d1"}}}}"#
                    }
                }
            })
        };
        let qimei = post(|| async { r#"{"data":"{\"data\":{\"q16\":\"x\",\"q36\":\"y\"}}"}"# });
        let base = spawn_dual(cgi, qimei).await;

        let ctx = Arc::new({
            let mut ctx = ApiContext::new_with_proxy(None, Some(Platform::Android), None).unwrap();
            ctx.cgi_base_url = format!("{base}/cgi-bin");
            ctx.qimei_url = format!("{base}/tme/trpc/proxy");
            let mut d0 = ctx.device();
            d0.android_id = "aid-d0".into();
            d0.qimei = Some("q16-d0".into());
            d0.qimei36 = Some("q36-d0".into());
            d0.qimei_save_time = Some(now());
            ctx.set_device(d0);
            ctx
        });

        let mut cred = Credential::default();
        cred.musicid = 11;
        cred.str_musicid = "11".into();

        let task = {
            let ctx = ctx.clone();
            let cred = cred.clone();
            tokio::spawn(async move {
                ctx.build_api_kwargs(
                    &[json!({"module": "music.test", "method": "Ping", "param": {}})],
                    None,
                    Some(&cred),
                    Some(Platform::Android),
                    false,
                    false,
                )
                .await
            })
        };
        gate.wait().await;
        let mut d1 = Device::random();
        d1.android_id = "aid-d1".into();
        d1.qimei = Some("q16-d1".into());
        d1.qimei36 = Some("q36-d1".into());
        d1.qimei_save_time = Some(now());
        ctx.set_device(d1);
        gate.wait().await;

        let (_url, payload, _params, _ua) = task.await.unwrap().unwrap();
        let comm = &payload["comm"];
        assert_eq!(comm["aid"], "aid-d1");
        assert_eq!(comm["QIMEI"], "q16-d1");
        assert_eq!(comm["QIMEI36"], "q36-d1");
        assert_eq!(comm["uid"], "u-d1");
        assert_ne!(comm["aid"], "aid-d0");
        assert_ne!(comm["QIMEI"], "q16-d0");
        assert_ne!(comm["uid"], "u-d0");
    }
}
