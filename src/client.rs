//! API 客户端核心实现 (对应 Python 端 `core/client.py`).

use serde::de::DeserializeOwned;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

use crate::context::ApiContext;
use crate::error::{QmError, Result};
use crate::modules::*;
use crate::reply::CgiReply;
use crate::transport::{
    ApiTransport, CancellationToken, HttpMethod, RedirectMode, RetryClass, TransportConfig,
};
use crate::versioning::Platform;
use crate::Credential;

/// CGI 请求选项.
#[derive(Debug, Clone)]
pub struct CgiOptions {
    /// 自定义公共参数 (与默认 comm 合并).
    pub comm: Option<Value>,
    /// 是否完全覆盖默认公共参数.
    pub override_comm: bool,
    /// 是否保留布尔值 (为 false 时转换为 0/1).
    pub preserve_bool: bool,
    /// 请求凭证 (优先于客户端全局凭证).
    pub credential: Option<Credential>,
    /// 请求平台 (优先于客户端全局平台).
    pub platform: Option<Platform>,
    /// 是否需要签名 (走 `musics.fcg`).
    pub sign: bool,
    /// 是否需要登录.
    pub require_login: bool,
    /// 传输层重试类别. 写操作应设为 [`RetryClass::Write`].
    pub retry: RetryClass,
    /// 请求取消令牌. 见 `transport` 模块文档.
    pub cancellation: CancellationToken,
}

impl CgiOptions {
    /// 构造默认选项.
    pub fn new() -> Self {
        CgiOptions::default()
    }
}

impl Default for CgiOptions {
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

impl From<&CgiOptions> for crate::context::RequestOptions {
    fn from(o: &CgiOptions) -> Self {
        crate::context::RequestOptions {
            comm: o.comm.clone(),
            override_comm: o.override_comm,
            preserve_bool: o.preserve_bool,
            credential: o.credential.clone(),
            platform: o.platform,
            sign: o.sign,
            require_login: o.require_login,
            retry: o.retry,
            cancellation: o.cancellation.clone(),
        }
    }
}

/// HTTP 请求选项.
///
/// 与 CGI 请求不同, 通用 HTTP 请求在未显式提供 `credential` 时按匿名请求处理,
/// 不会自动继承 [`Client`] 的全局账号凭证. 需要鉴权时请显式设置
/// `credential: Some(...)`.
#[derive(Clone)]
pub struct HttpOptions {
    pub params: Vec<(String, String)>,
    /// 普通 header 列表, 不含 `reqwest::header::HeaderMap`.
    pub headers: Vec<(String, String)>,
    pub cookies: Vec<(String, String)>,
    pub json: Option<Value>,
    pub data: Option<Value>,
    /// 原始字节体 (JSON / form 优先).
    pub body: Option<Vec<u8>>,
    /// 通用 HTTP 请求的显式凭证. `None` 表示匿名, 不继承全局凭证.
    pub credential: Option<Credential>,
    /// 覆盖默认总超时 (connect 超时仍由 transport 配置决定).
    pub timeout: Option<Duration>,
    pub retry: RetryClass,
    pub redirects: RedirectMode,
    pub cancellation: CancellationToken,
}

impl std::fmt::Debug for HttpOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let param_keys: Vec<&str> = self.params.iter().map(|(key, _)| key.as_str()).collect();
        let header_names: Vec<&str> = self.headers.iter().map(|(key, _)| key.as_str()).collect();
        let cookie_names: Vec<&str> = self.cookies.iter().map(|(key, _)| key.as_str()).collect();
        let body_len = self.body.as_ref().map(Vec::len);

        f.debug_struct("HttpOptions")
            .field("param_keys", &param_keys)
            .field("header_names", &header_names)
            .field("cookie_names", &cookie_names)
            .field("has_json", &self.json.is_some())
            .field("has_data", &self.data.is_some())
            .field("body_len", &body_len)
            .field("credential", &self.credential)
            .field("timeout", &self.timeout)
            .field("retry", &self.retry)
            .field("redirects", &self.redirects)
            .finish_non_exhaustive()
    }
}

