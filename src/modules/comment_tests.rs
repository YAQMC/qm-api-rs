use super::*;
use crate::{ApiTransport, Client, HttpBody, Platform, TransportRequest, TransportResponse};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Fixture {
    responses: Mutex<VecDeque<Vec<u8>>>,
    calls: Mutex<Vec<Value>>,
}

#[async_trait::async_trait]
impl ApiTransport for Fixture {
    async fn execute(&self, request: TransportRequest) -> Result<TransportResponse> {
        assert_eq!(
            request.max_response_bytes,
            Some(MAX_SONG_COMMENTS_RESPONSE_BYTES)
        );
        let HttpBody::Json(payload) = request.body else {
            panic!("expected JSON CGI request");
        };
        assert_eq!(
            payload["comm"]["uin"], 0,
            "public read inherited login state"
        );
        self.calls.lock().unwrap().push(payload);
        Ok(TransportResponse {
            status: 200,
            final_url: request.url,
            headers: Vec::new(),
            body: self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected prefetch"),
        })
    }
}

fn response(rows: usize, has_more: bool, prefix: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({"code":0,"req_0":{"code":0,"data":{
        "SubCode":0,"TotalCmNum":1_000_000,"CommentList":{
            "Comments":(0..rows).map(|i|json!({
                "CmId":format!("{prefix}-{i}"),"SeqNo":format!("{prefix}-seq-{i}"),
                "Content":"synthetic comment","Nick":"Listener"
            })).collect::<Vec<_>>(),
            "HasMore":i32::from(has_more),"Total":1_000_000
        }
    }}}))
    .unwrap()
}

fn client(responses: Vec<Vec<u8>>) -> (Client, Arc<Fixture>) {
    let fixture = Arc::new(Fixture {
        responses: Mutex::new(responses.into()),
        ..Default::default()
    });
    let credential = Credential {
        musicid: 123,
        musickey: "synthetic-session".into(),
        ..Default::default()
    };
    (
        Client::new_with_transport(Some(credential), Some(Platform::Web), fixture.clone()),
        fixture,
    )
}

fn request() -> SongCommentsRequest {
    SongCommentsRequest {
        song_id: 97773,
        sort: SongCommentSort::Hot,
        page: 1,
        limit: 20,
        last_seq: String::new(),
        parent_id: None,
    }
}

#[tokio::test]
async fn opaque_cursor_round_trip_preserves_extra_rows_and_final_sequence() {
    let (client, fixture) = client(vec![
        response(21, true, "first"),
        response(20, false, "second"),
    ]);
    let query = SongCommentsRequest::new(97773, SongCommentSort::Hot, 20, None)
        .with_cursor(None)
        .unwrap();
    let first = client.comment.get_song_comments(&query).await.unwrap();
    let cursor = first.next_cursor(&query).unwrap().unwrap();
    let resumed = SongCommentsRequest::new(97773, SongCommentSort::Hot, 20, None)
        .with_cursor(Some(&cursor))
        .unwrap();
    assert_eq!(resumed.page, 2);
    assert_eq!(resumed.last_seq, "first-seq-20");
    let second = client.comment.get_song_comments(&resumed).await.unwrap();
    assert_eq!(first.comments.len() + second.comments.len(), 41);
    assert!(second.next_cursor(&resumed).unwrap().is_none());
    assert_eq!(
        fixture.calls.lock().unwrap()[1]["req_0"]["param"]["LastCommentSeqNo"],
        "first-seq-20"
    );
}

#[test]
fn opaque_cursors_reject_cross_scope_state_and_accept_legacy_encoding() {
    let legacy = json!({"version":1,"song":97773,"sort":"hot","parent":null,
        "limit":20,"page":100_001,"seq":"seq-20"});
    let query = request().with_cursor(Some(&legacy.to_string())).unwrap();
    assert_eq!(query.page, 100_002);
    assert_eq!(query.last_seq, "seq-20");
    for (key, replacement) in [
        ("version", json!(2)),
        ("song", json!(1)),
        ("sort", json!("new")),
        ("parent", json!("other-thread")),
        ("limit", json!(10)),
        ("page", json!(u32::MAX)),
        ("seq", json!("x".repeat(257))),
        ("unknown", json!(true)),
    ] {
        let mut invalid = legacy.clone();
        invalid[key] = replacement;
        assert!(
            request().with_cursor(Some(&invalid.to_string())).is_err(),
            "{key}"
        );
    }
    for invalid in [String::new(), "garbage".into(), "x".repeat(1025)] {
        assert!(request().with_cursor(Some(&invalid)).is_err());
    }
}

