use qqmusic_api::{
    auth::{
        create_desktop_qr, mobile_qr_launch_url, oauth_callback_contract,
        oauth_callback_url_prefix, poll_desktop_qr, DesktopQrPoll, DESKTOP_QR_LIFETIME_MS,
        MAX_DESKTOP_QR_IMAGE_BYTES, MAX_MOBILE_QR_ID_BYTES,
    },
    ApiTransport, CancellationToken, Client, Credential, HttpMethod, OAuthLoginProvider, QmError,
    RedirectMode, RetryClass, TransportRequest, TransportResponse,
};
use serde_json::json;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

const PTQR_SHOW: &str = "https://ssl.ptlogin2.qq.com/ptqrshow";
const PTQR_LOGIN: &str = "https://ssl.ptlogin2.qq.com/ptqrlogin";
const AUTHORIZE: &str = "https://graph.qq.com/oauth2.0/authorize";
const EXCHANGE: &str = "https://u.y.qq.com/cgi-bin/musicu.fcg";
const CALLBACK: &str =
    "https://ssl.ptlogin2.graph.qq.com/check_sig?uin=1000000001&ptsigx=SYNTHETIC_SIG&s_url=x";
const CODE_REDIRECT: &str = "https://y.qq.com/portal/wx_redirect.html?login_type=1&surl=https://y.qq.com/&state=state&code=SYNTHETIC_CODE";
const NOW: u64 = 1_700_000_000_000;

struct Step {
    method: HttpMethod,
    url: &'static str,
    response: TransportResponse,
}

#[derive(Clone)]
struct Seen {
    url: String,
    query: Vec<(String, String)>,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    retry: RetryClass,
    redirects: RedirectMode,
    max_response_bytes: Option<usize>,
}

struct Script {
    steps: Mutex<VecDeque<Step>>,
    seen: Mutex<Vec<Seen>>,
}

impl Script {
    fn new(steps: Vec<Step>) -> Arc<Self> {
        Arc::new(Self {
            steps: Mutex::new(steps.into()),
            seen: Mutex::new(Vec::new()),
        })
    }
    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl ApiTransport for Script {
    async fn execute(&self, request: TransportRequest) -> qqmusic_api::Result<TransportResponse> {
        let step = self
            .steps
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected request");
        assert_eq!(request.method, step.method, "method for {}", step.url);
        assert_eq!(request.url, step.url, "url");
        let body = match request.body {
            qqmusic_api::HttpBody::Empty => Vec::new(),
            qqmusic_api::HttpBody::Bytes(bytes) => bytes,
            qqmusic_api::HttpBody::Json(value) => serde_json::to_vec(&value).unwrap(),
            other => panic!("unexpected body {other:?}"),
        };
        self.seen.lock().unwrap().push(Seen {
            url: request.url.clone(),
            query: request.query.clone(),
            headers: request.headers.clone(),
            body,
            retry: request.retry,
            redirects: request.redirects,
            max_response_bytes: request.max_response_bytes,
        });
        Ok(step.response)
    }
}

fn response(
    status: u16,
    url: &str,
    headers: Vec<(&str, &str)>,
    body: Vec<u8>,
) -> TransportResponse {
    TransportResponse {
        status,
        final_url: url.to_string(),
        headers: headers
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        body,
    }
}

fn qr_show() -> Step {
    Step {
        method: HttpMethod::Get,
        url: PTQR_SHOW,
        response: response(
            200,
            PTQR_SHOW,
            vec![
                ("content-type", "image/png"),
                ("set-cookie", "qrsig=SYNTHETIC_QRSIG; Path=/"),
            ],
            b"\x89PNG\r\n\x1a\nSYNTHETIC".to_vec(),
        ),
    }
}

fn qr_poll(status: &str) -> Step {
    let body = match status {
        "0" => format!("ptuiCB('0','0','{CALLBACK}','0','登录成功', ' ');\r\n"),
        other => format!("ptuiCB('{other}','0','','0','二维码状态', ' ');\r\n"),
    };
    Step {
        method: HttpMethod::Get,
        url: PTQR_LOGIN,
        response: response(
            200,
            PTQR_LOGIN,
            vec![("set-cookie", "pt_login_sig=SYNTHETIC_LOGIN_SIG; Path=/")],
            body.into_bytes(),
        ),
    }
}

fn check_sig(location: &str) -> Step {
    Step {
        method: HttpMethod::Get,
        url: CALLBACK,
        response: response(
            302,
            CALLBACK,
            vec![
                ("location", location),
                ("set-cookie", "p_skey=SYNTHETIC_P_SKEY; Path=/"),
            ],
            Vec::new(),
        ),
    }
}

fn authorize(location: &str) -> Step {
    Step {
        method: HttpMethod::Post,
        url: AUTHORIZE,
        response: response(302, AUTHORIZE, vec![("location", location)], Vec::new()),
    }
}

fn exchange() -> Step {
    Step {
        method: HttpMethod::Post,
        url: EXCHANGE,
        response: response(
            200,
            EXCHANGE,
            Vec::new(),
            serde_json::to_vec(&json!({
                "code": 0,
                "req": {"code": 0, "data": {"musicid": 1000000001, "musickey": "SYNTHETIC_KEY"}}
            }))
            .unwrap(),
        ),
    }
}

fn client_with(transport: Arc<Script>) -> Client {
    Client::new_with_transport(
        Some(Credential {
            musicid: 999,
            musickey: "SYNTHETIC_AMBIENT".into(),
            ..Default::default()
        }),
        None,
        transport,
    )
}

fn cookie_header(seen: &Seen) -> String {
    seen.headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("cookie"))
        .map(|(_, v)| v.clone())
        .collect::<Vec<_>>()
        .join("; ")
}

