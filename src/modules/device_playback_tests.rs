use super::*;
use crate::{
    ApiTransport, Client, HttpBody, Platform, RetryClass, TransportRequest, TransportResponse,
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Fixture {
    response: Mutex<Option<Vec<u8>>>,
    requests: Mutex<Vec<RecordedRequest>>,
}

struct RecordedRequest {
    url: String,
    query_keys: Vec<String>,
    retry: RetryClass,
    max_response_bytes: Option<usize>,
    body: Value,
}

#[async_trait::async_trait]
impl ApiTransport for Fixture {
    async fn execute(&self, request: TransportRequest) -> Result<TransportResponse> {
        let HttpBody::Json(body) = request.body else {
            panic!("expected JSON CGI request");
        };
        self.requests.lock().unwrap().push(RecordedRequest {
            url: request.url.clone(),
            query_keys: request.query.iter().map(|(key, _)| key.clone()).collect(),
            retry: request.retry,
            max_response_bytes: request.max_response_bytes,
            body,
        });
        let body = self
            .response
            .lock()
            .unwrap()
            .take()
            .expect("unexpected extra request");
        Ok(TransportResponse {
            status: 200,
            final_url: request.url,
            headers: Vec::new(),
            body,
        })
    }
}

fn client(response: Value) -> (Client, Arc<Fixture>) {
    let fixture = Arc::new(Fixture {
        response: Mutex::new(Some(serde_json::to_vec(&response).unwrap())),
        ..Default::default()
    });
    let client = Client::new_with_transport(None, Some(Platform::Web), fixture.clone());
    (client, fixture)
}

fn successful_reply(data: Value) -> Value {
    json!({"code":0,"req_0":{"code":0,"data":data}})
}

#[tokio::test]
async fn other_platform_snapshot_uses_signed_read_and_decodes_observed_fields() {
    let (client, fixture) = client(successful_reply(json!({
        "ct": 24,
        "cv": 4_747_474,
        "updateTimestampSec": 1_700_000_000_i64,
        "playList": [11, 22, 11],
        "listeningSongID": 22,
        "text": "{\"txt\":\"PC端播过：\",\"songName\":\"fixture-track\"}",
        "keepSec": 900
    })));

    let snapshot = client
        .device_playback
        .get_other_platform_current_playlist_for(&tests_credential())
        .await
        .unwrap();

    assert_eq!(snapshot.ct, 24);
    assert_eq!(snapshot.cv, 4_747_474);
    assert_eq!(snapshot.update_timestamp_sec, 1_700_000_000);
    assert_eq!(snapshot.play_list, [11, 22, 11]);
    assert_eq!(snapshot.listening_song_id, 22);
    assert_eq!(snapshot.keep_sec, 900);
    assert!(snapshot.has_current_song());
    assert_eq!(
        snapshot.display_text(),
        Some(crate::models::device_playback::OtherPlatformPlaylistText {
            text: "PC端播过：".into(),
            song_name: "fixture-track".into(),
        })
    );

    let requests = fixture.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.url, "https://u.y.qq.com/cgi-bin/musics.fcg");
    assert_eq!(request.retry, RetryClass::SafeRead);
    assert_eq!(
        request.max_response_bytes,
        Some(MAX_OTHER_PLATFORM_PLAYLIST_RESPONSE_BYTES)
    );
    let query_keys: Vec<_> = request.query_keys.iter().map(String::as_str).collect();
    assert!(query_keys.contains(&"sign"));
    assert!(query_keys.contains(&"_"));
    let body = &request.body;
    assert_eq!(body["req_0"]["module"], MODULE);
    assert_eq!(body["req_0"]["method"], METHOD);
    assert_eq!(body["req_0"]["param"], json!({}));
}

#[tokio::test]
async fn other_platform_snapshot_requires_login() {
    let (client, fixture) = client(successful_reply(json!({})));
    let error = client
        .device_playback
        .get_other_platform_current_playlist()
        .await
        .unwrap_err();
    assert!(matches!(error, QmError::CredentialInvalid(_)));
    assert!(fixture.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn invalid_optional_text_does_not_discard_current_snapshot() {
    let (client, _) = client(successful_reply(json!({
        "listeningSongID": 42,
        "playList": [42],
        "text": "not-json"
    })));
    let snapshot = client
        .device_playback
        .get_other_platform_current_playlist_for(&tests_credential())
        .await
        .unwrap();
    assert!(snapshot.has_current_song());
    assert!(snapshot.display_text().is_none());
}

#[tokio::test]
async fn other_platform_snapshot_rejects_invalid_playlist_shape() {
    let (client, _) = client(successful_reply(json!({"playList":"not-an-array"})));
    let error = client
        .device_playback
        .get_other_platform_current_playlist_for(&tests_credential())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        QmError::Protocol {
            stage: "other-platform-playlist",
            ..
        }
    ));
}

