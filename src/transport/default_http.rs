//! 默认 `ApiTransport` 实现. reqwest 类型不得泄漏到 crate 公开 API.

use std::sync::Mutex;
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use url::Url;

use super::{
    form_pairs, is_redirect_status, parse_origin_input, redirects_as_get, same_origin,
    validate_url, ApiTransport, HttpBody, HttpMethod, RedirectMode, RetryClass, TransportConfig,
    TransportRequest, TransportResponse,
};
use crate::error::{NetworkErrorKind, Result};
use crate::QmError;

pub struct ReqwestApiTransport {
    client: reqwest::Client,
    config: TransportConfig,
    extra_origins: Mutex<Vec<String>>,
}

impl std::fmt::Debug for ReqwestApiTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReqwestApiTransport")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl ReqwestApiTransport {
    pub fn new(config: TransportConfig) -> Result<Self> {
        let mut builder = reqwest::Client::builder()
            .gzip(true)
            .brotli(true)
            .cookie_store(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(config.connect_timeout)
            .timeout(config.total_timeout);
        if let Some(proxy) = &config.proxy {
            let proxy = reqwest::Proxy::all(proxy).map_err(QmError::map_transport_error)?;
            builder = builder.proxy(proxy);
        }
        let client = builder.build().map_err(QmError::map_transport_error)?;
        Ok(Self {
            client,
            config,
            extra_origins: Mutex::new(Vec::new()),
        })
    }

    fn extras(&self) -> Vec<String> {
        self.extra_origins
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn validate(&self, url: &Url) -> Result<()> {
        validate_url(url, &self.extras())
    }

    async fn execute_attempt(
        &self,
        request: &TransportRequest,
        start_url: &Url,
    ) -> Result<TransportResponse> {
        let mut current_url = start_url.clone();
        let mut method = request.method;
        let mut headers = request.headers.clone();
        let mut body = request.body.clone();
        let mut hops = 0_usize;

        loop {
            self.validate(&current_url)?;
            let response = self
                .send(
                    method,
                    current_url.clone(),
                    &headers,
                    &body,
                    request.timeout,
                    &request.cancellation,
                )
                .await?;
            let status = response.status().as_u16();

            if request.redirects == RedirectMode::FollowValidated && is_redirect_status(status) {
                if let Some(location) = header_value(response.headers(), "location") {
                    if hops >= self.config.max_redirects {
                        return Err(QmError::network_kind(
                            NetworkErrorKind::Redirect,
                            "too many redirects",
                        ));
                    }
                    let next = current_url
                        .join(&location)
                        .map_err(|e| QmError::ValueError(format!("invalid redirect: {e}")))?;
                    self.validate(&next)?;
                    if !same_origin(&current_url, &next) {
                        strip_secret_headers(&mut headers);
                    }
                    if redirects_as_get(status, method) {
                        method = HttpMethod::Get;
                        body = HttpBody::Empty;
                        strip_entity_headers(&mut headers);
                    }
                    strip_hop_headers(&mut headers);
                    current_url = next;
                    hops += 1;
                    continue;
                }
            }

            let response_headers = headers_from_reqwest(response.headers());
            let body_bytes = self.collect_body(response, &request.cancellation).await?;
            return Ok(TransportResponse {
                status,
                final_url: current_url.to_string(),
                headers: response_headers,
                body: body_bytes,
            });
        }
    }

    async fn send(
        &self,
        method: HttpMethod,
        url: Url,
        headers: &[(String, String)],
        body: &HttpBody,
        timeout: Option<Duration>,
        cancellation: &tokio_util::sync::CancellationToken,
    ) -> Result<reqwest::Response> {
        if cancellation.is_cancelled() {
            return Err(QmError::cancelled());
        }
        let mut builder = self
            .client
            .request(to_reqwest_method(method), url)
            .headers(to_header_map(headers));
        builder = apply_body(builder, body);
        builder = builder.timeout(timeout.unwrap_or(self.config.total_timeout));

        tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(QmError::cancelled()),
            response = builder.send() => response.map_err(QmError::map_transport_error),
        }
    }

    async fn collect_body(
        &self,
        response: reqwest::Response,
        cancellation: &tokio_util::sync::CancellationToken,
    ) -> Result<Vec<u8>> {
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(QmError::cancelled()),
            body = response.bytes() => body
                .map(|b| b.to_vec())
                .map_err(QmError::map_transport_error),
        }
    }
}