fn header(seen: &Seen, name: &str) -> Option<String> {
    seen.headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.clone())
}

fn query(seen: &Seen, key: &str) -> Option<String> {
    seen.query
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
}

#[tokio::test]
async fn desktop_qr_reaches_a_session_without_sending_ambient_credentials() {
    let transport = Script::new(vec![
        qr_show(),
        qr_poll("66"),
        qr_poll("67"),
        qr_poll("0"),
        check_sig("https://graph.qq.com/oauth2.0/login_jump"),
        authorize(CODE_REDIRECT),
        exchange(),
    ]);
    let client = client_with(Arc::clone(&transport));

    let challenge = create_desktop_qr(&client, NOW, CancellationToken::new())
        .await
        .expect("challenge");
    assert_eq!(challenge.mime_type, "image/png");
    assert_eq!(challenge.qrsig, "SYNTHETIC_QRSIG");
    assert_eq!(challenge.expires_at_ms, NOW + DESKTOP_QR_LIFETIME_MS);

    assert!(matches!(
        poll_desktop_qr(&client, &challenge.qrsig, NOW, CancellationToken::new())
            .await
            .unwrap(),
        DesktopQrPoll::WaitingForScan
    ));
    assert!(matches!(
        poll_desktop_qr(&client, &challenge.qrsig, NOW, CancellationToken::new())
            .await
            .unwrap(),
        DesktopQrPoll::WaitingForConfirmation
    ));
    let DesktopQrPoll::Confirmed(session) =
        poll_desktop_qr(&client, &challenge.qrsig, NOW, CancellationToken::new())
            .await
            .unwrap()
    else {
        panic!("expected a confirmed session")
    };
    assert_eq!(session.credential.musicid, 1000000001);
    assert_eq!(session.credential.login_type, 2);
    assert_eq!(session.credential.musickey, "SYNTHETIC_KEY");
    assert!(session.cookie_header.contains("qm_keyst=SYNTHETIC_KEY"));
    assert!(!session.cookie_header.contains("qrsig"));
    assert!(!session.cookie_header.contains("pt_login_sig"));

    let seen = transport.seen();
    assert_eq!(seen.len(), 7);
    assert!(seen
        .iter()
        .all(|s| !cookie_header(s).contains("SYNTHETIC_AMBIENT")));
    assert!(seen.iter().all(|s| !cookie_header(s).contains("999")));
    assert_eq!(seen[0].retry, RetryClass::SafeRead);
    assert_eq!(seen[0].redirects, RedirectMode::FollowValidated);
    assert_eq!(seen[0].max_response_bytes, Some(MAX_DESKTOP_QR_IMAGE_BYTES));
    assert_eq!(header(&seen[0], "cookie").as_deref(), Some(""));
    assert_eq!(
        header(&seen[0], "referer").unwrap(),
        "https://xui.ptlogin2.qq.com/"
    );
    assert_eq!(
        query(&seen[0], "u1").unwrap(),
        "https://graph.qq.com/oauth2.0/login_jump"
    );
    assert_eq!(query(&seen[0], "t").unwrap(), format!("0.{NOW}"));
    assert_eq!(seen[1].retry, RetryClass::AuthPoll);
    assert_eq!(seen[1].redirects, RedirectMode::None);
    assert_eq!(seen[1].max_response_bytes, Some(16 * 1024));
    assert_eq!(cookie_header(&seen[1]), "qrsig=SYNTHETIC_QRSIG");
    // Reference golden value: hash33("SYNTHETIC_QRSIG", 0). The 5381-seeded
    // variant used by `g_tk` would be 452482981.
    assert_eq!(query(&seen[1], "ptqrtoken").unwrap(), "30614848");
    assert_eq!(seen[3].url, PTQR_LOGIN);
    assert_eq!(cookie_header(&seen[3]), "qrsig=SYNTHETIC_QRSIG");
    assert_eq!(seen[4].url, CALLBACK);
    assert_eq!(
        cookie_header(&seen[4]),
        "pt_login_sig=SYNTHETIC_LOGIN_SIG; qrsig=SYNTHETIC_QRSIG"
    );
    assert_eq!(
        cookie_header(&seen[5]),
        "p_skey=SYNTHETIC_P_SKEY; pt_login_sig=SYNTHETIC_LOGIN_SIG; qrsig=SYNTHETIC_QRSIG"
    );
    let form = String::from_utf8(seen[5].body.clone()).unwrap();
    assert!(form.starts_with("response_type=code&client_id=100497308&"));
    // Reference golden value: hash33("SYNTHETIC_P_SKEY", 5381).
    assert!(form.contains("&g_tk=2023320266&"));
    let exchange = String::from_utf8(seen[6].body.clone()).unwrap();
    assert!(exchange.contains("SYNTHETIC_CODE"));
    assert_eq!(
        header(&seen[5], "content-type").unwrap(),
        "application/x-www-form-urlencoded"
    );
    assert_eq!(seen[6].max_response_bytes, Some(256 * 1024));
}

