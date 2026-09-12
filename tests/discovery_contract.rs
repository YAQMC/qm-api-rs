use async_trait::async_trait;
use qqmusic_api::models::discovery::FeedCardKind;
use qqmusic_api::{
    ApiTransport, Client, Credential, HttpBody, Platform, Result, TransportRequest,
    TransportResponse,
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

struct ExpectTransport {
    module: &'static str,
    method: &'static str,
    params: Value,
    response: Value,
    calls: Mutex<usize>,
}

#[async_trait]
impl ApiTransport for ExpectTransport {
    async fn execute(&self, request: TransportRequest) -> Result<TransportResponse> {
        *self.calls.lock().unwrap() += 1;
        assert_eq!(request.url, "https://u.y.qq.com/cgi-bin/musicu.fcg");
        assert!(!request
            .headers
            .iter()
            .any(|(_, value)| value.contains("synthetic-global-token")));
        let HttpBody::Json(body) = request.body else {
            panic!("JSON CGI required")
        };
        assert_eq!(body["comm"], json!({"ct":24,"cv":0}));
        assert_eq!(body["req_0"]["module"], self.module);
        assert_eq!(body["req_0"]["method"], self.method);
        assert_eq!(body["req_0"]["param"], self.params);
        Ok(TransportResponse {
            status: 200,
            final_url: request.url,
            headers: vec![],
            body: serde_json::to_vec(&self.response).unwrap(),
        })
    }
}

fn client(
    module: &'static str,
    method: &'static str,
    params: Value,
    data: Value,
    code: i64,
) -> (Client, Arc<ExpectTransport>) {
    let transport = Arc::new(ExpectTransport {
        module,
        method,
        params,
        response: json!({"code":0,"req_0":{"code":code,"data":data}}),
        calls: Mutex::new(0),
    });
    let client = Client::new_with_transport(
        Some(Credential {
            musicid: 10001,
            str_musicid: "10001".into(),
            musickey: "synthetic-global-token".into(),
            ..Default::default()
        }),
        Some(Platform::Android),
        transport.clone(),
    );
    (client, transport)
}

fn shelf() -> Value {
    json!({"id":1,"v_niche":[{"v_card":[
        {"id":"route?encArea=abc%2Fdef&x=1","title":"Category","cover":{"medium_url":"https://y.gtimg.cn/cover.jpg"}},
        {"id":"7654321","title":"Playlist","type":500,"subtype":0,"cover":"https://y.gtimg.cn/list.jpg"}
    ]}]})
}

#[tokio::test]
async fn discovery_requests_use_exact_contracts_and_typed_cards() {
    let (api, _) = client(
        "music.area.CategoryArea",
        "getCategoryAreaInCategoryPlaylist",
        json!({}),
        json!({"shelf":shelf()}),
        0,
    );
    let categories = api.discovery.categories().await.unwrap();
    assert_eq!(categories.len(), 1);
    assert_eq!(categories[0].area_key, "abc%2Fdef");
    assert_eq!(categories[0].cover_url, "https://y.gtimg.cn/cover.jpg");

    let (api, _) = client(
        "music.longRadio.recommend",
        "getRadioList",
        json!({"pos":6}),
        json!({"radioList":[{"id":"radio","title":"Podcast","subtitle":"Episode","picurl":"https://y.gtimg.cn/radio.jpg"}]}),
        0,
    );
    let podcasts = api.discovery.podcasts().await.unwrap();
    assert_eq!(podcasts[0].id, "radio");
    assert_eq!(podcasts[0].subtitle, "Episode");

    let (api, _) = client(
        "MvService.MvInfoProServer",
        "GetNewMv",
        json!({"style":0,"tag":0,"start":8,"size":8}),
        json!({"list":[{"mvid":1001,"title":"MV","duration":180,"singers":[{"name":"Artist"}]}]}),
        0,
    );
    let mvs = api.discovery.new_mvs(8, 8).await.unwrap();
    assert_eq!(mvs[0].id, "1001");
    assert_eq!(mvs[0].artist, "Artist");
    assert_eq!(mvs[0].duration_seconds, 180);

    let (api, _) = client(
        "music.musicHall.MusicHallPlatformSvr",
        "GetFocus",
        json!({"Device":{"OS":"3","AppName":"QQ音乐"}}),
        json!({"shelf":shelf()}),
        0,
    );
    let cards = api.discovery.featured().await.unwrap();
    assert_eq!(cards.len(), 2);
    assert_eq!(cards[1].kind, FeedCardKind::Playlist);

    let (api, _) = client(
        "music.area.AreaHome",
        "getAreaHomePage",
        json!({"encArea":"abc%2Fdef","cmd":0}),
        json!({"title":"Area","v_shelf":[shelf()]}),
        0,
    );
    let area = api.discovery.area("abc%2Fdef").await.unwrap();
    assert_eq!(area.title, "Area");
    assert_eq!(area.shelves[0].id, Some(1));
    assert_eq!(area.shelves[0].cards.len(), 2);
}

#[tokio::test]
async fn web_chart_preserves_period_artwork_and_audio_metadata() {
    let (api, _) = client(
        "musicToplist.ToplistInfoServer",
        "GetDetail",
        json!({"topId":62,"offset":0,"num":18,"period":""}),
        json!({"data":{"title":"Chart","intro":"Description","magicColor":{"r":1,"g":2,"b":3},
            "topAlbumURL":"https://y.gtimg.cn/chart.jpg","updateTime":"2026-09-12"},
            "songInfoList":[{"mid":"TRACK","title":"Track","file":{"size_128mp3":4096}}]}),
        0,
    );
    let result = api.top.get_web_detail(62, 0, 18, "").await.unwrap();
    assert_eq!(result.info.title, "Chart");
    assert_eq!(result.info.artwork, "https://y.gtimg.cn/chart.jpg");
    assert_eq!(result.info.magic_color.unwrap().b, 3);
    assert_eq!(result.songs[0].file.size_128mp3, 4096);
}

#[tokio::test]
async fn discovery_never_turns_business_errors_or_bad_shapes_into_empty_success() {
    for (data, code) in [(json!({}), 0), (json!({"shelf":shelf()}), 104003)] {
        let (api, transport) = client(
            "music.area.CategoryArea",
            "getCategoryAreaInCategoryPlaylist",
            json!({}),
            data,
            code,
        );
        assert!(api.discovery.categories().await.is_err());
        assert_eq!(*transport.calls.lock().unwrap(), 1);
    }
    let (api, _) = client(
        "music.longRadio.recommend",
        "getRadioList",
        json!({"pos":6}),
        json!({"radioList":{}}),
        0,
    );
    assert!(api.discovery.podcasts().await.is_err());
    let (api, _) = client(
        "MvService.MvInfoProServer",
        "GetNewMv",
        json!({"style":0,"tag":0,"start":0,"size":8}),
        json!({}),
        0,
    );
    assert!(api.discovery.new_mvs(0, 8).await.is_err());
    let (api, _) = client(
        "music.musicHall.MusicHallPlatformSvr",
        "GetFocus",
        json!({"Device":{"OS":"3","AppName":"QQ音乐"}}),
        json!({"shelf":null}),
        0,
    );
    assert!(api.discovery.featured().await.is_err());
    let (api, _) = client(
        "music.area.AreaHome",
        "getAreaHomePage",
        json!({"encArea":"key","cmd":0}),
        json!({"v_shelf":{}}),
        0,
    );
    assert!(api.discovery.area("key").await.is_err());
    let (api, _) = client(
        "musicToplist.ToplistInfoServer",
        "GetDetail",
        json!({"topId":62,"offset":0,"num":18,"period":""}),
        json!({"data":{}}),
        0,
    );
    assert!(api.top.get_web_detail(62, 0, 18, "").await.is_err());
}

#[tokio::test]
async fn discovery_invalid_requests_do_not_reach_transport() {
    let (api, transport) = client("", "", json!({}), json!({}), 0);
    for limit in [0, 101] {
        assert!(api.discovery.new_mvs(0, limit).await.is_err());
    }
    for area in ["", "a\nb"] {
        assert!(api.discovery.area(area).await.is_err());
    }
    assert!(api.top.get_web_detail(0, 0, 18, "").await.is_err());
    assert!(api.top.get_web_detail(62, 0, 101, "").await.is_err());
    assert_eq!(*transport.calls.lock().unwrap(), 0);
}

#[tokio::test]
async fn malformed_discovery_rows_cannot_erase_a_section() {
    for bad_card in [json!(false), json!({}), json!({"id":"item","title":false})] {
        let (api, _) = client(
            "music.longRadio.recommend",
            "getRadioList",
            json!({"pos":6}),
            json!({"radioList":[bad_card.clone()]}),
            0,
        );
        assert!(api.discovery.podcasts().await.is_err());
        let (api, _) = client(
            "music.musicHall.MusicHallPlatformSvr",
            "GetFocus",
            json!({"Device":{"OS":"3","AppName":"QQ音乐"}}),
            json!({"shelf":{"v_niche":[{"v_card":[bad_card]}]}}),
            0,
        );
        assert!(api.discovery.featured().await.is_err());
    }
    for bad_mv in [
        json!(false),
        json!({"mvid":0}),
        json!({"mvid":1,"title":"MV","duration":-1}),
    ] {
        let (api, _) = client(
            "MvService.MvInfoProServer",
            "GetNewMv",
            json!({"style":0,"tag":0,"start":0,"size":8}),
            json!({"list":[bad_mv]}),
            0,
        );
        assert!(api.discovery.new_mvs(0, 8).await.is_err());
    }
    let (api, _) = client(
        "music.musicHall.MusicHallPlatformSvr",
        "GetFocus",
        json!({"Device":{"OS":"3","AppName":"QQ音乐"}}),
        json!({"shelf":{"v_niche":[{}]}}),
        0,
    );
    assert!(api.discovery.featured().await.is_err());
}

#[tokio::test]
async fn discovery_distinguishes_empty_sections_from_unknown_card_kinds() {
    let (api, _) = client(
        "music.longRadio.recommend",
        "getRadioList",
        json!({"pos":6}),
        json!({"radioList":[]}),
        0,
    );
    assert!(api.discovery.podcasts().await.unwrap().is_empty());
    let (api, _) = client(
        "music.musicHall.MusicHallPlatformSvr",
        "GetFocus",
        json!({"Device":{"OS":"3","AppName":"QQ音乐"}}),
        json!({"shelf":{"v_niche":[{"v_card":[{"id":"future-card","title":"Future","type":9999}]}]}}),
        0,
    );
    let cards = api.discovery.featured().await.unwrap();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].id, "future-card");
    assert_eq!(cards[0].kind, FeedCardKind::Other);
}
