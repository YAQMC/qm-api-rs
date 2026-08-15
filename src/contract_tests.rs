//! 业务 endpoint contract tests.
//!
//! Mock 会解析请求 JSON, 校验 `req_0.module` / `req_0.method` / 关键 param / comm,
//! 只有完全匹配时才返回 fixture; 否则返回失败信封, 避免错误 endpoint 仍能 decode 通过.

use serde_json::{json, Value};
use std::sync::Arc;

use crate::context::ApiContext;
use crate::models::comment::CommentBizType;
use crate::models::login::QRLoginType;
use crate::modules::comment::CommentApi;
use crate::modules::login::LoginApi;
use crate::modules::recommend::RecommendApi;
use crate::modules::search::{SearchApi, SearchType};
use crate::modules::song::{SongApi, SongFileInfo, SongFileType};
use crate::modules::songlist::SonglistApi;
use crate::modules::user::UserApi;
use crate::versioning::Platform;
use crate::Credential;

const SESSION_BODY: &str =
    r#"{"code":0,"req_0":{"code":0,"data":{"session":{"uid":"u1","sid":"s1"}}}}"#;
const QIMEI_BODY: &str = r#"{"data":"{\"data\":{\"q16\":\"q16\",\"q36\":\"q36\"}}"}"#;

fn mismatch(reason: &str) -> String {
    format!(r#"{{"code":-1,"message":"contract mismatch: {reason}"}}"#)
}

struct Expect {
    module: &'static str,
    method: &'static str,
    param_ok: fn(&Value) -> bool,
    comm_ok: fn(&Value) -> bool,
    body: &'static str,
}

async fn spawn_strict(expect: Expect) -> String {
    use axum::extract::Json;
    use axum::routing::post;
    use axum::Router;
    use std::sync::Arc;

    let expect = Arc::new(expect);
    let app = Router::new()
        .route(
            "/cgi-bin/musicu.fcg",
            post(move |Json(payload): Json<Value>| {
                let expect = expect.clone();
                async move {
                    let req = payload.get("req_0").cloned().unwrap_or(Value::Null);
                    let module = req.get("module").and_then(Value::as_str).unwrap_or("");
                    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
                    let param = req.get("param").cloned().unwrap_or(Value::Null);
                    let comm = payload.get("comm").cloned().unwrap_or(Value::Null);

                    if module == "music.getSession.session" && method == "GetSession" {
                        return SESSION_BODY.to_string();
                    }
                    if module != expect.module || method != expect.method {
                        return mismatch(&format!("{module}/{method}"));
                    }
                    if !(expect.param_ok)(&param) {
                        return mismatch("param");
                    }
                    if !(expect.comm_ok)(&comm) {
                        return mismatch("comm");
                    }
                    expect.body.to_string()
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

fn android_ctx(base: &str) -> ApiContext {
    let mut ctx = ApiContext::new(None, Some(Platform::Android)).unwrap();
    ctx.cgi_base_url = format!("{base}/cgi-bin");
    ctx.qimei_url = format!("{base}/tme/trpc/proxy");
    let mut device = ctx.device();
    device.qimei = Some("q16".into());
    device.qimei36 = Some("q36".into());
    device.qimei_save_time = Some(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64,
    );
    ctx.set_device(device);
    ctx
}

fn web_ctx(base: &str) -> ApiContext {
    let mut ctx = ApiContext::new(None, Some(Platform::Web)).unwrap();
    ctx.cgi_base_url = format!("{base}/cgi-bin");
    ctx.qimei_url = format!("{base}/tme/trpc/proxy");
    ctx
}

fn login_cred() -> Credential {
    Credential {
        musicid: 10001,
        str_musicid: "10001".into(),
        musickey: "mk".into(),
        encrypt_uin: "enc".into(),
        ..Default::default()
    }
}

fn android_comm_ok(comm: &Value) -> bool {
    comm.get("QIMEI")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.is_empty())
        && comm.get("ct").and_then(Value::as_i64).is_some()
        && comm.get("uid").and_then(Value::as_str).is_some()
}

fn web_comm_ok(comm: &Value) -> bool {
    comm.get("ct").and_then(Value::as_i64).is_some()
}

#[tokio::test]
async fn search_contract_checks_module_method_param() {
    let base = spawn_strict(Expect {
        module: "music.search.SearchCgiService",
        method: "DoSearchForQQMusicMobile",
        param_ok: |p| {
            p.get("query").and_then(Value::as_str) == Some("晴天")
                && p.get("search_type").and_then(Value::as_i64) == Some(0)
                && p.get("num_per_page").and_then(Value::as_i64) == Some(5)
                && p.get("page_num").and_then(Value::as_i64) == Some(1)
        },
        comm_ok: android_comm_ok,
        body: r#"{"code":0,"req_0":{"code":0,"data":{
            "meta":{"searchid":"sid-1","perpage":5,"nextpage":2,"estimate_sum":1,"sum":1},
            "body":{"item_song":[{
                "id":1,"mid":"001X3HEN1oK0Jr","name":"晴天","title":"晴天","subtitle":"",
                "singer":[{"id":6452,"mid":"0025NhlN2yWrP4","name":"周杰伦","title":"周杰伦"}]
            }]}
        }}}"#,
    })
    .await;
    let search = SearchApi::new(Arc::new(android_ctx(&base)));
    let resp = search
        .search_by_type("晴天", SearchType::Song, 5, 1, &[], None, true)
        .await
        .unwrap();
    assert_eq!(resp.total_num, 1);
    assert_eq!(resp.song[0].base.name, "晴天");
}

#[tokio::test]
async fn login_create_qrcode_contract() {
    let base = spawn_strict(Expect {
        module: "music.login.LoginServer",
        method: "CreateQRCode",
        param_ok: |p| p.get("tmeAppID").and_then(Value::as_str) == Some("qqmusic"),
        comm_ok: |c| c.get("ct").and_then(Value::as_i64) == Some(23),
        body: r#"{"code":0,"req_0":{"code":0,"data":{
            "qrcode":"data:image/png;base64,AQID",
            "qrcodeID":"qid-contract-1"
        }}}"#,
    })
    .await;
    let login = LoginApi::new(Arc::new(web_ctx(&base)));
    let qr = login.get_qrcode(QRLoginType::Mobile).await.unwrap();
    assert_eq!(qr.identifier, "qid-contract-1");
    assert_eq!(qr.data, vec![1, 2, 3]);
}

