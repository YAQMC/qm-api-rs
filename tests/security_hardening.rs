use async_trait::async_trait;
use qqmusic_api::{
    ApiTransport, Client, Credential, HttpMethod, HttpOptions, Result, TransportRequest,
    TransportResponse,
};
use std::sync::{Arc, Mutex};

#[derive(Debug, Default)]
struct CaptureTransport {
    requests: Mutex<Vec<TransportRequestSnapshot>>,
}

#[derive(Debug, Clone)]
struct TransportRequestSnapshot {
    headers: Vec<(String, String)>,
}

#[async_trait]
impl ApiTransport for CaptureTransport {
    async fn execute(&self, request: TransportRequest) -> Result<TransportResponse> {
        self.requests
            .lock()
            .unwrap()
            .push(TransportRequestSnapshot {
                headers: request.headers.clone(),
            });
        Ok(TransportResponse {
            status: 200,
            final_url: request.url,
            headers: Vec::new(),
            body: b"{}".to_vec(),
        })
    }
}

fn cookie_header(headers: &[(String, String)]) -> Option<&str> {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("cookie"))
        .map(|(_, value)| value.as_str())
}

#[tokio::test]
async fn generic_http_none_credential_is_anonymous() {
    let transport = Arc::new(CaptureTransport::default());
    let client = Client::new_with_transport(
        Some(Credential {
            musicid: 10001,
            str_musicid: "10001".into(),
            musickey: "global-secret".into(),
            ..Default::default()
        }),
        None,
        transport.clone(),
    );

    client
        .request_http(
            HttpMethod::Get,
            "https://u.y.qq.com/cgi-bin/ping",
            &HttpOptions::default(),
        )
        .await
        .unwrap();

    let requests = transport.requests.lock().unwrap();
    let cookie = cookie_header(&requests[0].headers);
    assert!(
        cookie.is_none(),
        "global credential leaked into generic HTTP: {cookie:?}"
    );
}

#[tokio::test]
async fn generic_http_explicit_credential_is_sent() {
    let transport = Arc::new(CaptureTransport::default());
    let client = Client::new_with_transport(None, None, transport.clone());
    let explicit = Credential {
        musicid: 20002,
        str_musicid: "20002".into(),
        musickey: "explicit-secret".into(),
        ..Default::default()
    };
    let opts = HttpOptions {
        credential: Some(explicit),
        ..Default::default()
    };

    client
        .request_http(HttpMethod::Get, "https://u.y.qq.com/cgi-bin/ping", &opts)
        .await
        .unwrap();

    let requests = transport.requests.lock().unwrap();
    let cookie = cookie_header(&requests[0].headers).expect("explicit credential cookie");
    assert!(cookie.contains("uin=20002"));
    assert!(cookie.contains("qm_keyst=explicit-secret"));
}

#[tokio::test]
async fn download_none_credential_is_anonymous() {
    let transport = Arc::new(CaptureTransport::default());
    let client = Client::new_with_transport(
        Some(Credential {
            musicid: 30003,
            str_musicid: "30003".into(),
            musickey: "download-global-secret".into(),
            ..Default::default()
        }),
        None,
        transport.clone(),
    );

    client
        .download("https://isure.stream.qqmusic.qq.com/test.m4a", None)
        .await
        .unwrap();

    let requests = transport.requests.lock().unwrap();
    assert!(cookie_header(&requests[0].headers).is_none());
}

#[test]
fn http_options_debug_redacts_values() {
    let opts = HttpOptions {
        headers: vec![
            ("Authorization".into(), "Bearer super-secret-token".into()),
            ("X-Api-Key".into(), "api-secret-value".into()),
        ],
        cookies: vec![("session".into(), "cookie-secret-value".into())],
        credential: Some(Credential {
            musicid: 7,
            str_musicid: "7".into(),
            musickey: "credential-secret-value".into(),
            ..Default::default()
        }),
        ..Default::default()
    };

    let debug = format!("{opts:?}");
    assert!(debug.contains("Authorization"));
    assert!(debug.contains("X-Api-Key"));
    assert!(debug.contains("session"));
    assert!(!debug.contains("super-secret-token"));
    assert!(!debug.contains("api-secret-value"));
    assert!(!debug.contains("cookie-secret-value"));
    assert!(!debug.contains("credential-secret-value"));
}
