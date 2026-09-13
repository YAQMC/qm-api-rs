use std::sync::{Arc, Mutex};

use qqmusic_api::{
    account::{read_page, AccountRead},
    ApiTransport, CancellationToken, Client, Credential, HttpBody, HttpMethod, NetworkError,
    NetworkErrorKind, Platform, QmError, RetryClass, TransportRequest, TransportResponse,
};
use serde_json::{json, Value};

struct RecordingTransport {
    requests: Mutex<Vec<Value>>,
    cookies: Mutex<Vec<String>>,
    response: Value,
    status: u16,
    timeout: bool,
    cancel_after_send: bool,
}

impl RecordingTransport {
    fn new(data: Value) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            cookies: Mutex::new(Vec::new()),
            response: json!({"code":0,"req":{"code":0,"data":data}}),
            status: 200,
            timeout: false,
            cancel_after_send: false,
        }
    }
    fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

#[async_trait::async_trait]
impl ApiTransport for RecordingTransport {
    async fn execute(&self, request: TransportRequest) -> qqmusic_api::Result<TransportResponse> {
        assert_eq!(request.method, HttpMethod::Post);
        assert_eq!(request.url, "https://u.y.qq.com/cgi-bin/musicu.fcg");
        assert_eq!(request.retry, RetryClass::SafeRead);
        let HttpBody::Json(body) = request.body else {
            panic!("JSON body");
        };
        self.requests.lock().unwrap().push(body);
        self.cookies.lock().unwrap().push(
            request
                .headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("cookie"))
                .unwrap()
                .1
                .clone(),
        );
        if self.cancel_after_send {
            request.cancellation.cancel();
        }
        if self.timeout {
            return Err(QmError::Network(NetworkError {
                kind: NetworkErrorKind::Timeout,
                message: "synthetic timeout".into(),
            }));
        }
        Ok(TransportResponse {
            status: self.status,
            final_url: request.url,
            headers: Vec::new(),
            body: serde_json::to_vec(&self.response).unwrap(),
        })
    }
}

fn credential(id: i64) -> Credential {
    Credential {
        musicid: id,
        musickey: format!("SYNTHETIC_KEY_{id}"),
        encrypt_uin: format!("SYNTHETIC_EUIN_{id}"),
        ..Credential::default()
    }
}

fn client(transport: Arc<RecordingTransport>) -> Client {
    Client::new_with_transport(Some(credential(99)), Some(Platform::Android), transport)
}