#[test]
fn opaque_cursor_and_lazy_pager_share_termination_rules() {
    for (page_number, rows, initial_seq) in [(1, 0, ""), (1, 1, "same-seq-0"), (u32::MAX, 1, "")] {
        let envelope: Value = serde_json::from_slice(&response(rows, true, "same")).unwrap();
        let page = SongCommentsPage::parse(envelope["req_0"]["data"].clone()).unwrap();
        let mut query = request();
        query.page = page_number;
        query.last_seq = initial_seq.into();
        assert!(page.next_cursor(&query).unwrap().is_none());
    }
}

#[tokio::test]
async fn extra_rows_are_preserved_and_next_page_uses_the_final_returned_sequence() {
    let (client, fixture) = client(vec![
        response(21, true, "first"),
        response(20, false, "second"),
    ]);
    let mut pages = client
        .comment
        .song_comments_pages(request(), CancellationToken::new());
    assert!(
        fixture.calls.lock().unwrap().is_empty(),
        "pager fetched eagerly"
    );
    let first = pages.next().await.unwrap().unwrap();
    assert_eq!(first.comments.len(), 21);
    assert_eq!(first.comments[20].cmid, "first-20");
    assert_eq!(first.total_comments, Some(1_000_000));
    assert_eq!(fixture.calls.lock().unwrap().len(), 1, "pager prefetched");
    let second = pages.next().await.unwrap().unwrap();
    assert_eq!(second.comments.len(), 20);
    assert!(second
        .comments
        .iter()
        .all(|row| first.comments.iter().all(|old| old.cmid != row.cmid)));
    assert!(pages.next().await.is_none());
    let calls = fixture.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0]["req_0"]["method"], "GetHotCommentList");
    assert_eq!(calls[0]["req_0"]["param"]["PageSize"], 20);
    assert_eq!(calls[1]["req_0"]["param"]["PageNum"], 1);
    assert_eq!(
        calls[1]["req_0"]["param"]["LastCommentSeqNo"],
        "first-seq-20"
    );
}

#[tokio::test]
async fn new_and_reply_requests_preserve_their_scope_and_extra_rows() {
    let (client, fixture) = client(vec![
        response(21, false, "new"),
        response(21, false, "reply"),
    ]);
    let mut request = request();
    request.sort = SongCommentSort::New;
    assert_eq!(
        client
            .comment
            .get_song_comments(&request)
            .await
            .unwrap()
            .comments
            .len(),
        21
    );
    request.parent_id = Some("parent-comment".into());
    assert_eq!(
        client
            .comment
            .get_song_comments(&request)
            .await
            .unwrap()
            .comments
            .len(),
        21
    );
    let calls = fixture.calls.lock().unwrap();
    assert_eq!(calls[0]["req_0"]["method"], "GetNewCommentList");
    assert_eq!(calls[1]["req_0"]["method"], "GetReplyCommentList");
    assert_eq!(calls[1]["req_0"]["param"]["RootCmId"], "parent-comment");
}

