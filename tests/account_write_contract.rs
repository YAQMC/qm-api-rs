use async_trait::async_trait;
use qqmusic_api::account::{write, AccountWrite};
use qqmusic_api::{
    ApiTransport, CancellationToken, Client, Credential, HttpBody, HttpMethod, NetworkError,
    NetworkErrorKind, Platform, QmError, Result, RetryClass, TransportRequest, TransportResponse,
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Default)]
enum Outcome {
    #[default]
    Accepted,
    Timeout,
    ServerError,
    BusinessError,
    CancelBeforeReply,
}

#[derive(Default)]
struct Transport {
    calls: Mutex<Vec<Value>>,
    outcome: Outcome,
}

#[async_trait]
impl ApiTransport for Transport {
    async fn execute(&self, request: TransportRequest) -> Result<TransportResponse> {
        assert_eq!(request.method, HttpMethod::Post);
        assert_eq!(request.url, "https://u.y.qq.com/cgi-bin/musicu.fcg");
        assert_eq!(request.retry, RetryClass::Write);
        let HttpBody::Json(body) = request.body else {
            panic!("expected JSON account write")
        };
        let uin = body["comm"]["uin"].as_str().unwrap();
        let token = format!("synthetic-{uin}");
        assert_eq!(body["comm"]["authst"], token);
        assert_eq!(body["comm"]["ct"], "11");
        assert_eq!(body["comm"]["g_tk"], qqmusic_api::hash33(&token, 5381));
        for key in ["uid", "qq", "loginUin"] {
            assert_eq!(body["comm"][key], uin);
        }
        if body["req_0"]["module"] == "music.musicasset.PlaylistFavWrite" {
            assert_eq!(
                body["req_0"]["param"]["uin"],
                format!("synthetic-euin-{uin}")
            );
        }
        let cookie = request
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("cookie"))
            .map(|(_, value)| value.as_str())
            .unwrap();
        assert!(cookie.contains(&format!("qm_keyst={token}")));
        assert!(!cookie.contains("synthetic-999"));
        self.calls.lock().unwrap().push(body);
        // Keep both requests in flight in the concurrent-identity test.
        tokio::task::yield_now().await;
        if matches!(self.outcome, Outcome::Timeout) {
            return Err(QmError::Network(NetworkError {
                kind: NetworkErrorKind::Timeout,
                message: "synthetic timeout".into(),
            }));
        }
        if matches!(self.outcome, Outcome::CancelBeforeReply) {
            request.cancellation.cancel();
        }
        let code = if matches!(self.outcome, Outcome::BusinessError) {
            104401
        } else {
            0
        };
        Ok(TransportResponse {
            status: if matches!(self.outcome, Outcome::ServerError) {
                503
            } else {
                200
            },
            final_url: request.url,
            headers: vec![],
            body: serde_json::to_vec(&json!({
                "code": 0, "req_0": {"code": code, "data": {"accepted": true}}
            }))
            .unwrap(),
        })
    }
}

fn credential(id: i64) -> Credential {
    Credential {
        musicid: id,
        str_musicid: id.to_string(),
        musickey: format!("synthetic-{id}"),
        encrypt_uin: format!("synthetic-euin-{id}"),
        ..Default::default()
    }
}

fn client(outcome: Outcome) -> (Client, Arc<Transport>) {
    let transport = Arc::new(Transport {
        outcome,
        ..Default::default()
    });
    let client = Client::new_with_transport(
        Some(credential(999)),
        Some(Platform::Android),
        transport.clone(),
    );
    (client, transport)
}

fn favorite() -> AccountWrite {
    AccountWrite::FavoriteSong {
        add: true,
        song_id: 42,
        song_type: 0,
    }
}

#[tokio::test]
async fn all_typed_mutations_use_fixed_wire_contracts() {
    let mut cases = vec![
        (
            AccountWrite::CreatePlaylist {
                name: "歌曲".into(),
            },
            "PlaylistBaseWrite",
            "AddPlaylist",
            json!({"dirName": "歌曲"}),
        ),
        (
            AccountWrite::DeletePlaylist { dir_id: 9 },
            "PlaylistBaseWrite",
            "DelPlaylist",
            json!({"dirId": 9}),
        ),
        (
            AccountWrite::EditPlaylist {
                dir_id: 9,
                mask: 1,
                name: "Title".into(),
                description: "Desc".into(),
                picture_url: String::new(),
                tag_list: String::new(),
            },
            "PlaylistBaseWrite",
            "EditPlaylist",
            json!({
               "dirId":9, "mask":1, "dirNewName":"Title", "dirNewDesc":"Desc",
                "dirNewPicUrl":"", "dirNewtaglist":""
            }),
        ),
    ];
    for add in [true, false] {
        let method = if add { "AddSonglist" } else { "DelSonglist" };
        cases.push((
            AccountWrite::FavoriteSong {
                add,
                song_id: 42,
                song_type: 0,
            },
            "PlaylistDetailWrite",
            method,
            json!({"dirId":201,"tid":0,"bFmtUtf8":true,"v_songInfo":[{"songId":42,"songType":0}]}),
        ));
        cases.push((
            AccountWrite::PlaylistTracks { add, dir_id: 9, tid: 100, songs: vec![(42,0),(43,1)] },
            "PlaylistDetailWrite", method,
            json!({"dirId":9,"tid":100,"bFmtUtf8":true,"v_songInfo":[{"songId":42,"songType":0},{"songId":43,"songType":1}]}),
        ));
        cases.push((
            AccountWrite::CollectPlaylist {
                collect: add,
                playlist_id: 100,
            },
            "PlaylistFavWrite",
            if add {
                "FavPlaylist"
            } else {
                "CancelFavPlaylist"
            },
            json!({"uin":"synthetic-euin-101", "v_playlistId":[100]}),
        ));
    }
    for (operation, module, method, param) in cases {
        let (api, transport) = client(Outcome::Accepted);
        let reply = write(&api, &credential(101), operation, CancellationToken::new())
            .await
            .unwrap();
        assert!(reply.succeeded());
        let calls = transport.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0]["req_0"],
            json!({
                "module":format!("music.musicasset.{module}"), "method":method, "param":param
            })
        );
    }
}

