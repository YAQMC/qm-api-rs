use async_trait::async_trait;
use qqmusic_api::{
    AlbumFilterType, AlbumTabOptions, ApiTransport, Client, HttpBody, HttpMethod, OrderType,
    Platform, Result, SingerTabExtension, TransportRequest, TransportResponse, VideoTabOptions,
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
        self.0.lock().unwrap().push(payload.clone());
        let method = payload["req_0"]["method"].as_str().unwrap();
        let data = match method {
            "GetSingerMvTag" => json!({
                "list":[{"id":2,"name":"官方"},{"ID":7,"Title":"现场"}]
            }),
            "GetSingerMvList" => json!({
                "total":1,
                "list":[{"mvid":9,"vid":"mv-vid","title":"MV"}]
            }),
            "GetHomepageTabDetail" => json!({
                "TabID":"album",
                "HasMore":1,
                "AlbumTab":{
                    "AlbumList":[{"albumID":3,"albumMid":"album-mid","albumName":"Album","tags":null}],
                    "TypeList":{"DefaultID":1,"ItemList":[{"ID":0,"Title":"录音室"},{"ID":1,"Title":"现场"}]}
                }
            }),
            "GetSingerSongList" => json!({"singerMid":"singer","totalNum":0,"songList":[]}),
            "GetAlbumList" => json!({"singerMid":"singer","total":0,"albumList":[]}),
            _ => panic!("unexpected method: {method}"),
        };
        Ok(TransportResponse {
            status: 200,
            final_url: request.url,
            headers: vec![],
            body: serde_json::to_vec(&json!({
                "code":0,
                "req_0":{"code":0,"data":data}
            }))
            .unwrap(),
        })
    }
}

#[tokio::test]
async fn singer_mv_tags_filters_and_order_are_sent_and_decoded() {
    let transport = Arc::new(RecordingTransport::default());
    let client = Client::new_with_transport(None, Some(Platform::Web), transport.clone());

    let tags = client.singer.get_mv_tag("singer").await.unwrap();
    assert_eq!(tags.tags.len(), 2);
    assert_eq!(tags.tags[1].id, 7);
    assert_eq!(tags.tags[1].name, "现场");

    let list = client
        .singer
        .get_mv_list_with_filter("singer", 20, 2, OrderType::Latest, Some(7))
        .await
        .unwrap();
    assert_eq!(list.total, 1);
    assert_eq!(list.mv_list[0].base.vid, "mv-vid");

    let albums = client
        .singer
        .get_album_list_with_filter(
            "singer",
            10,
            1,
            OrderType::Latest,
            Some(AlbumFilterType::Live),
        )
        .await
        .unwrap();
    assert!(albums.album_list.is_empty());

    let songs = client
        .singer
        .get_songs_list_with_order("singer", 10, 1, OrderType::Hot)
        .await
        .unwrap();
    assert_eq!(songs.singer_mid, "singer");

    let tab = client
        .singer
        .get_tab_detail_with_options(
            "singer",
            qqmusic_api::TabType::Album,
            2,
            10,
            OrderType::Latest,
            Some(SingerTabExtension::Album(AlbumTabOptions {
                filter_type: Some(AlbumFilterType::Live),
                include_filter_list: true,
            })),
        )
        .await
        .unwrap();
    assert_eq!(tab.album_tab.len(), 1);
    assert_eq!(tab.album_type_list.default_id, 1);
    assert_eq!(tab.album_type_list.items[1].id, 1);

    let requests = transport.0.lock().unwrap();
    assert_eq!(requests[0]["req_0"]["param"], json!({"singermid":"singer"}));
    assert_eq!(
        requests[1]["req_0"]["param"],
        json!({"singermid":"singer","order":0,"count":20,"start":20,"tagid":7})
    );
    assert_eq!(
        requests[2]["req_0"]["param"],
        json!({"singerMid":"singer","order":0,"number":10,"begin":0,"types":[1]})
    );
    assert_eq!(
        requests[3]["req_0"]["param"],
        json!({"singerMid":"singer","order":1,"number":10,"begin":0})
    );
    assert_eq!(
        requests[4]["req_0"]["param"],
        json!({
            "SingerMid":"singer",
            "IsQueryTabDetail":1,
            "TabID":"album",
            "PageNum":1,
            "PageSize":10,
            "Order":0,
            "AlbumExtension":{"IsNeedFilterType":1,"FilterType":1}
        })
    );
}

#[tokio::test]
async fn mismatched_tab_extension_and_invalid_pages_never_send() {
    let transport = Arc::new(RecordingTransport::default());
    let client = Client::new_with_transport(None, Some(Platform::Web), transport.clone());
    let error = client
        .singer
        .get_tab_detail_with_options(
            "singer",
            qqmusic_api::TabType::Video,
            1,
            10,
            OrderType::Latest,
            Some(SingerTabExtension::Album(AlbumTabOptions::default())),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, qqmusic_api::QmError::ValueError(_)));
    let error = client
        .singer
        .get_tab_detail_with_options(
            "singer",
            qqmusic_api::TabType::Video,
            1,
            10,
            OrderType::Latest,
            Some(SingerTabExtension::Video(VideoTabOptions {
                tag_id: Some(-1),
                include_tag_list: false,
            })),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, qqmusic_api::QmError::ValueError(_)));
    assert!(client.singer.get_mv_tag(" ").await.is_err());
    assert!(transport.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn video_tab_extension_uses_the_video_wire_key() {
    let transport = Arc::new(RecordingTransport::default());
    let client = Client::new_with_transport(None, Some(Platform::Web), transport.clone());
    let _ = client
        .singer
        .get_tab_detail_with_options(
            "singer",
            qqmusic_api::TabType::Video,
            1,
            5,
            OrderType::Hot,
            Some(SingerTabExtension::Video(VideoTabOptions {
                tag_id: Some(2),
                include_tag_list: true,
            })),
        )
        .await;
    let requests = transport.0.lock().unwrap();
    assert_eq!(
        requests[0]["req_0"]["param"]["VideoExtension"],
        json!({"TagID":2,"IsNeedTagList":1})
    );
}
