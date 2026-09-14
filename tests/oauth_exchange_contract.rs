use qqmusic_api::{
    auth::{exchange_oauth_code, OAuthExchange, MAX_OAUTH_RESPONSE_BYTES},
    ApiTransport, CancellationToken, Client, Credential, HttpBody, HttpMethod, NetworkError,
    NetworkErrorKind, OAuthLoginProvider, QmError, RetryClass, TransportRequest, TransportResponse,
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

const URL: &str = "https://u.y.qq.com/cgi-bin/musicu.fcg";
const NOW: u64 = 1_700_000_000_000;

#[tokio::test]
async fn concurrent_exchanges_share_no_attempt_cookies_or_credential_results() {
    struct Echo;
    #[async_trait::async_trait]
    impl ApiTransport for Echo {
        async fn execute(
            &self,
            request: TransportRequest,
        ) -> qqmusic_api::Result<TransportResponse> {
            let HttpBody::Json(payload) = request.body else {
                panic!("JSON")
            };
            let code = payload["req"]["param"]["code"].as_str().unwrap();
            let cookie = &request
                .headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("cookie"))
                .unwrap()
                .1;
            let id = if code == "SYNTHETIC_A" { 10001 } else { 10002 };
            assert_eq!(cookie, &format!("p_skey=SYNTHETIC_{id}"));
            tokio::task::yield_now().await;
            Ok(TransportResponse {status:200,final_url:URL.into(),headers:vec![],body:serde_json::to_vec(&json!({
                "code":0,"req":{"code":0,"data":{"musicid":id,"musickey":format!("SYNTHETIC_KEY_{id}")}}
            })).unwrap()})
        }
    }
    let client = Client::new_with_transport(
        Some(Credential {
            musicid: 99,
            musickey: "SYNTHETIC_AMBIENT".into(),
            ..Default::default()
        }),
        None,
        Arc::new(Echo),
    );
    let mut a = request(OAuthLoginProvider::Qq);
    a.code = "SYNTHETIC_A";
    a.cookie_header = "p_skey=SYNTHETIC_10001";
    let mut b = request(OAuthLoginProvider::Wechat);
    b.code = "SYNTHETIC_B";
    b.cookie_header = "p_skey=SYNTHETIC_10002";
    let (a, b) = tokio::join!(
        exchange_oauth_code(&client, a, CancellationToken::new()),
        exchange_oauth_code(&client, b, CancellationToken::new())
    );
    let a = a.unwrap();
    let b = b.unwrap();
    assert_eq!(a.credential.musicid, 10001);
    assert_eq!(b.credential.musicid, 10002);
    assert_eq!(a.credential.login_type, 2);
    assert_eq!(b.credential.login_type, 1);
    assert!(!a.cookie_header.contains("10002"));
    assert!(!b.cookie_header.contains("10001"));
}
struct Mock {
    calls: Mutex<Vec<(Value, String)>>,
    response: TransportResponse,
    timeout: bool,
    cancel: bool,
}
impl Mock {
    fn new(data: Value) -> Self {
        Self {
            calls: Mutex::new(vec![]),
            response: TransportResponse {
                status: 200,
                final_url: URL.into(),
                headers: vec![],
                body: serde_json::to_vec(&json!({"code":0,"req":{"code":0,"data":data}})).unwrap(),
            },
            timeout: false,
            cancel: false,
        }
    }
}
#[async_trait::async_trait]
impl ApiTransport for Mock {
    async fn execute(&self, request: TransportRequest) -> qqmusic_api::Result<TransportResponse> {
        assert_eq!(request.method, HttpMethod::Post);
        assert_eq!(request.url, URL);
        assert_eq!(request.retry, RetryClass::AuthPoll);
        assert_eq!(request.max_response_bytes, Some(MAX_OAUTH_RESPONSE_BYTES));
        let cookie = request
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("cookie"))
            .expect("explicit Cookie")
            .1
            .clone();
        assert!(request
            .headers
            .iter()
            .any(|(k, v)| k.eq_ignore_ascii_case("origin") && v == "https://y.qq.com"));
        let HttpBody::Json(payload) = request.body else {
            panic!("JSON body")
        };
        assert!(!cookie.contains("SYNTHETIC_AMBIENT"));
        self.calls.lock().unwrap().push((payload, cookie));
        if self.cancel {
            request.cancellation.cancel();
        }
        if self.timeout {
            return Err(QmError::Network(NetworkError {
                kind: NetworkErrorKind::Timeout,
                message: "synthetic timeout".into(),
            }));
        }
        Ok(self.response.clone())
    }
}
fn client(transport: Arc<Mock>) -> Client {
    Client::new_with_transport(
        Some(Credential {
            musicid: 99,
            musickey: "SYNTHETIC_AMBIENT".into(),
            ..Default::default()
        }),
        None,
        transport,
    )
}
fn request(provider: OAuthLoginProvider) -> OAuthExchange<'static> {
    OAuthExchange {
        provider,
        code: "SYNTHETIC_CODE",
        gtk: None,
        cookie_header: "",
        now_ms: NOW,
    }
}
fn data() -> Value {
    json!({"str_musicid":"10001","musickey":"SYNTHETIC_MUSIC_KEY","musickeyCreateTime":1_700_000_000,"keyExpiresIn":3600})
}