#[tokio::test]
async fn desktop_qr_maps_terminal_statuses_without_a_session() {
    for (status, expired) in [("65", true), ("68", false)] {
        let transport = Script::new(vec![qr_show(), qr_poll(status)]);
        let client = client_with(Arc::clone(&transport));
        let challenge = create_desktop_qr(&client, NOW, CancellationToken::new())
            .await
            .unwrap();
        let poll = poll_desktop_qr(&client, &challenge.qrsig, NOW, CancellationToken::new())
            .await
            .unwrap();
        match (expired, poll) {
            (true, DesktopQrPoll::Expired) | (false, DesktopQrPoll::Rejected) => {}
            other => panic!("unexpected poll result {other:?}"),
        }
        assert_eq!(transport.seen().len(), 2);
    }
}

#[tokio::test]
async fn desktop_qr_rejects_unrecognized_status_and_hostile_redirects() {
    let transport = Script::new(vec![qr_show(), qr_poll("99")]);
    let client = client_with(Arc::clone(&transport));
    let challenge = create_desktop_qr(&client, NOW, CancellationToken::new())
        .await
        .unwrap();
    let error = poll_desktop_qr(&client, &challenge.qrsig, NOW, CancellationToken::new())
        .await
        .expect_err("unknown status");
    assert!(matches!(
        error,
        QmError::Protocol {
            stage: "desktop-qr",
            ..
        }
    ));

    let transport = Script::new(vec![
        qr_show(),
        Step {
            method: HttpMethod::Get,
            url: PTQR_LOGIN,
            response: response(
                200,
                PTQR_LOGIN,
                Vec::new(),
                b"ptuiCB('0','0','https://evil.example/check_sig?uin=1','0','ok', ' ');".to_vec(),
            ),
        },
    ]);
    let client = client_with(Arc::clone(&transport));
    let challenge = create_desktop_qr(&client, NOW, CancellationToken::new())
        .await
        .unwrap();
    assert!(
        poll_desktop_qr(&client, &challenge.qrsig, NOW, CancellationToken::new())
            .await
            .is_err()
    );
    assert_eq!(
        transport.seen().len(),
        2,
        "hostile check_sig must not be fetched"
    );

    let transport = Script::new(vec![
        qr_show(),
        qr_poll("0"),
        check_sig("https://graph.qq.com/oauth2.0/login_jump"),
        authorize("https://evil.example/portal/wx_redirect.html?code=SYNTHETIC_CODE"),
    ]);
    let client = client_with(Arc::clone(&transport));
    let challenge = create_desktop_qr(&client, NOW, CancellationToken::new())
        .await
        .unwrap();
    assert!(
        poll_desktop_qr(&client, &challenge.qrsig, NOW, CancellationToken::new())
            .await
            .is_err()
    );
    assert_eq!(
        transport.seen().len(),
        4,
        "no exchange after a hostile redirect"
    );
}