#[async_trait::async_trait]
impl ApiTransport for ReqwestApiTransport {
    async fn execute(&self, request: TransportRequest) -> Result<TransportResponse> {
        let mut url = Url::parse(&request.url)
            .map_err(|e| QmError::ValueError(format!("invalid url: {e}")))?;
        if !request.query.is_empty() {
            let mut pairs = url.query_pairs_mut();
            for (k, v) in &request.query {
                pairs.append_pair(k, v);
            }
        }
        self.validate(&url)?;

        let max_extra = match request.retry {
            RetryClass::SafeRead => self.config.retry_max,
            RetryClass::AuthPoll | RetryClass::Write => 0,
        };
        let mut extra_used = 0_u32;
        loop {
            match self.execute_attempt(&request, &url).await {
                Ok(resp)
                    if extra_used < max_extra
                        && request.retry == RetryClass::SafeRead
                        && is_retryable_status(resp.status) =>
                {
                    extra_used += 1;
                    sleep_or_cancel(&request, self.config.retry_delay).await?;
                }
                Ok(resp) => return Ok(resp),
                Err(e)
                    if extra_used < max_extra
                        && request.retry == RetryClass::SafeRead
                        && e.is_retryable() =>
                {
                    extra_used += 1;
                    sleep_or_cancel(&request, self.config.retry_delay).await?;
                }
                Err(e) => return Err(e),
            }
        }
    }

    fn allow_origin(&self, origin: &str) {
        let Some(origin) = parse_origin_input(origin) else {
            return;
        };
        let mut extras = self.extra_origins.lock().unwrap_or_else(|e| e.into_inner());
        if !extras.contains(&origin) {
            extras.push(origin);
        }
    }
}

async fn sleep_or_cancel(request: &TransportRequest, delay: Duration) -> Result<()> {
    tokio::select! {
        biased;
        _ = request.cancellation.cancelled() => Err(QmError::cancelled()),
        _ = tokio::time::sleep(delay) => Ok(()),
    }
}

fn is_retryable_status(status: u16) -> bool {
    status == 429 || status >= 500
}

fn to_reqwest_method(method: HttpMethod) -> reqwest::Method {
    match method {
        HttpMethod::Get => reqwest::Method::GET,
        HttpMethod::Post => reqwest::Method::POST,
        HttpMethod::Put => reqwest::Method::PUT,
        HttpMethod::Head => reqwest::Method::HEAD,
        HttpMethod::Delete => reqwest::Method::DELETE,
        HttpMethod::Patch => reqwest::Method::PATCH,
    }
}

fn to_header_map(headers: &[(String, String)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (k, v) in headers {
        let Ok(name) = HeaderName::from_bytes(k.as_bytes()) else {
            continue;
        };
        let Ok(value) = HeaderValue::from_str(v) else {
            continue;
        };
        map.append(name, value);
    }
    map
}

fn headers_from_reqwest(map: &HeaderMap) -> Vec<(String, String)> {
    map.iter()
        .map(|(k, v)| {
            (
                k.as_str().to_string(),
                String::from_utf8_lossy(v.as_bytes()).into_owned(),
            )
        })
        .collect()
}

fn header_value(map: &HeaderMap, name: &str) -> Option<String> {
    map.get(name)
        .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())
}

fn apply_body(builder: reqwest::RequestBuilder, body: &HttpBody) -> reqwest::RequestBuilder {
    match body {
        HttpBody::Empty => builder,
        HttpBody::Json(v) => builder.json(v),
        HttpBody::Form(v) => builder.form(&form_pairs(v)),
        HttpBody::Bytes(b) => builder.body(b.clone()),
    }
}

fn strip_hop_headers(headers: &mut Vec<(String, String)>) {
    headers.retain(|(k, _)| {
        !k.eq_ignore_ascii_case("host")
            && !k.eq_ignore_ascii_case("content-length")
            && !k.eq_ignore_ascii_case("transfer-encoding")
    });
}

fn strip_entity_headers(headers: &mut Vec<(String, String)>) {
    headers.retain(|(k, _)| {
        !k.eq_ignore_ascii_case("content-type")
            && !k.eq_ignore_ascii_case("content-length")
            && !k.eq_ignore_ascii_case("content-encoding")
    });
}

fn strip_secret_headers(headers: &mut Vec<(String, String)>) {
    headers.retain(|(k, _)| {
        !k.eq_ignore_ascii_case("cookie")
            && !k.eq_ignore_ascii_case("authorization")
            && !k.eq_ignore_ascii_case("x-cos-security-token")
    });
}
