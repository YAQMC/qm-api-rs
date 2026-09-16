use async_trait::async_trait;
use qqmusic_api::models::discovery::FeedCardKind;
use qqmusic_api::{
    ApiTransport, Client, Credential, HttpBody, Platform, QmError, Result, TransportRequest,
    TransportResponse,
};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

struct Expected {
    module: &'static str,
    method: &'static str,
    param: Value,
    data: Value,
    code: i64,
    personalized: bool,
    calls: AtomicUsize,
}

#[async_trait]
impl ApiTransport for Expected {
    async fn execute(&self, request: TransportRequest) -> Result<TransportResponse> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        assert_eq!(request.url, "https://u.y.qq.com/cgi-bin/musicu.fcg");
        assert_eq!(request.retry, qqmusic_api::RetryClass::SafeRead);
        let cookie = request
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("cookie"))
            .map(|(_, v)| v.as_str())
            .unwrap_or("");
        assert!(!cookie.contains("synthetic-default-token"));
        if self.personalized {
            assert!(cookie.contains("synthetic-explicit-token"));
            assert!(cookie.contains("uin=20002"));
        } else {
            assert!(cookie.is_empty());
        }
        let HttpBody::Json(body) = request.body else {
            panic!("JSON CGI expected");
        };
        assert_eq!(body["comm"], json!({"ct":24,"cv":0}));
        assert_eq!(body["req_0"]["module"], self.module);
        assert_eq!(body["req_0"]["method"], self.method);
        assert_eq!(body["req_0"]["param"], self.param);
        Ok(TransportResponse {
            status: 200,
            final_url: request.url,
            headers: vec![],
            body: serde_json::to_vec(
                &json!({"code":0,"req_0":{"code":self.code,"data":self.data}}),
            )
            .unwrap(),
        })
    }
}

fn fixture(
    module: &'static str,
    method: &'static str,
    param: Value,
    data: Value,
    code: i64,
    personalized: bool,
) -> (Client, Arc<Expected>) {
    let transport = Arc::new(Expected {
        module,
        method,
        param,
        data,
        code,
        personalized,
        calls: AtomicUsize::new(0),
    });
    let client = Client::new_with_transport(
        Some(Credential {
            musicid: 10001,
            musickey: "synthetic-default-token".into(),
            ..Default::default()
        }),
        Some(Platform::Android),
        transport.clone(),
    );
    (client, transport)
}

fn account() -> Credential {
    Credential {
        musicid: 20002,
        musickey: "synthetic-explicit-token".into(),
        ..Default::default()
    }
}

#[tokio::test]
async fn personalized_web_feed_uses_explicit_account_and_preserves_card_semantics() {
    let (api, transport) = fixture(
        "music.recommend.RecommendFeed",
        "get_recommend_feed",
        json!({"direction":0,"page":2,"s_num":1,"v_cache":["7"]}),
        json!({"retcode":0,"v_shelf":[{"id":8,"v_niche":[{"v_card":[
            {"id":"7654321","title":"Songs","type":500,"subtype":511},
            {"id":"7654322","title":"Playlist","type":500,"subtype":0},
            {"id":"7654323","title":"Daily 30","type":500,"subtype":510},
            {"id":"","title":"More","type":-1},
            {"id":"future","title":"Future","type":9999}
        ]}]}]}),
        0,
        true,
    );
    let shelves = api
        .recommend
        .get_web_home_feed(2, 1, &["7".into()], &account())
        .await
        .unwrap();
    assert_eq!(shelves.len(), 1);
    assert_eq!(shelves[0].id, Some(8));
    assert_eq!(shelves[0].cards[0].kind, FeedCardKind::NewSongs);
    assert_eq!(shelves[0].cards[1].kind, FeedCardKind::Playlist);
    assert_eq!(shelves[0].cards[2].kind, FeedCardKind::DailySonglist);
    assert_eq!(shelves[0].cards[3].kind, FeedCardKind::Other);
    assert_eq!(shelves[0].cards[4].kind, FeedCardKind::Other);
    assert_eq!(api.credential().musicid, 10001);
    assert_eq!(transport.calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn public_web_feeds_preserve_metadata_order_and_duplicate_songs() {
    let (api, _) = fixture(
        "music.playlist.PlaylistSquare",
        "GetRecommendFeed",
        json!({"From":8,"Size":8}),
        json!({"List":[{"Playlist":{"basic":{"tid":7654321,"dissname":"First","desc":"Description",
            "cover":{"medium_url":"https://y.gtimg.cn/cover.jpg"},"creator":{"nick":"Curator"}}}},
            {"Playlist":{"basic":{"dissid":"7654322","title":"Second","creator_nick":"Fallback"}}}]}),
        0,
        false,
    );
    let rows = api.recommend.get_web_songlists(8, 8).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].id, 7654321);
    assert_eq!(rows[0].description, "Description");
    assert_eq!(rows[0].cover_url, "https://y.gtimg.cn/cover.jpg");
    assert_eq!(rows[0].creator, "Curator");
    assert_eq!(rows[1].creator, "Fallback");
    let (api, _) = fixture(
        "newsong.NewSongServer",
        "get_new_song_info",
        json!({"type":5}),
        json!({"songlist":[{"mid":"A","file":{"size_128mp3":4096}},{"mid":"B"},{"mid":"A"}]}),
        0,
        false,
    );
    let songs = api.recommend.get_web_newsongs(5).await.unwrap();
    assert_eq!(
        songs.iter().map(|s| s.mid.as_str()).collect::<Vec<_>>(),
        ["A", "B", "A"]
    );
    assert_eq!(songs[0].file.size_128mp3, 4096);
}