#[tokio::test]
async fn owned_and_saved_lists_use_precise_ranges_and_explicit_credentials() {
    for (operation, data, expected) in [
        (
            AccountRead::OwnedPlaylists,
            json!({"sin":2,"total":4,"v_playlist":[{},{}]}),
            json!({"module":"music.musicasset.PlaylistBaseRead","method":"GetPlaylistByUin",
                "param":{"uin":"10001","sin":2,"ein":3}}),
        ),
        (
            AccountRead::CollectedPlaylists,
            json!({"offset":2,"total":4,"v_list":[{},{}]}),
            json!({"module":"music.musicasset.PlaylistFavRead","method":"CgiGetPlaylistFavInfo",
                "param":{"uin":"SYNTHETIC_EUIN_10001","offset":2,"size":2}}),
        ),
    ] {
        let transport = Arc::new(RecordingTransport::new(data));
        let page = read_page(
            &client(transport.clone()),
            &credential(10001),
            operation,
            2,
            2,
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(page.row_count, 2);
        assert_eq!(page.next_offset, None);
        assert_eq!(page.total, Some(4));
        assert_eq!(transport.count(), 1);
        let requests = transport.requests.lock().unwrap();
        assert_eq!(requests[0]["req"], expected);
        assert_eq!(requests[0]["comm"]["uin"], "10001");
        assert_eq!(requests[0]["comm"]["ct"], 24);
        assert!(!requests[0].to_string().contains("SYNTHETIC_KEY"));
        let cookies = transport.cookies.lock().unwrap();
        assert!(cookies[0].contains("SYNTHETIC_KEY_10001"));
        assert!(!cookies[0].contains("SYNTHETIC_KEY_99"));
    }
}

#[tokio::test]
async fn same_client_concurrent_accounts_do_not_share_identity() {
    let transport = Arc::new(RecordingTransport::new(json!({"v_list":[],"total":0})));
    let client = client(transport.clone());
    let a = credential(10001);
    let b = credential(10002);
    let (a, b) = tokio::join!(
        read_page(
            &client,
            &a,
            AccountRead::CollectedPlaylists,
            0,
            100,
            CancellationToken::new()
        ),
        read_page(
            &client,
            &b,
            AccountRead::CollectedPlaylists,
            0,
            100,
            CancellationToken::new()
        ),
    );
    a.unwrap();
    b.unwrap();
    let mut ids = transport
        .requests
        .lock()
        .unwrap()
        .iter()
        .map(|body| {
            (
                body["comm"]["uin"].as_str().unwrap().to_owned(),
                body["req"]["param"]["uin"].as_str().unwrap().to_owned(),
            )
        })
        .collect::<Vec<_>>();
    ids.sort();
    assert_eq!(
        ids,
        vec![
            ("10001".into(), "SYNTHETIC_EUIN_10001".into()),
            ("10002".into(), "SYNTHETIC_EUIN_10002".into())
        ]
    );
}

#[tokio::test]
async fn continuation_uses_raw_rows_and_accepts_a_real_finish_flag() {
    for (data, next) in [
        (json!({"v_playlist":[{"tid":1},null],"total":4}), Some(2)),
        (json!({"v_playlist":[{}],"bFinish":true}), None),
        (json!({"playlist":[{}],"bFinish":1}), None),
        (json!({"v_playlist":[{}],"bFinish":false}), Some(1)),
        (json!({"v_playlist":[],"bFinish":true}), None),
    ] {
        let transport = Arc::new(RecordingTransport::new(data));
        let page = read_page(
            &client(transport),
            &credential(1),
            AccountRead::OwnedPlaylists,
            0,
            2,
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(page.next_offset, next);
    }
}

#[tokio::test]
async fn malformed_or_non_progressing_pages_fail_closed() {
    for data in [
        json!({"v_playlist":[],"hasmore":true}),
        json!({"v_playlist":[],"total":1}),
        json!({"v_playlist":[{}],"sin":1}),
        json!({"v_playlist":[{}],"offset":"0"}),
        json!({"v_playlist":[{}, {}, {}]}),
        json!({"v_playlist":{},"total":0}),
        json!({"songlist":[],"total":0}),
        json!({"v_playlist":[{}],"total":0}),
        json!({"v_playlist":[{}],"hasmore":true,"bFinish":true}),
        json!({"v_playlist":[{}],"has_more":false,"hasmore":true}),
        json!({"v_playlist":[{}],"bFinish":"true"}),
        json!({"v_playlist":[{}],"total":-1}),
        json!({"v_playlist":[{}],"total":2,"bFinish":true}),
    ] {
        let transport = Arc::new(RecordingTransport::new(data));
        assert!(matches!(
            read_page(
                &client(transport),
                &credential(1),
                AccountRead::OwnedPlaylists,
                0,
                2,
                CancellationToken::new()
            )
            .await,
            Err(QmError::ApiData(_))
        ));
    }
}

#[tokio::test]
async fn business_errors_and_wrong_list_kind_are_not_empty_success() {
    for code in [104401, 2001] {
        let transport = Arc::new(RecordingTransport {
            response: json!({"code":0,"req":{"code":code,"data":{"v_playlist":[]}}}),
            ..RecordingTransport::new(Value::Null)
        });
        let error = read_page(
            &client(transport),
            &credential(1),
            AccountRead::OwnedPlaylists,
            0,
            100,
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
        assert!(matches!(
            (code, error),
            (104401, QmError::CredentialExpired(_)) | (2001, QmError::RateLimited)
        ));
    }
    for (operation, data) in [
        (AccountRead::OwnedPlaylists, json!({"v_list":[],"total":0})),
        (
            AccountRead::CollectedPlaylists,
            json!({"songlist":[],"total":0}),
        ),
    ] {
        let transport = Arc::new(RecordingTransport::new(data));
        assert!(matches!(
            read_page(
                &client(transport),
                &credential(1),
                operation,
                0,
                100,
                CancellationToken::new()
            )
            .await,
            Err(QmError::ApiData(_))
        ));
    }
}

#[tokio::test]
async fn invalid_bounds_identity_and_cancellation_send_nothing() {
    let transport = Arc::new(RecordingTransport::new(json!({"v_list":[],"total":0})));
    let client = client(transport.clone());
    for (offset, limit) in [(0, 0), (0, 101), (u64::MAX, 1)] {
        assert!(matches!(
            read_page(
                &client,
                &credential(1),
                AccountRead::OwnedPlaylists,
                offset,
                limit,
                CancellationToken::new()
            )
            .await,
            Err(QmError::ValueError(_))
        ));
    }
    let mut no_euin = credential(1);
    no_euin.encrypt_uin.clear();
    assert!(matches!(
        read_page(
            &client,
            &no_euin,
            AccountRead::CollectedPlaylists,
            0,
            100,
            CancellationToken::new()
        )
        .await,
        Err(QmError::CredentialInvalid(_))
    ));
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    assert!(read_page(
        &client,
        &credential(1),
        AccountRead::OwnedPlaylists,
        0,
        100,
        cancellation
    )
    .await
    .is_err());
    assert_eq!(transport.count(), 0);
}

#[tokio::test]
async fn transport_errors_and_post_response_cancellation_are_not_empty_success() {
    for (status, flag) in [(401, "auth"), (429, "rate"), (503, "offline")] {
        let transport = Arc::new(RecordingTransport {
            status,
            ..RecordingTransport::new(json!({"v_list":[],"total":0}))
        });
        let result = read_page(
            &client(transport.clone()),
            &credential(1),
            AccountRead::CollectedPlaylists,
            0,
            100,
            CancellationToken::new(),
        )
        .await;
        assert!(
            matches!(result, Err(QmError::Http { status: code, .. }) if code == status),
            "{flag}"
        );
        assert_eq!(transport.count(), 1);
    }
    for cancel in [false, true] {
        let transport = Arc::new(RecordingTransport {
            timeout: !cancel,
            cancel_after_send: cancel,
            ..RecordingTransport::new(json!({"v_list":[],"total":0}))
        });
        let result = read_page(
            &client(transport.clone()),
            &credential(1),
            AccountRead::CollectedPlaylists,
            0,
            100,
            CancellationToken::new(),
        )
        .await;
        assert!(
            matches!(result, Err(QmError::Network(error)) if error.kind ==
            if cancel { NetworkErrorKind::Cancelled } else { NetworkErrorKind::Timeout })
        );
        assert_eq!(transport.count(), 1);
    }
}
