//! Attempt-scoped OAuth exchange. Browser callback validation, attempt ownership
//! and persistence belong to the host; QQ protocol and wire cookies live here.
use crate::{
    hash33,
    modules::login::{build_oauth_code_exchange_request, credential_from_login_data},
    CancellationToken, Client, Credential, HttpMethod, HttpOptions, OAuthLoginProvider, QmError,
    RedirectMode, Result, RetryClass, TransportResponse,
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

// ---------------------------------------------------------------------------
// Desktop QQ sign-in: ptlogin QR challenge -> OAuth authorization code.
//
// The desktop client never redeems the ptlogin ticket directly. A confirmed QR
// is followed by `check_sig` and `graph.qq.com/oauth2.0/authorize`, and the
// resulting authorization code goes through [`exchange_oauth_code`], so the QR
// and the browser-redirect flows produce the same session shape.
// ---------------------------------------------------------------------------

/// Upper bound for the QR image returned by `ptqrshow`.
pub const MAX_DESKTOP_QR_IMAGE_BYTES: usize = 256 * 1024;
/// Upper bound for the textual poll and redirect responses of the QR flow.
pub const MAX_DESKTOP_QR_TEXT_BYTES: usize = 16 * 1024;
/// Lifetime a host should advertise for a freshly created desktop QR challenge.
pub const DESKTOP_QR_LIFETIME_MS: u64 = 2 * 60 * 1000;

const PTLOGIN_REFERER: &str = "https://xui.ptlogin2.qq.com/";
const PTLOGIN_U1: &str = "https://graph.qq.com/oauth2.0/login_jump";
const PTQR_SHOW_URL: &str = "https://ssl.ptlogin2.qq.com/ptqrshow";
const PTQR_LOGIN_URL: &str = "https://ssl.ptlogin2.qq.com/ptqrlogin";
const CHECK_SIG_HOST: &str = "ssl.ptlogin2.graph.qq.com";
const CHECK_SIG_PATH: &str = "/check_sig";
const OAUTH_AUTHORIZE_URL: &str = "https://graph.qq.com/oauth2.0/authorize";
const OAUTH_AUTHORIZE_REFERER: &str = "https://graph.qq.com/";
/// Fixed `surl` query value the browser-redirect login flow always returns to.
pub const OAUTH_CALLBACK_SURL: &str = "https://y.qq.com/";
pub const OAUTH_CALLBACK_URL_PREFIX: &str = "https://y.qq.com/portal/wx_redirect.html";
const OAUTH_REDIRECT_URI: &str =
    "https://y.qq.com/portal/wx_redirect.html?login_type=1&surl=https://y.qq.com/";
const OAUTH_CODE_HOST: &str = "y.qq.com";
const OAUTH_CODE_PATH: &str = "/portal/wx_redirect.html";
const OAUTH_CLIENT_ID: &str = "100497308";

/// Host-visible callback contract for the browser-redirect login flow.
///
/// The host still decides which navigations it accepts, but it must match and
/// validate the callback against this contract instead of duplicating the wire
/// string, so a future redirect-URI change cannot drift out of sync.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OAuthCallbackContract {
    pub host: &'static str,
    pub path: &'static str,
    pub login_type: &'static str,
    pub surl: &'static str,
}

/// Returns the callback contract for `provider`.
pub fn oauth_callback_contract(provider: OAuthLoginProvider) -> OAuthCallbackContract {
    OAuthCallbackContract {
        host: OAUTH_CODE_HOST,
        path: OAUTH_CODE_PATH,
        login_type: match provider {
            OAuthLoginProvider::Qq => "1",
            OAuthLoginProvider::Wechat => "2",
        },
        surl: OAUTH_CALLBACK_SURL,
    }
}

/// The fixed `https://<host><path>` prefix a host callback URL starts with.
pub fn oauth_callback_url_prefix(_provider: OAuthLoginProvider) -> &'static str {
    OAUTH_CALLBACK_URL_PREFIX
}
/// `ptqrtoken` is `hash33(qrsig)` with the default zero seed in both reference
/// clients (L-1124 `utils.hash33(t, h=0)` and wxuyu `loginUtils.hash33`).
const PTQR_TOKEN_SEED: i64 = 0;
/// `g_tk` uses the ptlogin `hash33` variant seeded with 5381.
const GTK_SEED: i64 = 5381;

