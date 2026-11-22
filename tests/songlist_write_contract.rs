use async_trait::async_trait;
use qqmusic_api::{
    ApiTransport, Client, Credential, HttpBody, HttpMethod, Platform, Result, TransportRequest,
    TransportResponse,
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct RecordingTransport(Mutex<Vec<Value>>);

#[async_trait]
impl ApiTransport for RecordingTransport {
    async fn execute(&self, request: TransportRequest) -> Result<TransportResponse> {
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
            body: serde_json::to_vec(&json!({
                "code":0,
                "req_0":{"code":0,"data":{"retCode":0}}
            }))
            .unwrap(),
        })
    }
}

fn credential() -> Credential {
    Credential {
        musicid: 10001,
        str_musicid: "10001".into(),
        musickey: "synthetic-key".into(),
        ..Default::default()
    }
}

#[tokio::test]
async fn add_delete_and_reorder_convert_read_song_types_at_the_wire_boundary() {
    let transport = Arc::new(RecordingTransport::default());
    let client =
        Client::new_with_transport(Some(credential()), Some(Platform::Web), transport.clone());
    let songs = [(100, 1), (101, 5), (102, 111), (103, 2)];

    assert!(client.songlist.add_songs(7, &songs, 9, None).await.unwrap());
    assert!(client.songlist.del_songs(7, &songs, 9, None).await.unwrap());
    assert!(client
        .songlist
        .reorder_songs(7, &songs, 9, None)
        .await
        .unwrap());

    let requests = transport.0.lock().unwrap();
    assert_eq!(requests[0]["req_0"]["method"], "AddSonglist");
    assert_eq!(requests[1]["req_0"]["method"], "DelSonglist");
    assert_eq!(requests[2]["req_0"]["method"], "SeqSonglist");
    assert_eq!(
        requests[0]["req_0"]["param"]["v_songInfo"],
        json!([
            {"songId":100,"songType":0},
            {"songId":101,"songType":5},
            {"songId":102,"songType":111},
            {"songId":103,"songType":1}
        ])
    );
    assert_eq!(
        requests[2]["req_0"]["param"],
        json!({
            "dirId":7,
            "tid":9,
            "v_songPair":[
                {"songId":100,"songType":0},
                {"songId":101,"songType":5},
                {"songId":102,"songType":111},
                {"songId":103,"songType":1}
            ]
        })
    );
}

#[tokio::test]
async fn invalid_reorder_is_rejected_without_a_request() {
    let transport = Arc::new(RecordingTransport::default());
    let client =
        Client::new_with_transport(Some(credential()), Some(Platform::Web), transport.clone());
    assert!(client
        .songlist
        .reorder_songs(0, &[(1, 1)], 0, None)
        .await
        .is_err());
    assert!(client
        .songlist
        .reorder_songs(1, &[], 0, None)
        .await
        .is_err());
    assert!(transport.0.lock().unwrap().is_empty());
}