#[tokio::test]
async fn listening_report_is_explicit_signed_and_non_retrying() {
    let (client, fixture) = client(successful_reply(json!({
        "code": 0,
        "errmsg": "",
        "needPlayList": 1
    })));
    let request = ListeningMusicReportRequest {
        song_mid: "fixture-mid".into(),
        song_type: 0,
        pause_flag: false,
        remaining_time_ms: 31_000,
        song_play_time_ms: 220_000,
        song_id: 123,
        speed: 1.0,
        icon2: 7,
        play_list: Some(vec![11, 22, 33]),
    };
    let credential = tests_credential();

    let response = client
        .device_playback
        .report_listening_music(&request, &credential, crate::CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(response.code, 0);
    assert_eq!(response.need_play_list, 1);
    let calls = fixture.requests.lock().unwrap();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(call.retry, RetryClass::Write);
    assert_eq!(call.url, "https://u.y.qq.com/cgi-bin/musics.fcg");
    assert_eq!(
        call.max_response_bytes,
        Some(MAX_LISTENING_REPORT_RESPONSE_BYTES)
    );
    assert!(call.query_keys.iter().any(|key| key == "sign"));
    let body = &call.body;
    assert_eq!(body["req_0"]["module"], MODULE);
    assert_eq!(body["req_0"]["method"], REPORT_METHOD);
    assert_eq!(body["req_0"]["param"]["songmid"], "fixture-mid");
    assert_eq!(body["req_0"]["param"]["songtype"], 0);
    assert_eq!(body["req_0"]["param"]["pauseFlag"], false);
    assert_eq!(body["req_0"]["param"]["remainingTime"], 31_000);
    assert_eq!(body["req_0"]["param"]["songPlayTime"], 220_000);
    assert_eq!(body["req_0"]["param"]["songid"], 123);
    assert_eq!(body["req_0"]["param"]["speed"], 1.0);
    assert_eq!(body["req_0"]["param"]["icon2"], 7);
    assert_eq!(body["req_0"]["param"]["playList"], json!([11, 22, 33]));
}

#[tokio::test]
async fn listening_report_rejects_invalid_fields_before_transport() {
    let (client, fixture) = client(successful_reply(json!({
        "code": 0,
        "errmsg": "",
        "needPlayList": 0
    })));
    let mut request = ListeningMusicReportRequest {
        song_mid: "fixture-mid".into(),
        song_type: 0,
        pause_flag: false,
        remaining_time_ms: 0,
        song_play_time_ms: 0,
        song_id: 123,
        speed: 1.0,
        icon2: 0,
        play_list: None,
    };
    request.speed = f32::NAN;
    let error = client
        .device_playback
        .report_listening_music(
            &request,
            &tests_credential(),
            crate::CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, QmError::ValueError(_)));
    assert!(fixture.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn listening_report_bounds_the_observed_queue_slice() {
    let (client, fixture) = client(successful_reply(json!({
        "code": 0,
        "errmsg": "",
        "needPlayList": 0
    })));
    let request = ListeningMusicReportRequest {
        song_mid: "fixture-mid".into(),
        song_type: 0,
        pause_flag: false,
        remaining_time_ms: 0,
        song_play_time_ms: 0,
        song_id: 123,
        speed: 1.0,
        icon2: 0,
        play_list: Some(vec![1; MAX_LISTENING_REPORT_PLAYLIST_ITEMS + 1]),
    };
    let error = client
        .device_playback
        .report_listening_music(
            &request,
            &tests_credential(),
            crate::CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, QmError::ValueError(_)));
    assert!(fixture.requests.lock().unwrap().is_empty());
}

fn tests_credential() -> crate::Credential {
    crate::Credential {
        musicid: 12345,
        musickey: "synthetic-account-token".into(),
        ..Default::default()
    }
}