impl Default for HttpOptions {
    fn default() -> Self {
        Self {
            params: Vec::new(),
            headers: Vec::new(),
            cookies: Vec::new(),
            json: None,
            data: None,
            body: None,
            credential: None,
            timeout: None,
            retry: RetryClass::SafeRead,
            redirects: RedirectMode::FollowValidated,
            cancellation: CancellationToken::new(),
        }
    }
}

/// QQMusic API 客户端.
#[derive(Clone)]
pub struct Client {
    pub(crate) context: Arc<ApiContext>,
    pub song: SongApi,
    pub search: SearchApi,
    pub singer: SingerApi,
    pub album: AlbumApi,
    pub lyric: LyricApi,
    pub mv: MvApi,
    pub top: TopApi,
    pub songlist: SonglistApi,
    pub comment: CommentApi,
    pub recommend: RecommendApi,
    pub user: UserApi,
    pub login: LoginApi,
    pub helper: HelperApi,
    pub private_message: PrivateMessageApi,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("platform", &self.context.platform)
            .finish()
    }
}

impl Client {
    /// 创建客户端.
    pub fn new(credential: Option<Credential>, platform: Option<Platform>) -> Result<Self> {
        Self::new_with_proxy(credential, platform, None)
    }

    /// 创建客户端 (可指定 HTTP 代理, 如 `"http://127.0.0.1:7890"`).
    pub fn new_with_proxy(
        credential: Option<Credential>,
        platform: Option<Platform>,
        proxy: Option<&str>,
    ) -> Result<Self> {
        Ok(Self::from_context(Arc::new(ApiContext::new_with_proxy(
            credential, platform, proxy,
        )?)))
    }

    /// 使用指定的默认 transport 配置创建客户端.
    pub fn new_with_transport_config(
        credential: Option<Credential>,
        platform: Option<Platform>,
        config: TransportConfig,
    ) -> Result<Self> {
        Ok(Self::from_context(Arc::new(
            ApiContext::new_with_transport_config(credential, platform, config)?,
        )))
    }

    /// 注入自定义 [`ApiTransport`].
    pub fn new_with_transport(
        credential: Option<Credential>,
        platform: Option<Platform>,
        transport: Arc<dyn ApiTransport>,
    ) -> Self {
        Self::from_context(Arc::new(ApiContext::new_with_transport(
            credential, platform, transport,
        )))
    }

    fn from_context(context: Arc<ApiContext>) -> Self {
        Client {
            song: SongApi::new(context.clone()),
            search: SearchApi::new(context.clone()),
            singer: SingerApi::new(context.clone()),
            album: AlbumApi::new(context.clone()),
            lyric: LyricApi::new(context.clone()),
            mv: MvApi::new(context.clone()),
            top: TopApi::new(context.clone()),
            songlist: SonglistApi::new(context.clone()),
            comment: CommentApi::new(context.clone()),
            recommend: RecommendApi::new(context.clone()),
            user: UserApi::new(context.clone()),
            login: LoginApi::new(context.clone()),
            helper: HelperApi::new(context.clone()),
            private_message: PrivateMessageApi::new(context.clone()),
            context,
        }
    }

    /// 全局凭证 (只读).
    pub fn credential(&self) -> Credential {
        self.context.credential()
    }

    /// 设置全局凭证.
    pub fn set_credential(&self, credential: Credential) {
        self.context.set_credential(credential);
    }

    /// 全局默认平台.
    pub fn platform(&self) -> Platform {
        self.context.platform
    }

    /// 底层上下文 (用于自定义请求).
    pub fn context(&self) -> &ApiContext {
        &self.context
    }

    /// 将当前设备指纹持久化到文件.
    ///
    /// 仅包含**设备身份** (android_id/imei/open_udid/QIMEI 等), 不含
    /// Android session —— session 是账号运行态, 按账号缓存在内存中,
    /// 不随设备持久化.
    pub fn save_device(&self, path: &std::path::Path) -> Result<()> {
        let device = self.context.device();
        let bytes = serde_json::to_vec(&device).map_err(QmError::from)?;
        std::fs::write(path, bytes).map_err(QmError::from)
    }