#[tokio::test]
async fn song_url_vkey_contract() {
    let base = spawn_strict(Expect {
        module: "music.vkey.GetVkey",
        method: "UrlGetVkey",
        param_ok: |p| {
            p.get("songmid")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(Value::as_str)
                == Some("001X3HEN1oK0Jr")
        },
        comm_ok: web_comm_ok,
        body: r#"{"code":0,"req_0":{"code":0,"data":{
            "expiration": 80400,
            "midurlinfo":[{"songmid":"001X3HEN1oK0Jr","filename":"xxx.flac",
                           "purl":"/C400001X3HEN1oK0Jr.flac?k=1","vkey":"v","ekey":"","result":0}]
        }}}"#,
    })
    .await;
    let song = SongApi::new(Arc::new(web_ctx(&base)));
    let resp = song
        .get_song_urls(
            &[SongFileInfo::new("001X3HEN1oK0Jr").with_type(SongFileType::Flac)],
            &SongFileType::Flac,
            None,
        )
        .await
        .unwrap();
    assert_eq!(resp.expiration, 80_400);
    assert_eq!(resp.data[0].mid, "001X3HEN1oK0Jr");
}

#[tokio::test]
async fn playlist_read_contract() {
    let base = spawn_strict(Expect {
        module: "music.srfDissInfo.DissInfo",
        method: "CgiGetDiss",
        param_ok: |p| {
            p.get("disstid").and_then(Value::as_i64) == Some(123)
                && p.get("dirid").and_then(Value::as_i64) == Some(201)
                && p.get("song_num").and_then(Value::as_i64) == Some(20)
        },
        comm_ok: web_comm_ok,
        body: r#"{"code":0,"req_0":{"code":0,"data":{
            "dirinfo":{"dissName":"mine"},
            "songlist":[],
            "total_song_num":0
        }}}"#,
    })
    .await;
    let api = SonglistApi::new(Arc::new(web_ctx(&base)));
    let resp = api
        .get_detail(123, 201, 20, 1, false, false, false)
        .await
        .unwrap();
    assert_eq!(resp.total, 0);
}

#[tokio::test]
async fn playlist_create_contract() {
    let base = spawn_strict(Expect {
        module: "music.musicasset.PlaylistBaseWrite",
        method: "AddPlaylist",
        param_ok: |p| p.get("dirName").and_then(Value::as_str) == Some("hardening"),
        comm_ok: web_comm_ok,
        body: r#"{"code":0,"req_0":{"code":0,"data":{
            "retCode":0,"result":{"tid":9,"dirId":301,"dirName":"hardening"}
        }}}"#,
    })
    .await;
    let ctx = web_ctx(&base);
    ctx.set_credential(login_cred());
    let api = SonglistApi::new(Arc::new(ctx));
    let resp = api.create("hardening", None).await.unwrap();
    assert_eq!(resp.retCode, 0);
    assert_eq!(resp.name, "hardening");
}