#[tokio::test]
async fn desktop_qr_rejects_ambiguous_headers_redirects_and_callback_grammar() {
    let malformed_callbacks = [
        "ptuiCB('0''0''https://ssl.ptlogin2.graph.qq.com/check_sig');",
        "prefix ptuiCB('66','0','','0','ok');",
        "ptuiCB('66','0','','0','ok'); trailing",
        "ptuiCB('66',);",
        "ptuiCB(66,'0','','0','ok');",
    ];
    for body in malformed_callbacks {
        let transport = Script::new(vec![Step {
            method: HttpMethod::Get,
            url: PTQR_LOGIN,
            response: response(200, PTQR_LOGIN, Vec::new(), body.as_bytes().to_vec()),
        }]);
        assert!(poll_desktop_qr(
            &client_with(transport),
            "SYNTHETIC_QRSIG",
            NOW,
            CancellationToken::new(),
        )
        .await
        .is_err());
    }

    let transport = Script::new(vec![Step {
        method: HttpMethod::Get,
        url: PTQR_SHOW,
        response: response(
            200,
            PTQR_SHOW,
            vec![
                ("content-type", "image/png"),
                ("Content-Type", "image/jpeg"),
                ("set-cookie", "qrsig=SYNTHETIC_QRSIG"),
            ],
            b"image".to_vec(),
        ),
    }]);
    assert!(
        create_desktop_qr(&client_with(transport), NOW, CancellationToken::new())
            .await
            .is_err()
    );

    for location in [
        "https://evil.example/oauth2.0/login_jump",
        "https://graph.qq.com.evil.example/oauth2.0/login_jump",
        "https://graph.qq.com/oauth2.0/other",
    ] {
        let transport = Script::new(vec![qr_poll("0"), check_sig(location)]);
        assert!(poll_desktop_qr(
            &client_with(transport.clone()),
            "SYNTHETIC_QRSIG",
            NOW,
            CancellationToken::new(),
        )
        .await
        .is_err());
        assert_eq!(
            transport.seen().len(),
            2,
            "hostile check_sig redirect stops authorization"
        );
    }

    let transport = Script::new(vec![
        qr_poll("0"),
        check_sig("https://graph.qq.com/oauth2.0/login_jump"),
        authorize("https://y.qq.com/portal/wx_redirect.html?login_type=1&surl=https://y.qq.com/&state=state&code=first&code=second"),
    ]);
    assert!(poll_desktop_qr(
        &client_with(transport.clone()),
        "SYNTHETIC_QRSIG",
        NOW,
        CancellationToken::new(),
    )
    .await
    .is_err());
    assert_eq!(
        transport.seen().len(),
        3,
        "ambiguous code must not be exchanged"
    );
}

