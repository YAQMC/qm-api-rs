//! 业务 endpoint contract tests.
//!
//! 用本地 mock MusicU 服务器验证关键业务链路的完整契约:
//! request → module/method/param → 模拟响应 → typed DTO。
//! 防止以后误改 module/method/param 名、alias 或 DTO 时被静默放行。

use serde_json::{json, Value};
use std::sync::Arc;

use crate::context::ApiContext;
use crate::modules::search::{SearchApi, SearchType};
use crate::modules::song::{SongApi, SongFileInfo, SongFileType};
use crate::modules::top::TopApi;
use crate::versioning::Platform;

/// 启动本地 mock 服务器 (同时服务 CGI session + 业务 + QIMEI), 返回 base URL。
///
/// 对 `/cgi-bin/musicu.fcg`: `GetSession` 返回会话, 其余请求返回 `business_body`;
/// 对 `/tme/trpc/proxy`: 返回固定 QIMEI。
async fn spawn(business_body: &'static str) -> String {
    use axum::extract::Json;
    use axum::routing::post;
    use axum::Router;

    const SESSION_BODY: &str =
        r#"{"code":0,"req_0":{"code":0,"data":{"session":{"uid":"u1","sid":"s1"}}}}"#;
    const QIMEI_BODY: &str = r#"{"data":"{\"data\":{\"q16\":\"q16\",\"q36\":\"q36\"}}"}"#;

    let app = Router::new()
        .route(
            "/cgi-bin/musicu.fcg",
            post(move |Json(payload): Json<Value>| async move {
                let method = payload
                    .get("req_0")
                    .and_then(|r| r.get("method"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if method == "GetSession" {
                    SESSION_BODY.to_string()
                } else {
                    business_body.to_string()
                }
            }),
        )
        .route("/tme/trpc/proxy", post(|| async { QIMEI_BODY }));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

fn ctx_with_mock(base: &str) -> ApiContext {
    let mut ctx = ApiContext::new(None, Some(Platform::Android)).unwrap();
    ctx.cgi_base_url = format!("{base}/cgi-bin");
    ctx.qimei_url = format!("{base}/tme/trpc/proxy");
    ctx
}

#[tokio::test]
async fn song_get_urls_contract() {
    let body = r#"{"code":0,"req_0":{"code":0,"data":{
        "expiration": 80400,
        "midurlinfo":[{"songmid":"001X3HEN1oK0Jr","filename":"xxx.mflac",
                       "purl":"/C400001X3HEN1oK0Jr.mflac?k=1","vkey":"v","ekey":"ekey1","result":0}]
    }}}"#;
    let base = spawn(body).await;
    let song = SongApi::new(Arc::new(ctx_with_mock(&base)));

    let file_type = SongFileType::Flac;
    let resp = song
        .get_song_urls(
            &[SongFileInfo::new("001X3HEN1oK0Jr").with_type(file_type)],
            &SongFileType::Flac,
            None,
        )
        .await
        .unwrap();
    assert_eq!(resp.expiration, 80_400, "expiration 为 TTL 秒数");
    assert_eq!(resp.data.len(), 1);
    assert_eq!(resp.data[0].mid, "001X3HEN1oK0Jr");
    assert_eq!(resp.data[0].ekey, "ekey1");
}

#[tokio::test]
async fn top_category_contract() {
    let body = r#"{"code":0,"req_0":{"code":0,"data":{
        "group":[{"groupId":1,"groupName":"内地","toplist":[{"topId":26,"title":"巅峰榜内地"}]}]
    }}}"#;
    let base = spawn(body).await;
    let top = TopApi::new(Arc::new(ctx_with_mock(&base)));

    let resp = top.get_category().await.unwrap();
    assert_eq!(resp.group.len(), 1);
    assert_eq!(resp.group[0].name, "内地");
}

#[tokio::test]
async fn search_contract() {
    let body = r#"{"code":0,"req_0":{"code":0,"data":{
        "meta":{"searchid":"sid-1","perpage":5,"nextpage":2,"estimate_sum":1,"sum":1},
        "body":{"item_song":[{
            "id":1,"mid":"001X3HEN1oK0Jr","name":"晴天","title":"晴天","subtitle":"",
            "singer":[{"id":6452,"mid":"0025NhlN2yWrP4","name":"周杰伦","title":"周杰伦"}]
        }]}
    }}}"#;
    let base = spawn(body).await;
    let search = SearchApi::new(Arc::new(ctx_with_mock(&base)));

    let resp = search
        .search_by_type("晴天", SearchType::Song, 5, 1, &[], None, true)
        .await
        .unwrap();
    assert_eq!(resp.total_num, 1);
    assert_eq!(resp.song.len(), 1);
    assert_eq!(resp.song[0].base.name, "晴天");
    assert_eq!(resp.song[0].base.singer[0].name, "周杰伦");
}

#[tokio::test]
async fn business_error_code_preserved_in_contract() {
    // 业务错误码 (如 104003 无权限) 应原样出现在 CgiReply, 不被吞掉.
    let body = r#"{"code":0,"req_0":{"code":104003,"data":{"msg":"no permission"}}}"#;
    let base = spawn(body).await;
    let song = SongApi::new(Arc::new(ctx_with_mock(&base)));

    let reply = song
        .base
        .cgi_reply(
            "music.vkey.GetVkey",
            "GetUrl",
            json!({ "songmid": ["001X3HEN1oK0Jr"], "songtype": [0] }),
            crate::context::RequestOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(reply.code, 104003);
}