#[tokio::test]
async fn qq_and_wechat_use_exact_wire_and_never_inherit_default_identity() {
    for (provider, module, method, login_type, param) in [
        (
            OAuthLoginProvider::Qq,
            "QQConnectLogin.LoginServer",
            "QQLogin",
            2,
            json!({"code":"SYNTHETIC_CODE"}),
        ),
        (
            OAuthLoginProvider::Wechat,
            "music.login.LoginServer",
            "Login",
            1,
            json!({"code":"SYNTHETIC_CODE","strAppid":"wx48db31d50e334801"}),
        ),
    ] {
        let transport = Arc::new(Mock::new(data()));
        let response = exchange_oauth_code(
            &client(transport.clone()),
            request(provider),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(response.credential.musicid, 10001);
        assert_eq!(response.credential.login_type, login_type);
        assert_eq!(response.expires_at_ms, NOW + 3_600_000);
        assert!(response
            .cookie_header
            .contains(&format!("tmeLoginType={login_type}")));
        let calls = transport.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].0,
            json!({"comm":{"platform":"yqq","ct":24,"cv":0,"tmeLoginType":login_type},
            "req":{"module":module,"method":method,"param":param}})
        );
        assert_eq!(calls[0].1, "");
    }
}

#[tokio::test]
async fn cookies_follow_the_attempt_but_new_credentials_replace_stale_identity() {
    let mut transport = Mock::new(
        json!({"uin":"10001","musicKey":"SYNTHETIC_MUSIC_KEY","encryptUin":"SYNTHETIC_NEW_EUIN"}),
    );
    transport.response.headers = vec![
        (
            "Set-Cookie".into(),
            "p_skey=SYNTHETIC_NEW_P_SKEY; Secure".into(),
        ),
        ("set-cookie".into(), "obsolete=; Max-Age=0".into()),
    ];
    let transport = Arc::new(transport);
    let mut exchange = request(OAuthLoginProvider::Qq);
    exchange.gtk = Some(42);
    exchange.cookie_header="p_skey=SYNTHETIC_OLD; qrsig=SYNTHETIC_QR; pt_login_sig=SYNTHETIC_LOGIN; obsolete=yes; euin=OLD_ACCOUNT; tmeLoginType=1";
    let result = exchange_oauth_code(
        &client(transport.clone()),
        exchange,
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(result.credential.encrypt_uin, "SYNTHETIC_NEW_EUIN");
    assert_eq!(result.expires_at_ms, NOW + 86_400_000);
    assert!(result.cookie_header.contains("p_skey=SYNTHETIC_NEW_P_SKEY"));
    assert!(result.cookie_header.contains("euin=SYNTHETIC_NEW_EUIN"));
    for removed in [
        "qrsig=",
        "pt_login_sig=",
        "obsolete=",
        "OLD_ACCOUNT",
        "tmeLoginType=1",
    ] {
        assert!(!result.cookie_header.contains(removed));
    }
    assert_eq!(transport.calls.lock().unwrap()[0].0["comm"]["g_tk"], 42);
}

#[tokio::test]
async fn invalid_inputs_and_precancellation_do_not_send() {
    let transport = Arc::new(Mock::new(data()));
    let client = client(transport.clone());
    for code in ["".to_owned(), "a".repeat(2049), "code\nforged".into()] {
        let mut r = request(OAuthLoginProvider::Qq);
        r.code = &code;
        assert!(exchange_oauth_code(&client, r, CancellationToken::new())
            .await
            .is_err());
    }
    for cookie in [
        "qrsig=x\r\nHeader:y".to_owned(),
        "broken".into(),
        "qrsig=a;qrsig=b".into(),
        "a=x;evil name=y".into(),
        format!("qrsig={}", "a".repeat(17 * 1024)),
    ] {
        let mut r = request(OAuthLoginProvider::Qq);
        r.cookie_header = &cookie;
        assert!(exchange_oauth_code(&client, r, CancellationToken::new())
            .await
            .is_err());
    }
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(
        matches!(exchange_oauth_code(&client,request(OAuthLoginProvider::Qq),cancel).await,
        Err(QmError::Network(e)) if e.kind==NetworkErrorKind::Cancelled)
    );
    assert!(transport.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn response_failures_do_not_publish_a_session_or_replay() {
    let mut fixtures = Vec::new();
    for payload in [
        json!({}),
        json!({"musicid":0,"musickey":"key"}),
        json!({"musicid":1}),
        json!({"musicid":1,"str_musicid":"2","musickey":"key"}),
        json!({"uin":"not-a-number","musickey":"key"}),
        json!({"musicid":1,"musickey":"key;Injected=1"}),
    ] {
        fixtures.push(Mock::new(payload));
    }
    let mut bad = Mock::new(data());
    bad.response.body = b"SYNTHETIC_SECRET_BAD_JSON".to_vec();
    fixtures.push(bad);
    let mut business = Mock::new(data());
    business.response.body = serde_json::to_vec(
        &json!({"code":0,"req":{"code":12345,"data":{"opaque":"SYNTHETIC_SECRET"}}}),
    )
    .unwrap();
    fixtures.push(business);
    let mut redirect = Mock::new(data());
    redirect.response.final_url = "https://evil.example/login".into();
    fixtures.push(redirect);
    let mut http = Mock::new(data());
    http.response.status = 503;
    fixtures.push(http);
    let mut cookie = Mock::new(data());
    cookie.response.headers = vec![("Set-Cookie".into(), "bad=x\r\nInjected=y".into())];
    fixtures.push(cookie);
    let mut oversized = Mock::new(data());
    oversized.response.body = vec![b' '; MAX_OAUTH_RESPONSE_BYTES + 1];
    fixtures.push(oversized);
    for fixture in fixtures {
        let transport = Arc::new(fixture);
        let err = exchange_oauth_code(
            &client(transport.clone()),
            request(OAuthLoginProvider::Qq),
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
        assert!(!format!("{err:?}").contains("SYNTHETIC_SECRET"));
        assert_eq!(transport.calls.lock().unwrap().len(), 1);
    }
    for cancel in [false, true] {
        let transport = Arc::new(Mock {
            cancel,
            timeout: !cancel,
            ..Mock::new(data())
        });
        let err = exchange_oauth_code(
            &client(transport.clone()),
            request(OAuthLoginProvider::Qq),
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err,QmError::Network(e) if e.kind==if cancel {NetworkErrorKind::Cancelled} else {NetworkErrorKind::Timeout})
        );
        assert_eq!(transport.calls.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn expiry_aliases_and_secret_safe_debug_are_preserved() {
    for (created, lifetime, expected) in [
        (NOW as i64, 3600, NOW + 3_600_000),
        (0, 0, NOW + 86_400_000),
        (-1, 3600, NOW + 86_400_000),
    ] {
        let transport = Arc::new(Mock::new(
            json!({"musicid":10001,"musickey":"SYNTHETIC_MUSIC_KEY",
            "musickey_create_time":created,"key_expires_in":lifetime,"euin":"SYNTHETIC_EUIN"}),
        ));
        let exchange = request(OAuthLoginProvider::Wechat);
        assert!(!format!("{exchange:?}").contains("SYNTHETIC_CODE"));
        let response = exchange_oauth_code(&client(transport), exchange, CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(response.expires_at_ms, expected);
        assert_eq!(response.credential.encrypt_uin, "SYNTHETIC_EUIN");
        assert!(!format!("{response:?}").contains("SYNTHETIC"));
    }
    let wire = qqmusic_api::build_oauth_code_exchange_request(
        OAuthLoginProvider::Qq,
        "SYNTHETIC_SECRET_CODE",
        Some(12345),
    );
    assert!(!format!("{wire:?}").contains("SYNTHETIC_SECRET_CODE"));
    assert!(!format!("{wire:?}").contains("12345"));
}