#[tokio::test]
async fn desktop_qr_binds_the_oauth_redirect_contract_before_exchange() {
    for location in [
        "https://y.qq.com/portal/wx_redirect.html?login_type=2&surl=https://y.qq.com/&state=state&code=SYNTHETIC_CODE",
        "https://y.qq.com/portal/wx_redirect.html?login_type=1&surl=https://evil.example/&state=state&code=SYNTHETIC_CODE",
        "https://y.qq.com/portal/wx_redirect.html?login_type=1&surl=https://y.qq.com/&state=wrong&code=SYNTHETIC_CODE",
        "https://y.qq.com/portal/wx_redirect.html?login_type=1&surl=https://y.qq.com/&state=state&error=denied&code=SYNTHETIC_CODE",
        "https://y.qq.com/portal/wx_redirect.html?login_type=1&login_type=1&surl=https://y.qq.com/&state=state&code=SYNTHETIC_CODE",
    ] {
        let transport = Script::new(vec![
            qr_poll("0"),
            check_sig("https://graph.qq.com/oauth2.0/login_jump"),
            authorize(location),
        ]);
        assert!(poll_desktop_qr(
            &client_with(transport.clone()),
            "SYNTHETIC_QRSIG",
            NOW,
            CancellationToken::new(),
        )
        .await
        .is_err());
        assert_eq!(transport.seen().len(), 3, "bad callback must not reach exchange");
    }
}

#[tokio::test]
async fn desktop_qr_rejects_mismatched_final_urls_and_duplicate_locations() {
    let transport = Script::new(vec![Step {
        method: HttpMethod::Get,
        url: PTQR_SHOW,
        response: response(
            200,
            "https://ssl.ptlogin2.qq.com/unexpected",
            vec![
                ("content-type", "image/png"),
                ("set-cookie", "qrsig=SYNTHETIC_QRSIG"),
            ],
            b"image".to_vec(),
        ),
    }]);
    assert!(
        create_desktop_qr(&client_with(transport), NOW, CancellationToken::new())
            .await
            .is_err()
    );

    let oversized_qrsig = format!("qrsig={}", "a".repeat(513));
    let transport = Script::new(vec![Step {
        method: HttpMethod::Get,
        url: PTQR_SHOW,
        response: response(
            200,
            PTQR_SHOW,
            vec![
                ("content-type", "image/png"),
                ("set-cookie", &oversized_qrsig),
            ],
            b"image".to_vec(),
        ),
    }]);
    assert!(
        create_desktop_qr(&client_with(transport), NOW, CancellationToken::new())
            .await
            .is_err()
    );

    let transport = Script::new(vec![Step {
        method: HttpMethod::Get,
        url: PTQR_LOGIN,
        response: response(
            200,
            "https://ssl.ptlogin2.qq.com/unexpected",
            Vec::new(),
            b"ptuiCB('66','0','','0','waiting');".to_vec(),
        ),
    }]);
    assert!(poll_desktop_qr(
        &client_with(transport),
        "SYNTHETIC_QRSIG",
        NOW,
        CancellationToken::new(),
    )
    .await
    .is_err());

    let mut duplicate = check_sig("https://graph.qq.com/oauth2.0/login_jump");
    duplicate
        .response
        .headers
        .push(("Location".into(), "https://evil.example/login_jump".into()));
    let transport = Script::new(vec![qr_poll("0"), duplicate]);
    assert!(poll_desktop_qr(
        &client_with(transport),
        "SYNTHETIC_QRSIG",
        NOW,
        CancellationToken::new(),
    )
    .await
    .is_err());
}