// ---------------------------------------------------------------------------
// Mobile QQ sign-in: a `display=mobile` QR must be approved inside the phone's
// in-app browser. The library owns the page and the identifier encoding so the
// host never assembles a QQ URL itself.
// ---------------------------------------------------------------------------

/// Upper bound for the QR challenge identifier accepted by
/// [`mobile_qr_launch_url`].
pub const MAX_MOBILE_QR_ID_BYTES: usize = 512;
const MOBILE_QR_LAUNCH_URL: &str = "https://y.qq.com/m/client/qr_code_login/authorize.html";

/// Builds the in-app-browser deep link a phone opens to approve a mobile QR
/// challenge. `identifier` is the value returned alongside the QR image; empty,
/// oversized or control-character identifiers are rejected so the host never
/// hands a malformed string to the system browser.
pub fn mobile_qr_launch_url(identifier: &str) -> Result<String> {
    if identifier.is_empty()
        || identifier.len() > MAX_MOBILE_QR_ID_BYTES
        || identifier.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(malformed("invalid mobile QR identifier"));
    }
    let mut url = url::Url::parse(MOBILE_QR_LAUNCH_URL)
        .map_err(|_| qr_protocol("invalid mobile QR launch url"))?;
    url.query_pairs_mut().append_pair("qrcode_id", identifier);
    Ok(url.to_string())
}

/// A created desktop QR challenge. The image is rendered by the host; `qrsig`
/// is attempt state and must only be handed back to [`poll_desktop_qr`].
pub struct DesktopQrChallenge {
    pub image: Vec<u8>,
    pub mime_type: String,
    pub qrsig: String,
    pub expires_at_ms: u64,
}

impl fmt::Debug for DesktopQrChallenge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DesktopQrChallenge")
            .field("image_len", &self.image.len())
            .field("mime_type", &self.mime_type)
            .field("has_qrsig", &!self.qrsig.is_empty())
            .field("expires_at_ms", &self.expires_at_ms)
            .finish_non_exhaustive()
    }
}

/// One poll of a desktop QR challenge.
pub enum DesktopQrPoll {
    WaitingForScan,
    WaitingForConfirmation,
    Expired,
    Rejected,
    /// The QR was confirmed; the exchanged session is already complete.
    Confirmed(Box<OAuthSession>),
}

impl fmt::Debug for DesktopQrPoll {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::WaitingForScan => "WaitingForScan",
            Self::WaitingForConfirmation => "WaitingForConfirmation",
            Self::Expired => "Expired",
            Self::Rejected => "Rejected",
            Self::Confirmed(_) => "Confirmed",
        };
        f.write_str(label)
    }
}

/// Create a desktop QQ QR challenge. Only the image, MIME type and qrsig needed
/// for the next poll leave this call; other response cookies stay internal.
pub async fn create_desktop_qr(
    client: &Client,
    now_ms: u64,
    cancellation: CancellationToken,
) -> Result<DesktopQrChallenge> {
    if cancellation.is_cancelled() {
        return Err(QmError::cancelled());
    }
    let options = HttpOptions {
        params: vec![
            ("appid".into(), "716027609".into()),
            ("e".into(), "2".into()),
            ("l".into(), "M".into()),
            ("s".into(), "3".into()),
            ("d".into(), "72".into()),
            ("v".into(), "4".into()),
            ("t".into(), format!("0.{now_ms}")),
            ("daid".into(), "383".into()),
            ("pt_3rd_aid".into(), OAUTH_CLIENT_ID.into()),
            ("u1".into(), PTLOGIN_U1.into()),
        ],
        headers: vec![
            ("Referer".into(), PTLOGIN_REFERER.into()),
            // A client may have a populated login jar. QR creation belongs to
            // a new attempt and must not inherit that account.
            ("Cookie".into(), String::new()),
        ],
        max_response_bytes: Some(MAX_DESKTOP_QR_IMAGE_BYTES),
        cancellation: cancellation.clone(),
        ..HttpOptions::default()
    };
    let response = client
        .context
        .request_http_raw(HttpMethod::Get, PTQR_SHOW_URL, &options)
        .await?;
    if cancellation.is_cancelled() {
        return Err(QmError::cancelled());
    }
    if !(200..300).contains(&response.status) {
        return Err(qr_protocol("ptqrshow returned a non-success status"));
    }
    if response.body.is_empty() || response.body.len() > MAX_DESKTOP_QR_IMAGE_BYTES {
        return Err(malformed("empty or oversized QR image"));
    }
    let mime_type = unique_header(&response, "content-type")?
        .and_then(normalize_image_mime)
        .ok_or_else(|| malformed("unsupported QR image type"))?;
    let mut cookies = Cookies::default();
    cookies.absorb(&response.headers)?;
    let qrsig = cookies
        .0
        .get("qrsig")
        .filter(|value| !value.is_empty())
        .cloned()
        .ok_or_else(|| malformed("missing qrsig"))?;
    Ok(DesktopQrChallenge {
        image: response.body,
        mime_type: mime_type.to_owned(),
        qrsig,
        expires_at_ms: now_ms.saturating_add(DESKTOP_QR_LIFETIME_MS),
    })
}