#[tokio::test]
async fn favorite_like_song_contract() {
    let base = spawn_strict(Expect {
        module: "music.musicasset.PlaylistDetailWrite",
        method: "AddSonglist",
        param_ok: |p| {
            p.get("dirId").and_then(Value::as_i64) == Some(201)
                && p.get("v_songInfo")
                    .and_then(Value::as_array)
                    .and_then(|a| a.first())
                    .and_then(|s| s.get("songId"))
                    .and_then(Value::as_i64)
                    == Some(42)
        },
        comm_ok: web_comm_ok,
        body: r#"{"code":0,"req_0":{"code":0,"data":{"retCode":0}}}"#,
    })
    .await;
    let ctx = web_ctx(&base);
    ctx.set_credential(login_cred());
    let api = SonglistApi::new(Arc::new(ctx));
    assert!(api.like_song(&[(42, 0)], None).await.unwrap());
}

#[tokio::test]
async fn comments_read_contract() {
    let base = spawn_strict(Expect {
        module: "music.globalComment.CommentRead",
        method: "GetHotCommentList",
        param_ok: |p| {
            p.get("BizId").and_then(Value::as_str) == Some("99")
                && p.get("BizType").and_then(Value::as_i64) == Some(0)
                && p.get("HotType").and_then(Value::as_i64) == Some(1)
                && p.get("PageSize").and_then(Value::as_i64) == Some(10)
        },
        comm_ok: web_comm_ok,
        body: r#"{"code":0,"req_0":{"code":0,"data":{
            "CommentList":{"Comments":[],"HasMore":0,"Total":0}
        }}}"#,
    })
    .await;
    let api = CommentApi::new(Arc::new(web_ctx(&base)));
    let resp = api
        .get_hot_comments(99, 1, 10, "", CommentBizType::Song, None)
        .await
        .unwrap();
    assert_eq!(resp.total, 0);
}

#[tokio::test]
async fn recommend_home_feed_contract() {
    let base = spawn_strict(Expect {
        module: "music.recommend.RecommendFeed",
        method: "get_recommend_feed",
        param_ok: |p| {
            p.get("page").and_then(Value::as_i64) == Some(1)
                && p.get("direction").and_then(Value::as_i64) == Some(0)
                && p.get("s_num").and_then(Value::as_i64) == Some(5)
        },
        comm_ok: web_comm_ok,
        body: r#"{"code":0,"req_0":{"code":0,"data":{
            "retcode":0,"v_shelf":[{"id":1,"title_content":"for you","v_niche":[]}]
        }}}"#,
    })
    .await;
    let api = RecommendApi::new(Arc::new(web_ctx(&base)));
    let resp = api.get_home_feed(1, 0, 5, &[]).await.unwrap();
    assert_eq!(resp.shelves.len(), 1);
    assert_eq!(resp.shelves[0].title_content, "for you");
}

#[tokio::test]
async fn account_homepage_contract() {
    let base = spawn_strict(Expect {
        module: "music.UnifiedHomepage.UnifiedHomepageSrv",
        method: "GetHomepageHeader",
        param_ok: |p| {
            p.get("uin").and_then(Value::as_str) == Some("u-enc")
                && p.get("IsQueryTabDetail").and_then(Value::as_i64) == Some(1)
        },
        comm_ok: web_comm_ok,
        body: r#"{"code":0,"req_0":{"code":0,"data":{
            "Info":{"BaseInfo":{"Name":"tester","EncryptedUin":"u-enc"}}
        }}}"#,
    })
    .await;
    let api = UserApi::new(Arc::new(web_ctx(&base)));
    let resp = api.get_homepage("u-enc", None).await.unwrap();
    assert_eq!(resp.base_info.name, "tester");
}

#[tokio::test]
async fn wrong_module_method_is_rejected() {
    // Mock 只接受 Search; 用 Song URL 打过去必须失败, 不能 fallback 到任意 business body.
    let base = spawn_strict(Expect {
        module: "music.search.SearchCgiService",
        method: "DoSearchForQQMusicMobile",
        param_ok: |_| true,
        comm_ok: |_| true,
        body: r#"{"code":0,"req_0":{"code":0,"data":{"expiration":1,"midurlinfo":[]}}}"#,
    })
    .await;
    let song = SongApi::new(Arc::new(web_ctx(&base)));
    let err = song
        .get_song_urls(
            &[SongFileInfo::new("001X3HEN1oK0Jr").with_type(SongFileType::Flac)],
            &SongFileType::Flac,
            None,
        )
        .await
        .unwrap_err();
    match err {
        crate::error::QmError::GlobalApi { code, .. } => assert_eq!(code, -1),
        other => panic!("expected GlobalApi mismatch, got {other:?}"),
    }
}

#[tokio::test]
async fn business_error_code_preserved_in_contract() {
    let base = spawn_strict(Expect {
        module: "music.vkey.GetVkey",
        method: "GetUrl",
        param_ok: |_| true,
        comm_ok: |_| true,
        body: r#"{"code":0,"req_0":{"code":104003,"data":{"msg":"no permission"}}}"#,
    })
    .await;
    let song = SongApi::new(Arc::new(web_ctx(&base)));
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