#[tokio::test]
async fn web_feed_rejects_errors_and_bad_shapes_without_empty_success() {
    for (code, category) in [(104401, "auth"), (2001, "rate-limit"), (104003, "business")] {
        let (api, _) = fixture(
            "music.recommend.RecommendFeed",
            "get_recommend_feed",
            json!({"direction":0,"page":1,"s_num":0,"v_cache":[]}),
            json!({"retcode":code}),
            0,
            true,
        );
        let error = api
            .recommend
            .get_web_home_feed(1, 0, &[], &account())
            .await
            .unwrap_err();
        assert!(match category {
            "auth" => matches!(error, QmError::CredentialExpired(_)),
            "rate-limit" => matches!(error, QmError::RateLimited),
            _ => matches!(error, QmError::CgiApi { code: 104003, .. }),
        });
    }
    for data in [
        json!({}),
        json!({"v_shelf":[{}]}),
        json!({"retcode":104003,"v_shelf":[]}),
    ] {
        let (api, _) = fixture(
            "music.recommend.RecommendFeed",
            "get_recommend_feed",
            json!({"direction":0,"page":1,"s_num":0,"v_cache":[]}),
            data,
            0,
            true,
        );
        assert!(api
            .recommend
            .get_web_home_feed(1, 0, &[], &account())
            .await
            .is_err());
    }
    for data in [
        json!({}),
        json!({"List":[{}]}),
        json!({"List":[{"Playlist":{"basic":{"id":0,"title":"Invalid"}}}]}),
    ] {
        let (api, _) = fixture(
            "music.playlist.PlaylistSquare",
            "GetRecommendFeed",
            json!({"From":0,"Size":8}),
            data,
            0,
            false,
        );
        assert!(api.recommend.get_web_songlists(0, 8).await.is_err());
    }
    for data in [
        json!({}),
        json!({"songlist":[{}]}),
        json!({"songlist":[false]}),
    ] {
        let (api, _) = fixture(
            "newsong.NewSongServer",
            "get_new_song_info",
            json!({"type":5}),
            data,
            0,
            false,
        );
        assert!(api.recommend.get_web_newsongs(5).await.is_err());
    }
    let (api, _) = fixture(
        "newsong.NewSongServer",
        "get_new_song_info",
        json!({"type":5}),
        json!({"songlist":[]}),
        104003,
        false,
    );
    assert!(matches!(
        api.recommend.get_web_newsongs(5).await,
        Err(QmError::CgiApi { code: 104003, .. })
    ));
}

#[tokio::test]
async fn web_feed_accepts_real_empty_results_and_rejects_invalid_input_before_transport() {
    let (api, _) = fixture(
        "newsong.NewSongServer",
        "get_new_song_info",
        json!({"type":5}),
        json!({"songlist":[]}),
        0,
        false,
    );
    assert!(api.recommend.get_web_newsongs(5).await.unwrap().is_empty());
    let (api, _) = fixture(
        "music.playlist.PlaylistSquare",
        "GetRecommendFeed",
        json!({"From":0,"Size":8}),
        json!({"List":[]}),
        0,
        false,
    );
    assert!(api
        .recommend
        .get_web_songlists(0, 8)
        .await
        .unwrap()
        .is_empty());
    let (api, _) = fixture(
        "music.recommend.RecommendFeed",
        "get_recommend_feed",
        json!({"direction":0,"page":1,"s_num":0,"v_cache":[]}),
        json!({"v_shelf":[]}),
        0,
        true,
    );
    assert!(api
        .recommend
        .get_web_home_feed(1, 0, &[], &account())
        .await
        .unwrap()
        .is_empty());
    let (api, transport) = fixture("", "", json!({}), json!({}), 0, false);
    assert!(api
        .recommend
        .get_web_home_feed(0, 0, &[], &account())
        .await
        .is_err());
    assert!(api
        .recommend
        .get_web_home_feed(1, 0, &vec!["1".into(); 101], &account())
        .await
        .is_err());
    assert!(api
        .recommend
        .get_web_home_feed(1, 0, &["a\nb".into()], &account())
        .await
        .is_err());
    assert!(matches!(
        api.recommend
            .get_web_home_feed(1, 0, &[], &Credential::default())
            .await,
        Err(QmError::CredentialInvalid(_))
    ));
    for limit in [0, 101] {
        assert!(api.recommend.get_web_songlists(0, limit).await.is_err());
    }
    assert_eq!(transport.calls.load(Ordering::Relaxed), 0);
}
