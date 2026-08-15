//! 将 QQ 音乐 API 封装为 HTTP 服务 (对应参考库 `web/` 目录的简单演示).
//!
//! 运行: `cargo run --example web_server`
//!
//! 端点:
//! - `GET /search?keyword=周杰伦&type=song&num=5`
//! - `GET /hotkey`
//! - `GET /song/url?mid=0039MnYb0qxYhV`
//! - `GET /song/lyric?mid=0039MnYb0qxYhV`
//! - `GET /toplist`

use axum::{Json, Router, extract::Query, routing::get};
use qqmusic_api::modules::song::{SongFileInfo, SongFileType};
use qqmusic_api::{Client, SearchType};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

type SharedClient = Arc<Client>;

#[derive(Deserialize)]
struct SearchParams {
    keyword: String,
    #[serde(default = "default_type")]
    r#type: String,
    #[serde(default = "default_num")]
    num: i64,
    #[serde(default = "default_page")]
    page: i64,
}

fn default_type() -> String {
    "song".into()
}
fn default_num() -> i64 {
    10
}
fn default_page() -> i64 {
    1
}

fn parse_search_type(t: &str) -> SearchType {
    match t {
        "song" => SearchType::Song,
        "singer" => SearchType::Singer,
        "album" => SearchType::Album,
        "songlist" => SearchType::Songlist,
        "mv" => SearchType::Mv,
        "lyric" => SearchType::Lyric,
        "user" => SearchType::User,
        _ => SearchType::Song,
    }
}

#[derive(Deserialize)]
struct MidParams {
    mid: String,
}

#[derive(Serialize)]
struct ApiResponse {
    code: i64,
    data: Value,
    msg: String,
}

fn ok(data: Value) -> Json<ApiResponse> {
    Json(ApiResponse {
        code: 0,
        data,
        msg: "ok".into(),
    })
}

fn err(e: impl std::fmt::Display) -> Json<ApiResponse> {
    Json(ApiResponse {
        code: -1,
        data: Value::Null,
        msg: e.to_string(),
    })
}

async fn search(client: &SharedClient, params: SearchParams) -> Json<ApiResponse> {
    let search_type = parse_search_type(&params.r#type);
    match client
        .search
        .search_by_type(&params.keyword, search_type, params.num, params.page, &[], None, true)
        .await
    {
        Ok(resp) => {
            let songs: Vec<Value> = resp
                .song
                .iter()
                .map(|s| {
                    json!({
                        "id": s.base.id,
                        "mid": s.base.mid,
                        "name": s.base.name,
                        "singer": s.base.singer.iter().map(|x| x.name.clone()).collect::<Vec<_>>(),
                        "album": s.base.album.name,
                        "interval": s.base.interval,
                    })
                })
                .collect();
            ok(json!({ "total": resp.total_num, "songs": songs }))
        }
        Err(e) => err(e),
    }
}

async fn hotkey(client: &SharedClient) -> Json<ApiResponse> {
    match client.search.get_hotkey().await {
        Ok(resp) => ok(json!({ "hotkey": resp.vec_hotkey.iter().map(|h| json!({ "query": h.query, "title": h.title })).collect::<Vec<_>>() })),
        Err(e) => err(e),
    }
}

async fn song_url(client: &SharedClient, params: MidParams) -> Json<ApiResponse> {
    match client
        .song
        .get_song_urls(
            &[SongFileInfo::new(&params.mid).with_type(SongFileType::Mp3_128)],
            &SongFileType::Mp3_128,
            None,
        )
        .await
    {
        Ok(resp) => {
            let urls: Vec<Value> = resp
                .data
                .iter()
                .filter(|u| !u.purl.is_empty())
                .map(|u| json!({ "mid": u.mid, "purl": u.purl, "vkey": u.vkey, "result": u.result }))
                .collect();
            ok(json!({ "expiration": resp.expiration, "urls": urls }))
        }
        Err(e) => err(e),
    }
}

async fn song_lyric(client: &SharedClient, params: MidParams) -> Json<ApiResponse> {
    match client
        .lyric
        .get_lyric(&params.mid, 1, false, false, false, false)
        .await
    {
        Ok(resp) => ok(json!({ "lyric": resp.lyric, "trans": resp.trans, "roma": resp.roma })),
        Err(e) => err(e),
    }
}

async fn toplist(client: &SharedClient) -> Json<ApiResponse> {
    match client.top.get_category().await {
        Ok(resp) => ok(json!({
            "group": resp.group.iter().map(|g| json!({
                "name": g.name,
                "toplist": g.toplist.iter().map(|t| json!({ "id": t.id, "name": t.name, "total_num": t.total_num })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        })),
        Err(e) => err(e),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Arc::new(Client::new(None, None)?);
    let app = Router::new()
        .route("/search", get(|c: axum::Extension<SharedClient>, q: Query<SearchParams>| async move { search(&c, q.0).await }))
        .route("/hotkey", get(|c: axum::Extension<SharedClient>| async move { hotkey(&c).await }))
        .route("/song/url", get(|c: axum::Extension<SharedClient>, q: Query<MidParams>| async move { song_url(&c, q.0).await }))
        .route("/song/lyric", get(|c: axum::Extension<SharedClient>, q: Query<MidParams>| async move { song_lyric(&c, q.0).await }))
        .route("/toplist", get(|c: axum::Extension<SharedClient>| async move { toplist(&c).await }))
        .layer(axum::Extension(client));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
    println!("QQMusicApi HTTP 服务已启动: http://127.0.0.1:3000");
    println!("  试试: curl 'http://127.0.0.1:3000/search?keyword=周杰伦&num=3'");
    axum::serve(listener, app).await?;
    Ok(())
}