#[tokio::test]
async fn concurrent_writes_use_explicit_credentials_without_changing_client() {
    let (api, transport) = client(Outcome::Accepted);
    let first = credential(101);
    let second = credential(202);
    let (a, b) = tokio::join!(
        write(
            &api,
            &first,
            AccountWrite::CollectPlaylist {
                collect: true,
                playlist_id: 100
            },
            CancellationToken::new()
        ),
        write(
            &api,
            &second,
            AccountWrite::CollectPlaylist {
                collect: false,
                playlist_id: 100
            },
            CancellationToken::new()
        ),
    );
    assert!(a.unwrap().succeeded());
    assert!(b.unwrap().succeeded());
    assert_eq!(api.credential().musicid, 999);
    let calls = transport.calls.lock().unwrap();
    let mut identities = calls
        .iter()
        .map(|body| body["comm"]["uin"].as_str().unwrap())
        .collect::<Vec<_>>();
    identities.sort_unstable();
    assert_eq!(identities, ["101", "202"]);
}

#[tokio::test]
async fn invalid_or_cancelled_writes_do_not_reach_transport() {
    let (api, transport) = client(Outcome::Accepted);
    let token = CancellationToken::new();
    token.cancel();
    assert!(matches!(
        write(&api, &credential(101), favorite(), token).await,
        Err(QmError::Network(NetworkError {
            kind: NetworkErrorKind::Cancelled,
            ..
        }))
    ));
    assert!(matches!(
        write(
            &api,
            &Credential::default(),
            favorite(),
            CancellationToken::new()
        )
        .await,
        Err(QmError::CredentialInvalid(_))
    ));
    assert!(matches!(
        write(
            &api,
            &credential(101),
            AccountWrite::DeletePlaylist { dir_id: 0 },
            CancellationToken::new()
        )
        .await,
        Err(QmError::ValueError(_))
    ));
    assert!(transport.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn collection_requires_the_explicit_encrypted_identity() {
    let (api, transport) = client(Outcome::Accepted);
    let mut explicit = credential(101);
    explicit.encrypt_uin.clear();
    let result = write(
        &api,
        &explicit,
        AccountWrite::CollectPlaylist {
            collect: true,
            playlist_id: 100,
        },
        CancellationToken::new(),
    )
    .await;
    assert!(matches!(result, Err(QmError::CredentialInvalid(_))));
    assert!(transport.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn write_failure_is_never_replayed_and_business_code_is_preserved() {
    for outcome in [
        Outcome::Timeout,
        Outcome::ServerError,
        Outcome::BusinessError,
    ] {
        let (api, transport) = client(outcome);
        let result = write(&api, &credential(101), favorite(), CancellationToken::new()).await;
        match outcome {
            Outcome::Timeout => assert!(matches!(
                result,
                Err(QmError::Network(NetworkError {
                    kind: NetworkErrorKind::Timeout,
                    ..
                }))
            )),
            Outcome::ServerError => {
                assert!(matches!(result, Err(QmError::Http { status: 503, .. })))
            }
            Outcome::BusinessError => assert_eq!(result.unwrap().code, 104401),
            _ => unreachable!(),
        }
        assert_eq!(transport.calls.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn cancelled_response_is_not_published_as_success() {
    let (api, transport) = client(Outcome::CancelBeforeReply);
    let result = write(&api, &credential(101), favorite(), CancellationToken::new()).await;
    assert!(matches!(
        result,
        Err(QmError::Network(NetworkError {
            kind: NetworkErrorKind::Cancelled,
            ..
        }))
    ));
    assert_eq!(transport.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn cancellation_reaches_an_in_flight_write_without_replay() {
    struct PendingTransport {
        started: tokio::sync::Notify,
        calls: std::sync::atomic::AtomicUsize,
    }
    #[async_trait]
    impl ApiTransport for PendingTransport {
        async fn execute(&self, request: TransportRequest) -> Result<TransportResponse> {
            assert_eq!(request.retry, RetryClass::Write);
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.started.notify_one();
            request.cancellation.cancelled().await;
            Err(QmError::Network(NetworkError {
                kind: NetworkErrorKind::Cancelled,
                message: "synthetic cancellation".into(),
            }))
        }
    }
    let transport = Arc::new(PendingTransport {
        started: tokio::sync::Notify::new(),
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    let api = Client::new_with_transport(None, Some(Platform::Web), transport.clone());
    let explicit = credential(101);
    let cancellation = CancellationToken::new();
    let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        tokio::join!(
            write(&api, &explicit, favorite(), cancellation.clone()),
            async {
                transport.started.notified().await;
                cancellation.cancel();
            },
        )
    })
    .await
    .expect("write must observe cancellation");
    assert!(matches!(
        result,
        Err(QmError::Network(NetworkError {
            kind: NetworkErrorKind::Cancelled,
            ..
        }))
    ));
    assert_eq!(transport.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}