/// Poll a desktop QR challenge once. A confirmed challenge performs the whole
/// `check_sig` -> `authorize` -> code exchange sequence before returning.
///
/// `qrsig` is the attempt secret from [`DesktopQrChallenge`]; only that field
/// is needed, so a host does not have to retain or resend the QR image.
pub async fn poll_desktop_qr(
    client: &Client,
    qrsig: &str,
    now_ms: u64,
    cancellation: CancellationToken,
) -> Result<DesktopQrPoll> {
    if cancellation.is_cancelled() {
        return Err(QmError::cancelled());
    }
    if qrsig.is_empty() || qrsig.len() > 512 || qrsig.bytes().any(|b| b.is_ascii_control()) {
        return Err(qr_protocol("invalid QR attempt secret"));
    }
    let mut cookies = Cookies::default();
    cookies.set("qrsig", qrsig)?;
    let options = HttpOptions {
        params: vec![
            ("u1".into(), PTLOGIN_U1.into()),
            (
                "ptqrtoken".into(),
                hash33(qrsig, PTQR_TOKEN_SEED).to_string(),
            ),
            ("ptredirect".into(), "0".into()),
            ("h".into(), "1".into()),
            ("t".into(), "1".into()),
            ("g".into(), "1".into()),
            ("from_ui".into(), "1".into()),
            ("ptlang".into(), "2052".into()),
            ("action".into(), format!("0-0-{now_ms}")),
            ("js_ver".into(), "20102616".into()),
            ("js_type".into(), "1".into()),
            ("pt_uistyle".into(), "40".into()),
            ("aid".into(), "716027609".into()),
            ("daid".into(), "383".into()),
            ("pt_3rd_aid".into(), OAUTH_CLIENT_ID.into()),
            ("has_onekey".into(), "1".into()),
        ],
        headers: attempt_headers(&cookies, PTLOGIN_REFERER),
        retry: RetryClass::AuthPoll,
        redirects: RedirectMode::None,
        max_response_bytes: Some(MAX_DESKTOP_QR_TEXT_BYTES),
        cancellation: cancellation.clone(),
        ..HttpOptions::default()
    };
    let response = client
        .context
        .request_http_raw(HttpMethod::Get, PTQR_LOGIN_URL, &options)
        .await?;
    if cancellation.is_cancelled() {
        return Err(QmError::cancelled());
    }
    if !(200..300).contains(&response.status) {
        return Err(qr_protocol("ptqrlogin returned a non-success status"));
    }
    require_response_endpoint(&response, "ssl.ptlogin2.qq.com", "/ptqrlogin")?;
    if response.body.len() > MAX_DESKTOP_QR_TEXT_BYTES {
        return Err(malformed("oversized QR poll response"));
    }
    cookies.absorb(&response.headers)?;
    let body = std::str::from_utf8(&response.body).map_err(|_| malformed("non-UTF8 QR poll"))?;
    let arguments = parse_ptui_arguments(body)?;
    let status = arguments.first().map(String::as_str).unwrap_or_default();
    match status {
        "66" => Ok(DesktopQrPoll::WaitingForScan),
        "67" => Ok(DesktopQrPoll::WaitingForConfirmation),
        "65" => Ok(DesktopQrPoll::Expired),
        "68" => Ok(DesktopQrPoll::Rejected),
        "0" => {
            let callback_url = arguments
                .get(2)
                .ok_or_else(|| malformed("missing check_sig callback"))?;
            complete_desktop_sign_in(client, callback_url, &mut cookies, now_ms, cancellation).await
        }
        _ => Err(qr_protocol("unrecognized QR login status")),
    }
}