#[tokio::test]
async fn desktop_qr_validates_the_image_and_honours_cancellation() {
    let transport = Script::new(vec![Step {
        method: HttpMethod::Get,
        url: PTQR_SHOW,
        response: response(
            200,
            PTQR_SHOW,
            vec![
                ("content-type", "text/html"),
                ("set-cookie", "qrsig=SYNTHETIC"),
            ],
            b"<html/>".to_vec(),
        ),
    }]);
    let client = client_with(Arc::clone(&transport));
    assert!(create_desktop_qr(&client, NOW, CancellationToken::new())
        .await
        .is_err());

    let transport = Script::new(vec![Step {
        method: HttpMethod::Get,
        url: PTQR_SHOW,
        response: response(
            200,
            PTQR_SHOW,
            vec![("content-type", "image/png")],
            vec![0u8; MAX_DESKTOP_QR_IMAGE_BYTES + 1],
        ),
    }]);
    let client = client_with(Arc::clone(&transport));
    assert!(create_desktop_qr(&client, NOW, CancellationToken::new())
        .await
        .is_err());

    let transport = Script::new(vec![qr_show()]);
    let client = client_with(Arc::clone(&transport));
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(create_desktop_qr(&client, NOW, cancelled.clone())
        .await
        .is_err());
    assert!(poll_desktop_qr(&client, "SYNTHETIC_QRSIG", NOW, cancelled)
        .await
        .is_err());
    assert!(poll_desktop_qr(&client, "", NOW, CancellationToken::new())
        .await
        .is_err());
    assert!(
        transport.seen().is_empty(),
        "cancelled calls must not hit the network"
    );
}

#[test]
fn mobile_qr_launch_url_is_library_owned_and_encoded() {
    assert_eq!(
        mobile_qr_launch_url("SYNTHETIC_ID").unwrap(),
        "https://y.qq.com/m/client/qr_code_login/authorize.html?qrcode_id=SYNTHETIC_ID"
    );
    assert_eq!(
        mobile_qr_launch_url("a b&c=d").unwrap(),
        "https://y.qq.com/m/client/qr_code_login/authorize.html?qrcode_id=a+b%26c%3Dd"
    );
}

#[test]
fn mobile_qr_launch_url_rejects_untrusted_identifiers() {
    assert!(mobile_qr_launch_url("").is_err());
    assert!(mobile_qr_launch_url("bad\nid").is_err());
    let oversized = "x".repeat(MAX_MOBILE_QR_ID_BYTES + 1);
    assert!(mobile_qr_launch_url(&oversized).is_err());
    let boundary = "x".repeat(MAX_MOBILE_QR_ID_BYTES);
    assert!(mobile_qr_launch_url(&boundary).is_ok());
}

#[test]
fn oauth_callback_contract_matches_the_wire_redirect() {
    let prefix = oauth_callback_url_prefix(OAuthLoginProvider::Qq);
    assert_eq!(prefix, "https://y.qq.com/portal/wx_redirect.html");
    assert!(CODE_REDIRECT.starts_with(prefix));

    let qq = oauth_callback_contract(OAuthLoginProvider::Qq);
    assert_eq!(qq.host, "y.qq.com");
    assert_eq!(qq.path, "/portal/wx_redirect.html");
    assert_eq!(qq.login_type, "1");
    assert_eq!(qq.surl, "https://y.qq.com/");
    assert!(CODE_REDIRECT.contains(&format!("login_type={}", qq.login_type)));
    assert!(CODE_REDIRECT.contains(&format!("surl={}", qq.surl)));

    let wechat = oauth_callback_contract(OAuthLoginProvider::Wechat);
    assert_eq!(wechat.login_type, "2");
    assert_eq!(wechat.host, qq.host);
    assert_eq!(wechat.path, qq.path);
    assert_eq!(
        oauth_callback_url_prefix(OAuthLoginProvider::Wechat),
        prefix
    );
}
