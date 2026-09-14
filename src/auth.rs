//! Attempt-scoped OAuth exchange. Browser callback validation, attempt ownership
//! and persistence belong to the host; QQ protocol and wire cookies live here.
use crate::{
    modules::login::{build_oauth_code_exchange_request, credential_from_login_data},
    CancellationToken, Client, Credential, HttpMethod, HttpOptions, OAuthLoginProvider, QmError,
    Result, RetryClass,
};
use serde_json::{json, Value};
use std::{collections::BTreeMap, fmt};

pub const MAX_OAUTH_RESPONSE_BYTES: usize = 256 * 1024;
const MAX_COOKIE_BYTES: usize = 16 * 1024;
const FALLBACK_LIFETIME_MS: u64 = 24 * 60 * 60 * 1000;
const EXCHANGE_URL: &str = "https://u.y.qq.com/cgi-bin/musicu.fcg";

pub struct OAuthExchange<'a> {
    pub provider: OAuthLoginProvider,
    pub code: &'a str,
    pub gtk: Option<u32>,
    /// Cookies belonging exclusively to this login attempt, never a shared jar.
    pub cookie_header: &'a str,
    pub now_ms: u64,
}

impl fmt::Debug for OAuthExchange<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuthExchange")
            .field("provider", &self.provider)
            .field("has_cookies", &!self.cookie_header.is_empty())
            .finish_non_exhaustive()
    }
}

pub struct OAuthSession {
    pub credential: Credential,
    pub cookie_header: String,
    pub expires_at_ms: u64,
}

impl fmt::Debug for OAuthSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuthSession")
            .field("expires_at_ms", &self.expires_at_ms)
            .finish_non_exhaustive()
    }
}

/// One authorization-code exchange. No automatic replay after network failure;
/// cancellation prevents publishing a session, not a remote side effect.
pub async fn exchange_oauth_code(
    client: &Client,
    request: OAuthExchange<'_>,
    cancellation: CancellationToken,
) -> Result<OAuthSession> {
    if cancellation.is_cancelled() {
        return Err(QmError::cancelled());
    }
    if request.code.is_empty()
        || request.code.len() > 2048
        || request.code.bytes().any(|b| b.is_ascii_control())
    {
        return Err(protocol("invalid authorization code"));
    }
    let mut cookies = Cookies::parse(request.cookie_header)?;
    let wire = build_oauth_code_exchange_request(request.provider, request.code, request.gtk);
    let options = HttpOptions {
        json: Some(
            json!({"comm":wire.comm,"req":{"module":wire.module,"method":wire.method,"param":wire.param}}),
        ),
        headers: vec![
            ("Origin".into(), "https://y.qq.com".into()),
            ("Referer".into(), "https://y.qq.com/".into()),
            ("Content-Type".into(), "application/json".into()),
            ("Cookie".into(), cookies.header()),
        ],
        retry: RetryClass::AuthPoll,
        cancellation: cancellation.clone(),
        max_response_bytes: Some(MAX_OAUTH_RESPONSE_BYTES),
        ..HttpOptions::default()
    };
    let response = client
        .context
        .request_http_raw(HttpMethod::Post, EXCHANGE_URL, &options)
        .await?;
    if cancellation.is_cancelled() {
        return Err(QmError::cancelled());
    }
    if !(200..300).contains(&response.status) {
        return Err(protocol("exchange HTTP failure"));
    }
    if url::Url::parse(&response.final_url).ok() != url::Url::parse(EXCHANGE_URL).ok() {
        return Err(protocol("unexpected exchange final URL"));
    }
    if response.body.len() > MAX_OAUTH_RESPONSE_BYTES {
        return Err(protocol("exchange response too large"));
    }
    cookies.absorb(&response.headers)?;
    let payload: Value =
        serde_json::from_slice(&response.body).map_err(|_| malformed("invalid exchange JSON"))?;
    let global = code(&payload)?;
    let reply = payload
        .get("req")
        .or_else(|| payload.get("req_0"))
        .ok_or_else(|| malformed("missing exchange reply"))?;
    let business = code(reply)?;
    if global != 0 || business != 0 {
        return Err(protocol("exchange business failure"));
    }
    let data = reply.get("data").unwrap_or(reply);
    let mut credential =
        credential_from_login_data(data).map_err(|_| malformed("invalid login data"))?;
    let uin = credential.str_musicid();
    let numeric = uin.parse::<i64>().ok().filter(|n| *n > 0);
    if uin.is_empty()
        || !uin.bytes().all(|b| b.is_ascii_digit())
        || numeric.is_none()
        || (credential.musicid != 0 && Some(credential.musicid) != numeric)
    {
        return Err(malformed("invalid or conflicting music identity"));
    }
    credential.musicid = numeric.expect("validated musicid");
    credential.str_musicid = uin.clone();
    if credential.musickey.trim().is_empty() {
        return Err(malformed("missing music key"));
    }
    credential.login_type = match request.provider {
        OAuthLoginProvider::Qq => 2,
        OAuthLoginProvider::Wechat => 1,
    };
    if credential.encrypt_uin.trim().is_empty() {
        credential.encrypt_uin = data
            .get("euin")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .or_else(|| {
                cookies
                    .0
                    .get("euin")
                    .or_else(|| cookies.0.get("encryptUin"))
                    .map(String::as_str)
            })
            .unwrap_or_default()
            .to_owned();
    }
    cookies.set("uin", &format!("o{uin}"))?;
    cookies.set("qqmusic_uin", &uin)?;
    cookies.set("qm_keyst", &credential.musickey)?;
    cookies.set("qqmusic_key", &credential.musickey)?;
    cookies.set("tmeLoginType", &credential.login_type.to_string())?;
    // Do not retain aliases for an earlier account when response identity wins.
    cookies.0.remove("euin");
    cookies.0.remove("encryptUin");
    if !credential.encrypt_uin.is_empty() {
        cookies.set("euin", &credential.encrypt_uin)?;
    }
    cookies.0.remove("qrsig");
    cookies.0.remove("pt_login_sig");
    let expires_at_ms = match (
        u64::try_from(credential.musickey_create_time),
        u64::try_from(credential.key_expires_in),
    ) {
        (Ok(created), Ok(lifetime)) if created > 0 && lifetime > 0 => {
            let created = if created >= 1_000_000_000_000 {
                created
            } else {
                created.saturating_mul(1000)
            };
            created.saturating_add(lifetime.saturating_mul(1000))
        }
        _ => request.now_ms.saturating_add(FALLBACK_LIFETIME_MS),
    };
    credential.expired_at = i64::try_from(expires_at_ms / 1000).unwrap_or(i64::MAX);
    if cancellation.is_cancelled() {
        return Err(QmError::cancelled());
    }
    Ok(OAuthSession {
        credential,
        cookie_header: cookies.header(),
        expires_at_ms,
    })
}