    /// 从文件加载设备指纹.
    ///
    /// 更换设备身份会使既有 Android session 缓存失效 (下次按需重新申请).
    pub fn load_device(&self, path: &std::path::Path) -> Result<()> {
        let bytes = std::fs::read(path).map_err(QmError::from)?;
        let device: crate::Device = serde_json::from_slice(&bytes).map_err(QmError::from)?;
        self.context.set_device(device);
        Ok(())
    }

    /// 执行一个 CGI 请求并返回固定形状的响应 `CgiReply { code, data }`.
    ///
    /// transport 层不解释业务错误码; 需要"成功才返回数据"的调用方应使用
    /// `cgi` / `cgi_typed`, 需要解释特殊状态码的调用方可直接读取 `code`.
    pub async fn request_cgi(
        &self,
        module: &str,
        method: &str,
        param: Value,
        opts: &CgiOptions,
    ) -> Result<CgiReply<Value>> {
        let ro: crate::context::RequestOptions = opts.into();
        self.context.request_cgi(module, method, param, &ro).await
    }

    /// 批量执行多个 CGI 请求 (合并为一次 `req_0..req_N` 调用, 减少网络往返).
    ///
    /// `requests` 为 `(module, method, param)` 三元组列表; 返回与输入顺序一致的
    /// 每个子请求的 `CgiReply { code, data }`. 单个子请求的业务错误码不会导致
    /// 整个批量请求失败.
    pub async fn request_cgi_batch(
        &self,
        requests: &[(&str, &str, Value)],
        opts: &CgiOptions,
    ) -> Result<Vec<CgiReply<Value>>> {
        let ro: crate::context::RequestOptions = opts.into();
        self.context.request_cgi_batch(requests, &ro).await
    }

    /// 批量执行 CGI 请求并反序列化为 `Vec<T>`, 任一子请求 `code != 0` 时失败.
    pub async fn cgi_batch<T: DeserializeOwned>(
        &self,
        requests: &[(&str, &str, Value)],
        opts: &CgiOptions,
    ) -> Result<Vec<T>> {
        let replies = self.request_cgi_batch(requests, opts).await?;
        replies
            .into_iter()
            .map(|reply| reply.into_typed::<T>())
            .collect()
    }

    /// 执行一个 CGI 请求并反序列化为 `T`, `code != 0` 时失败.
    pub async fn cgi<T: DeserializeOwned>(
        &self,
        module: &str,
        method: &str,
        param: Value,
        opts: &CgiOptions,
    ) -> Result<T> {
        let reply = self.request_cgi(module, method, param, opts).await?;
        reply.into_typed::<T>()
    }

    /// 执行一个标准 HTTP 请求, 返回原始响应文本.
    ///
    /// 安全默认: `opts.credential == None` 时发送匿名请求, 不继承全局凭证.
    pub async fn request_http(
        &self,
        method: HttpMethod,
        url: &str,
        opts: &HttpOptions,
    ) -> Result<String> {
        let mut safe_opts = opts.clone();
        if safe_opts.credential.is_none() {
            // `ApiContext` 仍为 CGI 保留 `None => global credential` 的历史语义.
            // 通用 HTTP 边界显式传入空凭证, 阻止隐式账号 Cookie 传播.
            safe_opts.credential = Some(Credential::default());
        }
        self.context.request_http(method, url, &safe_opts).await
    }

    /// 执行 HTTP 请求并反序列化.
    pub async fn http<T: DeserializeOwned>(
        &self,
        method: HttpMethod,
        url: &str,
        opts: &HttpOptions,
    ) -> Result<T> {
        let text = self.request_http(method, url, opts).await?;
        serde_json::from_str(&text).map_err(QmError::from)
    }

    /// 下载原始字节 (用于音频文件下载).
    ///
    /// 安全默认: `credential == None` 表示匿名下载. 如确需账号 Cookie, 必须显式
    /// 传入凭证, 例如先读取 `let cred = client.credential()` 再传 `Some(&cred)`.
    pub async fn download(&self, url: &str, credential: Option<&Credential>) -> Result<Vec<u8>> {
        match credential {
            Some(credential) => self.context.request_http_bytes(url, Some(credential)).await,
            None => {
                let anonymous = Credential::default();
                self.context.request_http_bytes(url, Some(&anonymous)).await
            }
        }
    }
}
