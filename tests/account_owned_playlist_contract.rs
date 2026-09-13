use std::sync::{Arc, Mutex};

use qqmusic_api::{
    account::{read_page, AccountRead},
    ApiTransport, CancellationToken, Client, Credential, HttpBody, HttpMethod, Platform, QmError,
    TransportRequest, TransportResponse,
};
use serde_json::{json, Value};

#[derive(Default)]
struct RecordingTransport(Mutex<Vec<Value>>);

#[async_trait::async_trait]
impl ApiTransport for RecordingTransport {
    async fn execute(&self, request: TransportRequest) -> qqmusic_api::Result<TransportResponse> {
        assert_eq!(request.method, HttpMethod::Post);
        assert_eq!(request.url, "https://u.y.qq.com/cgi-bin/musicu.fcg");
        let HttpBody::Json(payload) = request.body else {
            panic!("expected JSON");
        };
        self.0.lock().unwrap().push(payload);
        Ok(TransportResponse {
            status: 200,
            final_url: request.url,
            headers: vec![],
            body: serde_json::to_vec(&json!({"code":0,"req":{"code":0,"data":{
                "song_begin":0,"total_song_num":1,"songlist":[{"mid":"track"}],
                "dirinfo":{"id":3001,"dirid":3001}
            }}}))
            .unwrap(),
        })
    }
}

fn credential() -> Credential {
    Credential {
        musicid: 10001,
        musickey: "SYNTHETIC_KEY".into(),
        ..Credential::default()
    }
}

#[tokio::test]
async fn owned_binding_changes_validation_not_the_playlist_request_or_credential() {
    let transport = Arc::new(RecordingTransport::default());
    let client = Client::new_with_transport(None, Some(Platform::Android), transport.clone());
    let page = read_page(
        &client,
        &credential(),
        AccountRead::OwnedPlaylistTracks {
            tid: "7654321".into(),
            dir_id: 3001,
        },
        0,
        100,
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(page.row_count, 1);
    assert_eq!(page.next_offset, None);
    assert!(matches!(
        read_page(
            &client,
            &credential(),
            AccountRead::PlaylistTracks {
                tid: "7654321".into(),
            },
            0,
            100,
            CancellationToken::new()
        )
        .await,
        Err(QmError::ApiData(_))
    ));
    let requests = transport.0.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0], requests[1]);
    assert_eq!(requests[0]["comm"]["uin"], "10001");
    assert_eq!(
        requests[0]["req"],
        json!({
            "module":"music.srfDissInfo.DissInfo","method":"CgiGetDiss","param":{
                "disstid":7654321,"dirid":0,"tag":true,"song_begin":0,"song_num":100,
                "userinfo":true,"orderlist":true,"onlysonglist":1
            }
        })
    );
}

#[tokio::test]
async fn invalid_directory_binding_and_cancelled_reads_never_send() {
    let transport = Arc::new(RecordingTransport::default());
    let client = Client::new_with_transport(None, None, transport.clone());
    assert!(matches!(
        read_page(
            &client,
            &credential(),
            AccountRead::OwnedPlaylistTracks {
                tid: "7654321".into(),
                dir_id: 0,
            },
            0,
            100,
            CancellationToken::new()
        )
        .await,
        Err(QmError::ValueError(_))
    ));
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(read_page(
        &client,
        &credential(),
        AccountRead::OwnedPlaylistTracks {
            tid: "7654321".into(),
            dir_id: 3001,
        },
        0,
        100,
        cancelled
    )
    .await
    .is_err());
    assert!(transport.0.lock().unwrap().is_empty());
}