#[tokio::test]
async fn pager_stops_on_empty_or_nonadvancing_pages_and_page_number_overflow() {
    for (page_number, rows, initial_seq) in [(1, 0, ""), (1, 1, "same-seq-0"), (u32::MAX, 1, "")] {
        let (client, fixture) = client(vec![response(rows, true, "same")]);
        let mut request = request();
        request.page = page_number;
        request.last_seq = initial_seq.into();
        let mut pages = client
            .comment
            .song_comments_pages(request, CancellationToken::new());
        pages.next().await.unwrap().unwrap();
        assert!(pages.next().await.is_none());
        assert_eq!(fixture.calls.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn paging_has_no_arbitrary_total_page_cap() {
    let (client, fixture) = client(vec![response(1, true, "a"), response(1, false, "b")]);
    let mut request = request();
    request.page = 100_002;
    let mut pages = client
        .comment
        .song_comments_pages(request, CancellationToken::new());
    pages.next().await.unwrap().unwrap();
    pages.next().await.unwrap().unwrap();
    assert!(pages.next().await.is_none());
    let calls = fixture.calls.lock().unwrap();
    assert_eq!(calls[0]["req_0"]["param"]["PageNum"], 100_001);
    assert_eq!(calls[1]["req_0"]["param"]["PageNum"], 100_002);
}

#[tokio::test]
async fn response_bytes_are_bounded_even_if_an_injected_transport_ignores_the_limit() {
    let (client, fixture) = client(vec![vec![b' '; MAX_SONG_COMMENTS_RESPONSE_BYTES + 1]]);
    assert!(matches!(
        client.comment.get_song_comments(&request()).await,
        Err(QmError::Protocol {
            stage: "response-limit",
            ..
        })
    ));
    assert_eq!(fixture.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn cancelled_pager_never_starts_another_request() {
    let (client, fixture) = client(vec![response(21, true, "first")]);
    let cancellation = CancellationToken::new();
    let mut pages = client
        .comment
        .song_comments_pages(request(), cancellation.clone());
    pages.next().await.unwrap().unwrap();
    cancellation.cancel();
    assert!(matches!(pages.next().await.unwrap(),
        Err(QmError::Network(error)) if error.kind == crate::NetworkErrorKind::Cancelled));
    assert!(pages.next().await.is_none());
    assert_eq!(fixture.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn streamed_comment_response_stops_at_the_byte_limit() {
    use axum::{body::Body, response::Response, routing::post, Router};
    let app = Router::new().route(
        "/cgi-bin/musicu.fcg",
        post(|| async {
            Response::new(Body::from_stream(futures_util::stream::iter(
                (0..=MAX_SONG_COMMENTS_RESPONSE_BYTES / 16_384)
                    .map(|_| Ok::<_, std::convert::Infallible>(vec![b' '; 16_384])),
            )))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut context = crate::context::ApiContext::new(None, Some(Platform::Web)).unwrap();
    context.cgi_base_url = format!("http://{address}/cgi-bin");
    let api = CommentApi::new(Arc::new(context));
    assert!(matches!(
        api.get_song_comments(&request()).await,
        Err(QmError::Protocol {
            stage: "response-limit",
            ..
        })
    ));
    server.abort();
}

#[tokio::test]
async fn cancellation_interrupts_an_incomplete_streamed_page() {
    use axum::{body::Body, response::Response, routing::post, Router};
    let started = Arc::new(tokio::sync::Notify::new());
    let signal = started.clone();
    let app = Router::new().route(
        "/cgi-bin/musicu.fcg",
        post(move || {
            let signal = signal.clone();
            async move {
                signal.notify_one();
                let chunks = futures_util::StreamExt::chain(
                    futures_util::stream::once(async {
                        Ok::<_, std::convert::Infallible>("{\"code\":0,")
                    }),
                    futures_util::stream::pending(),
                );
                Response::new(Body::from_stream(chunks))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut context = crate::context::ApiContext::new(None, Some(Platform::Web)).unwrap();
    context.cgi_base_url = format!("http://{address}/cgi-bin");
    let api = CommentApi::new(Arc::new(context));
    let cancellation = CancellationToken::new();
    let stop = cancellation.clone();
    let read = tokio::spawn(async move {
        api.get_song_comments_with_cancellation(&request(), cancellation)
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), started.notified())
        .await
        .unwrap();
    stop.cancel();
    let result = tokio::time::timeout(std::time::Duration::from_secs(1), read)
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(result, Err(QmError::Network(error)) if error.kind == crate::NetworkErrorKind::Cancelled)
    );
    server.abort();
}

#[test]
fn extra_rows_do_not_weaken_identity_or_business_error_validation() {
    let valid: Value = serde_json::from_slice(&response(21, true, "row")).unwrap();
    let data = valid["req_0"]["data"].clone();
    assert_eq!(
        SongCommentsPage::parse(data.clone())
            .unwrap()
            .comments
            .len(),
        21
    );
    for replacement in [json!(""), json!("row-0"), json!("x".repeat(257))] {
        let mut invalid = data.clone();
        invalid["CommentList"]["Comments"][1]["CmId"] = replacement;
        assert!(SongCommentsPage::parse(invalid).is_err());
    }
    let mut invalid = data.clone();
    invalid["CommentList"]["HasMore"] = json!(-1);
    assert!(SongCommentsPage::parse(invalid).is_err());
    let mut denied = data;
    denied["SubCode"] = json!(2000);
    assert!(SongCommentsPage::parse(denied).is_err());
}