async fn complete_desktop_sign_in(
    client: &Client,
    callback_url: &str,
    cookies: &mut Cookies,
    now_ms: u64,
    cancellation: CancellationToken,
) -> Result<DesktopQrPoll> {
    let callback = url::Url::parse(callback_url).map_err(|_| malformed("invalid check_sig URL"))?;
    require_endpoint(&callback, CHECK_SIG_HOST, CHECK_SIG_PATH)?;
    let options = HttpOptions {
        headers: attempt_headers(cookies, PTLOGIN_REFERER),
        retry: RetryClass::AuthPoll,
        redirects: RedirectMode::None,
        max_response_bytes: Some(MAX_DESKTOP_QR_TEXT_BYTES),
        cancellation: cancellation.clone(),
        ..HttpOptions::default()
    };
    let check_sig = client
        .context
        .request_http_raw(HttpMethod::Get, callback_url, &options)
        .await?;
    require_response_endpoint(&check_sig, CHECK_SIG_HOST, CHECK_SIG_PATH)?;
    let check_sig_location = redirect_location(&check_sig, "check_sig did not redirect")?;
    let check_sig_location = url::Url::parse(&check_sig.final_url)
        .and_then(|base| base.join(check_sig_location))
        .map_err(|_| malformed("invalid check_sig redirect"))?;
    require_endpoint(&check_sig_location, "graph.qq.com", "/oauth2.0/login_jump")?;
    cookies.absorb(&check_sig.headers)?;
    let p_skey = cookies
        .0
        .get("p_skey")
        .filter(|value| !value.is_empty())
        .cloned()
        .ok_or_else(|| malformed("missing p_skey"))?;
    let gtk = hash33(&p_skey, GTK_SEED) as u32;

    let form = authorize_form(gtk, now_ms);
    let mut headers = attempt_headers(cookies, OAUTH_AUTHORIZE_REFERER);
    headers.push((
        "Content-Type".into(),
        "application/x-www-form-urlencoded".into(),
    ));
    let options = HttpOptions {
        headers,
        body: Some(form),
        retry: RetryClass::AuthPoll,
        redirects: RedirectMode::None,
        max_response_bytes: Some(MAX_DESKTOP_QR_TEXT_BYTES),
        cancellation: cancellation.clone(),
        ..HttpOptions::default()
    };
    let authorize = client
        .context
        .request_http_raw(HttpMethod::Post, OAUTH_AUTHORIZE_URL, &options)
        .await?;
    require_response_endpoint(&authorize, "graph.qq.com", "/oauth2.0/authorize")?;
    let location = redirect_location(&authorize, "authorize did not redirect")?;
    cookies.absorb(&authorize.headers)?;
    let location = url::Url::parse(&authorize.final_url)
        .and_then(|base| base.join(location))
        .map_err(|_| malformed("invalid authorize redirect"))?;
    require_endpoint(&location, OAUTH_CODE_HOST, OAUTH_CODE_PATH)?;
    let codes = location
        .query_pairs()
        .filter_map(|(key, value)| (key == "code").then(|| value.into_owned()))
        .collect::<Vec<_>>();
    let [code] = codes.as_slice() else {
        return Err(malformed("missing or duplicate authorization code"));
    };
    if code.is_empty() {
        return Err(malformed("missing or duplicate authorization code"));
    }
    let cookie_header = cookies.header();
    let session = exchange_oauth_code(
        client,
        OAuthExchange {
            provider: OAuthLoginProvider::Qq,
            code,
            gtk: Some(gtk),
            cookie_header: &cookie_header,
            now_ms,
        },
        cancellation,
    )
    .await?;
    Ok(DesktopQrPoll::Confirmed(Box::new(session)))
}

