//! 业务 API 模块.

pub mod album;
pub mod comment;
pub mod helper;
pub mod helper_utils;
pub mod login;
pub mod login_utils;
pub mod lyric;
pub mod mv;
pub mod private_message;
pub mod recommend;
pub mod search;
pub mod singer;
pub mod song;
pub mod songlist;
pub mod top;
pub mod user;

pub use album::AlbumApi;
pub use comment::CommentApi;
pub use helper::HelperApi;
pub use helper_utils::UploadFileSession;
pub use login::LoginApi;
pub use login_utils::{PhoneLoginSession, PollInterval, QRCodeLoginSession};
pub use lyric::LyricApi;
pub use mv::MvApi;
pub use private_message::PrivateMessageApi;
pub use recommend::RecommendApi;
pub use search::SearchApi;
pub use singer::SingerApi;
pub use song::SongApi;
pub use songlist::SonglistApi;
pub use top::TopApi;
pub use user::UserApi;

use serde_json::Value;
use std::sync::Arc;

use crate::context::{ApiContext, RequestOptions};
use crate::error::Result;
use crate::models::Credential;
use crate::versioning::Platform;

/// API 模块基类.
#[derive(Clone)]
pub struct ApiModule {
    pub(crate) context: Arc<ApiContext>,
}

impl std::fmt::Debug for ApiModule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiModule").finish()
    }
}

impl ApiModule {
    pub(crate) fn new(context: Arc<ApiContext>) -> Self {
        ApiModule { context }
    }

    /// 构建查询接口使用的版本参数.
    pub(crate) fn build_version_params(&self, platform: Option<Platform>) -> Value {
        let profile = self
            .context
            .version_policy
            .get_profile(platform.unwrap_or(self.context.platform));
        serde_json::json!({ "ct": profile.ct, "cv": profile.cv })
    }

    /// 发送 CGI 请求, 返回 `req_0.data`.
    ///
    /// `code != 0` 时映射为错误 (2000/2001/1000/104401/104400 有专用错误类型).
    pub(crate) async fn cgi(
        &self,
        module: &str,
        method: &str,
        param: Value,
        opts: RequestOptions,
    ) -> Result<Value> {
        let reply = self.context.request_cgi(module, method, param, &opts).await?;
        reply.require_success()
    }

    /// 发送 CGI 请求, 返回固定形状的 `CgiReply { code, data }`.
    ///
    /// 用于需要解释特殊业务状态码的接口 (如登录流程).
    pub(crate) async fn cgi_reply(
        &self,
        module: &str,
        method: &str,
        param: Value,
        opts: RequestOptions,
    ) -> Result<crate::reply::CgiReply<Value>> {
        self.context.request_cgi(module, method, param, &opts).await
    }

    /// 发送 HTTP 请求 (返回解析后的 JSON 值).
    #[allow(dead_code)]
    pub(crate) async fn http(
        &self,
        method: reqwest::Method,
        url: &str,
        opts: crate::client::HttpOptions,
    ) -> Result<Value> {
        let text = self.context.request_http(method, url, &opts).await?;
        Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    /// 获取当前凭证.
    pub(crate) fn credential(&self) -> Credential {
        self.context.credential()
    }
}