fn protocol(message: &'static str) -> QmError {
    QmError::Protocol {
        stage: "oauth-exchange",
        message: message.into(),
    }
}
fn malformed(message: &'static str) -> QmError {
    QmError::ApiData(message.into())
}
fn code(value: &Value) -> Result<i64> {
    value
        .get("code")
        .and_then(|n| n.as_i64().or_else(|| n.as_str()?.parse().ok()))
        .ok_or_else(|| malformed("missing exchange status"))
}

#[derive(Default)]
struct Cookies(BTreeMap<String, String>);
impl Cookies {
    fn parse(header: &str) -> Result<Self> {
        if header.len() > MAX_COOKIE_BYTES || header.bytes().any(|b| b.is_ascii_control()) {
            return Err(protocol("invalid attempt cookies"));
        }
        let mut cookies = Self::default();
        for part in header.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            let (name, value) = part
                .split_once('=')
                .ok_or_else(|| protocol("invalid attempt cookie"))?;
            if cookies.0.contains_key(name.trim()) {
                return Err(protocol("duplicate attempt cookie"));
            }
            cookies.set(name.trim(), value.trim())?;
        }
        Ok(cookies)
    }
    fn set(&mut self, name: &str, value: &str) -> Result<()> {
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            || value
                .bytes()
                .any(|b| !b.is_ascii() || b.is_ascii_control() || b == b';')
        {
            return Err(protocol("invalid cookie syntax"));
        }
        if value.is_empty() {
            self.0.remove(name);
        } else {
            self.0.insert(name.into(), value.into());
        }
        if self.0.len() > 64
            || self
                .0
                .iter()
                .map(|(k, v)| k.len() + v.len() + 3)
                .sum::<usize>()
                > MAX_COOKIE_BYTES
        {
            return Err(protocol("cookie limit exceeded"));
        }
        Ok(())
    }
    fn absorb(&mut self, headers: &[(String, String)]) -> Result<()> {
        for (_, value) in headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie"))
        {
            if value.bytes().any(|b| b.is_ascii_control()) || value.len() > MAX_COOKIE_BYTES {
                return Err(protocol("invalid response cookie"));
            }
            let (name, value) = value
                .split(';')
                .next()
                .and_then(|s| s.split_once('='))
                .ok_or_else(|| protocol("invalid response cookie"))?;
            self.set(name.trim(), value.trim())?;
        }
        Ok(())
    }
    fn header(&self) -> String {
        self.0
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ")
    }
}