/// Authorize body in wire order; `Url` encoding matches the ptlogin client.
fn authorize_form(gtk: u32, now_ms: u64) -> Vec<u8> {
    let fields: [(&str, String); 12] = [
        ("response_type", "code".into()),
        ("client_id", OAUTH_CLIENT_ID.into()),
        ("redirect_uri", OAUTH_REDIRECT_URI.into()),
        ("scope", "get_user_info,get_app_friends".into()),
        ("state", "state".into()),
        ("switch", String::new()),
        ("from_ptlogin", "1".into()),
        ("src", "1".into()),
        ("update_auth", "1".into()),
        ("openapi", "1010_1030".into()),
        ("g_tk", gtk.to_string()),
        ("auth_time", now_ms.to_string()),
    ];
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in &fields {
        serializer.append_pair(key, value);
    }
    serializer.append_pair("ui", &format!("{:032x}", rand::random::<u128>()));
    serializer.finish().into_bytes()
}

fn attempt_headers(cookies: &Cookies, referer: &str) -> Vec<(String, String)> {
    vec![
        ("Referer".into(), referer.to_owned()),
        ("Cookie".into(), cookies.header()),
    ]
}

fn require_endpoint(url: &url::Url, host: &str, path: &str) -> Result<()> {
    if url.scheme() == "https"
        && url.port_or_known_default() == Some(443)
        && url.host_str() == Some(host)
        && url.path() == path
        && url.username().is_empty()
        && url.password().is_none()
    {
        Ok(())
    } else {
        Err(qr_protocol("unexpected QR redirect endpoint"))
    }
}

fn require_response_endpoint(response: &TransportResponse, host: &str, path: &str) -> Result<()> {
    let final_url = url::Url::parse(&response.final_url)
        .map_err(|_| malformed("invalid final QR response URL"))?;
    require_endpoint(&final_url, host, path)
}

fn unique_header<'a>(response: &'a TransportResponse, name: &str) -> Result<Option<&'a str>> {
    let values = response
        .headers
        .iter()
        .filter_map(|(key, value)| key.eq_ignore_ascii_case(name).then_some(value.as_str()))
        .collect::<Vec<_>>();
    match values.as_slice() {
        [] => Ok(None),
        [value] => Ok(Some(value)),
        _ => Err(malformed("ambiguous QR response header")),
    }
}

fn redirect_location<'a>(
    response: &'a TransportResponse,
    message: &'static str,
) -> Result<&'a str> {
    if !matches!(response.status, 301 | 302 | 303 | 307 | 308) {
        return Err(qr_protocol(message));
    }
    unique_header(response, "location")?.ok_or_else(|| qr_protocol(message))
}

fn normalize_image_mime(value: &str) -> Option<&'static str> {
    match value
        .split(';')
        .next()?
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "image/png" => Some("image/png"),
        "image/jpeg" => Some("image/jpeg"),
        _ => None,
    }
}

/// Parse the `ptuiCB('..','..','..','..','..');` callback body.
fn parse_ptui_arguments(body: &str) -> Result<Vec<String>> {
    let inner = body
        .trim()
        .strip_prefix("ptuiCB(")
        .and_then(|value| value.strip_suffix(");"))
        .ok_or_else(|| malformed("invalid ptuiCB callback envelope"))?;
    let mut arguments = Vec::new();
    let mut chars = inner.chars().peekable();
    loop {
        while chars
            .next_if(|character| character.is_ascii_whitespace())
            .is_some()
        {}
        if chars.peek().is_none() {
            break;
        }
        if chars.next() != Some('\'') {
            return Err(malformed("invalid ptuiCB argument"));
        }
        let mut current = String::new();
        let mut closed = false;
        while let Some(character) = chars.next() {
            match character {
                '\\' => current.push(
                    chars
                        .next()
                        .ok_or_else(|| malformed("malformed ptuiCB escape"))?,
                ),
                '\'' => {
                    closed = true;
                    break;
                }
                character => current.push(character),
            }
        }
        if !closed {
            return Err(malformed("unterminated ptuiCB argument"));
        }
        arguments.push(current);
        while chars
            .next_if(|character| character.is_ascii_whitespace())
            .is_some()
        {}
        match chars.peek() {
            None => break,
            Some(',') => {
                chars.next();
                if chars
                    .clone()
                    .all(|character| character.is_ascii_whitespace())
                {
                    return Err(malformed("trailing ptuiCB separator"));
                }
            }
            _ => return Err(malformed("missing ptuiCB separator")),
        }
    }
    (!arguments.is_empty())
        .then_some(arguments)
        .ok_or_else(|| malformed("empty ptuiCB callback"))
}

fn qr_protocol(message: &'static str) -> QmError {
    QmError::Protocol {
        stage: "desktop-qr",
        message: message.into(),
    }
}
