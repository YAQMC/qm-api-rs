use qqmusic_api::account::{classify_write_reply, AccountWrite, WriteOutcome};
use qqmusic_api::models::comment::{SongCommentSort, SongCommentsRequest};
use qqmusic_api::{
    ApiTransport, CancellationToken, Client, Credential, HttpBody, Platform, QmError,
    TransportRequest, TransportResponse,
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

struct Fixture {
    data: Value,
    code: i64,
    calls: Mutex<Vec<Value>>,
    cookies: Mutex<Vec<String>>,
    cancel_after_send: bool,
}
impl Fixture {
    fn new(data: Value) -> Self {
        Self {
            data,
            code: 0,
            calls: Mutex::new(vec![]),
            cookies: Mutex::new(vec![]),
            cancel_after_send: false,
        }
    }
}
#[async_trait::async_trait]
impl ApiTransport for Fixture {
    async fn execute(&self, request: TransportRequest) -> qqmusic_api::Result<TransportResponse> {
        let HttpBody::Json(payload) = request.body else {
            panic!("CGI JSON expected")
        };
        self.calls.lock().unwrap().push(payload);
        self.cookies.lock().unwrap().push(
            request
                .headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("cookie"))
                .map(|(_, v)| v.clone())
                .unwrap_or_default(),
        );
        if self.cancel_after_send {
            request.cancellation.cancel();
        }
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
fn credential() -> Credential {
    Credential {
        musicid: 1000000001,
        str_musicid: "1000000001".into(),
        musickey: "SYNTHETIC_KEY".into(),
        ..Default::default()
    }
}
fn new_client(fixture: Arc<Fixture>) -> Client {
    Client::new_with_transport(Some(credential()), Some(Platform::Web), fixture)
}
fn comments() -> SongCommentsRequest {
    SongCommentsRequest {
        song_id: 123,
        sort: SongCommentSort::Hot,
        page: 1,
        limit: 20,
        last_seq: String::new(),
        parent_id: None,
    }
}
fn page() -> Value {
    json!({"SubCode":0,"TotalCmNum":100,"CommentList":{"Comments":[{"CmId":"comment-1","SeqNo":"opaque-seq","Content":"Text","PubTime":1700000000,"RepliedComments":[{"Nick":"Other","Content":"Quoted text"}]}],"HasMore":1,"Total":40}})
}

#[tokio::test]
async fn comments_own_hot_new_reply_wire_contract_and_anonymous_scope() {
    let transport = Arc::new(Fixture::new(page()));
    let client = new_client(transport.clone());
    let mut request = comments();
    let response = client.comment.get_song_comments(&request).await.unwrap();
    assert_eq!(response.total_comments, Some(100));
    assert_eq!(response.comments[0].timestamp_ms(), Some(1_700_000_000_000));
    assert!(response.has_more);
    assert_eq!(
        response.comments[0].replied_comments[0].content,
        "Quoted text"
    );
    request.sort = SongCommentSort::New;
    request.page = 2;
    request.last_seq = "opaque-seq".into();
    client.comment.get_song_comments(&request).await.unwrap();
    request.parent_id = Some("parent-1".into());
    client.comment.get_song_comments(&request).await.unwrap();
    let calls = transport.calls.lock().unwrap();
    assert_eq!(
        calls[0]["req_0"]["module"],
        "music.globalComment.CommentRead"
    );
    assert_eq!(calls[0]["req_0"]["method"], "GetHotCommentList");
    assert_eq!(calls[0]["req_0"]["param"]["BizType"], 1);
    assert_eq!(calls[0]["req_0"]["param"]["BizSubType"], 2);
    assert_eq!(calls[1]["req_0"]["method"], "GetNewCommentList");
    assert_eq!(calls[1]["req_0"]["param"]["PageNum"], 1);
    assert_eq!(calls[1]["req_0"]["param"]["LastCommentSeqNo"], "opaque-seq");
    assert_eq!(
        calls[2]["req_0"]["module"],
        "music.globalComment.CommentReadServer"
    );
    assert_eq!(calls[2]["req_0"]["param"]["RootCmId"], "parent-1");
    assert!(calls
        .iter()
        .all(|call| !call.to_string().contains("SYNTHETIC_KEY")));
    assert!(transport
        .cookies
        .lock()
        .unwrap()
        .iter()
        .all(String::is_empty));
}

#[tokio::test]
async fn comments_reject_bad_requests_shapes_business_errors_and_duplicate_ids() {
    let transport = Arc::new(Fixture::new(page()));
    let client = new_client(transport.clone());
    for request in [
        SongCommentsRequest {
            page: 0,
            ..comments()
        },
        SongCommentsRequest {
            limit: 51,
            ..comments()
        },
        SongCommentsRequest {
            song_id: 0,
            ..comments()
        },
        SongCommentsRequest {
            parent_id: Some(" ".into()),
            ..comments()
        },
    ] {
        assert!(client.comment.get_song_comments(&request).await.is_err());
    }
    assert!(transport.calls.lock().unwrap().is_empty());
    for data in [
        json!({}),
        json!({"CommentList":{"Comments":null,"HasMore":0}}),
        json!({"CommentList":{"Comments":[{"Content":"missing identity"}],"HasMore":0}}),
    ] {
        assert!(new_client(Arc::new(Fixture::new(data)))
            .comment
            .get_song_comments(&comments())
            .await
            .is_err());
    }
    let mut duplicate = page();
    let row = duplicate["CommentList"]["Comments"][0].clone();
    duplicate["CommentList"]["Comments"]
        .as_array_mut()
        .unwrap()
        .push(row);
    assert!(new_client(Arc::new(Fixture::new(duplicate)))
        .comment
        .get_song_comments(&comments())
        .await
        .is_err());
    let mut rejected = page();
    rejected["SubCode"] = json!(2001);
    assert!(matches!(
        new_client(Arc::new(Fixture::new(rejected)))
            .comment
            .get_song_comments(&comments())
            .await,
        Err(QmError::RateLimited)
    ));
}

#[tokio::test]
async fn profile_request_is_account_scoped_and_alias_decoding_lives_in_library() {
    let transport = Arc::new(Fixture::new(
        json!({"map_userinfo":{"1000000001":{"NickName":"Listener","headUrl":"http://thirdqq.qlogo.cn/avatar","EncryptUin":"encrypted"}}}),
    ));
    let response = new_client(transport.clone())
        .user
        .get_current_profile(&credential(), CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(response.nickname, "Listener");
    assert_eq!(
        response.avatar_url.as_deref(),
        Some("https://thirdqq.qlogo.cn/avatar")
    );
    assert_eq!(response.encrypted_uin.as_deref(), Some("encrypted"));
    let calls = transport.calls.lock().unwrap();
    assert_eq!(calls[0]["req_0"]["module"], "userInfo.BaseUserInfoServer");
    assert_eq!(
        calls[0]["req_0"]["param"],
        json!({"vec_uin":["1000000001"],"need_profile":1})
    );
}

#[tokio::test]
async fn profile_does_not_substitute_another_account_and_cancellation_remains_distinct() {
    for data in [
        json!({"map_userinfo":{"other":{"nick":"Wrong account"}}}),
        json!({"info":{"nick":" "}}),
        Value::Null,
    ] {
        assert!(new_client(Arc::new(Fixture::new(data)))
            .user
            .get_current_profile(&credential(), CancellationToken::new())
            .await
            .is_err());
    }
    for code in [1000, 104400, 104401, 2001, 104604, 10007] {
        let transport = Arc::new(Fixture {
            code,
            ..Fixture::new(json!({}))
        });
        let error = new_client(transport)
            .user
            .get_current_profile(&credential(), CancellationToken::new())
            .await
            .unwrap_err();
        match code {
            1000 | 104400 | 104401 => assert!(matches!(error, QmError::CredentialExpired(_))),
            2001 | 104604 => assert!(matches!(error, QmError::RateLimited)),
            _ => assert!(matches!(error, QmError::CgiApi { .. })),
        };
    }
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let transport = Arc::new(Fixture::new(json!({})));
    assert!(new_client(transport.clone())
        .user
        .get_current_profile(&credential(), cancellation)
        .await
        .is_err());
    assert!(transport.calls.lock().unwrap().is_empty());
    let transport = Arc::new(Fixture {
        cancel_after_send: true,
        ..Fixture::new(json!({"info":{"nick":"Listener"}}))
    });
    assert!(
        matches!(new_client(transport).user.get_current_profile(&credential(),CancellationToken::new()).await,Err(QmError::Network(error)) if error.kind==qqmusic_api::NetworkErrorKind::Cancelled)
    );
}

#[test]
fn write_acknowledgements_keep_rejection_and_uncertainty_separate() {
    let operation = AccountWrite::FavoriteSong {
        add: true,
        song_id: 123,
        song_type: 0,
    };
    let classify =
        |code, data| classify_write_reply(&operation, &qqmusic_api::CgiReply::new(code, data));
    for data in [
        json!({"retCode":0}),
        json!({"result":{"updateTime":1}}),
        json!({"updateTime":"1800000000"}),
    ] {
        assert_eq!(classify(0, data).unwrap(), WriteOutcome::Accepted);
    }
    for data in [
        json!({"retCode":1}),
        json!({"result":0,"v_failedPlaylistId":[88]}),
        json!({"retCode":0,"v_failTids":[42]}),
    ] {
        assert_eq!(classify(0, data).unwrap(), WriteOutcome::Rejected);
    }
    for data in [
        json!({"result":{}}),
        json!({"updateTime":0}),
        json!({"updateTime":null}),
        json!({"updateTime":""}),
    ] {
        assert_eq!(classify(0, data).unwrap(), WriteOutcome::Unknown);
    }
    assert_eq!(classify(80092, json!({})).unwrap(), WriteOutcome::Rejected);
    assert_eq!(
        classify(80105, json!({"retCode":0})).unwrap(),
        WriteOutcome::Unknown
    );
    let playlist = AccountWrite::CreatePlaylist {
        name: "Name".into(),
    };
    assert!(
        classify_write_reply(&playlist, &qqmusic_api::CgiReply::new(80092, json!({}))).is_err()
    );
}

#[test]
fn lyric_payloads_decode_text_but_do_not_expose_opaque_bytes() {
    use base64::Engine as _;
    use qqmusic_api::models::lyric::decode_lyric_text;
    let base64 = base64::engine::general_purpose::STANDARD;
    assert_eq!(
        decode_lyric_text(&base64.encode("[00:01.00]Text")).as_deref(),
        Some("[00:01.00]Text")
    );
    assert_eq!(
        decode_lyric_text("Plain text").as_deref(),
        Some("Plain text")
    );
    assert!(decode_lyric_text(&base64.encode([0u8; 48])).is_none());
    assert!(decode_lyric_text(&"deadbeef".repeat(20)).is_none());
}

#[test]
fn playback_lyric_syntax_preserves_absolute_words_xml_and_hour_timestamps() {
    use qqmusic_api::lyric_parser::{
        parse_playback_lrc, parse_playback_qrc, parse_timed_lrc, LyricFormat,
    };
    let qrc = parse_playback_qrc("[0,2000]Fixture(0,900) lyric(900,1100)").unwrap();
    assert_eq!(qrc.format, LyricFormat::Qrc);
    assert_eq!(qrc.lines[0].words[1].start_ms, 900);
    assert_eq!(qrc.lines[0].words[1].end_ms, 2000);
    let wrapped = parse_playback_qrc(
        r#"<QrcInfos><Lyric_1 LyricContent="[0,1000]你(0,500)好(500,500)"/></QrcInfos>"#,
    )
    .unwrap();
    assert_eq!(wrapped.lines[0].text, "你好");
    assert_eq!(wrapped.lines[0].words.len(), 2);
    let lrc = parse_playback_lrc("[offset:-25]\n[01:02:03.123]A\n[01:02:04.123]B").unwrap();
    assert_eq!(lrc.offset_ms, -25);
    assert_eq!(lrc.lines[0].start_ms, Some(3_723_123));
    assert_eq!(lrc.lines[0].end_ms, Some(3_724_123));
    assert_eq!(parse_timed_lrc("[00:01.00][00:02.00]A").len(), 2);
    assert!(parse_playback_qrc("not QRC").is_none());
    assert_eq!(
        parse_playback_lrc("Plain &amp; text").unwrap().lines[0].text,
        "Plain & text"
    );
}

#[test]
fn playback_qrc_xml_preserves_literal_and_escaped_line_breaks() {
    use qqmusic_api::lyric_parser::parse_playback_qrc;
    for separator in ["\n", "&#10;"] {
        let xml = format!(
            "<QrcInfos><Lyric_1 LyricContent=\"[0,1000]A(0,1000){separator}[1000,1000]B(1000,1000)\"/></QrcInfos>"
        );
        let parsed = parse_playback_qrc(&xml).unwrap();
        assert_eq!(parsed.lines.len(), 2);
        assert_eq!(parsed.lines[0].text, "A");
        assert_eq!(parsed.lines[1].text, "B");
        assert_eq!(parsed.lines[1].start_ms, Some(1000));
    }
}

#[test]
fn playback_lrc_accepts_accumulated_minutes_and_checks_hour_fields() {
    use qqmusic_api::lyric_parser::{ms_to_lrc_time, parse_playback_lrc, parse_timed_lrc};
    let raw = format!("[59:59.00]before\n[{}]after", ms_to_lrc_time(3_600_000));
    let parsed = parse_playback_lrc(&raw).unwrap();
    assert_eq!(parsed.lines.len(), 2);
    assert_eq!(parsed.lines[1].start_ms, Some(3_600_000));
    assert_eq!(parsed.lines[1].text, "after");
    assert_eq!(parse_timed_lrc("[123:45.67]long")[0].0, 7_425_670);
    assert_eq!(parse_timed_lrc("[01:02:03.123]hour")[0].0, 3_723_123);
    for invalid in ["[01:60:00.00]bad", "[60:60.00]bad", "[-1:00.00]bad"] {
        assert!(parse_timed_lrc(invalid).is_empty());
    }
}

#[tokio::test]
async fn media_source_requires_encrypted_keys_before_downloading() {
    use qqmusic_api::media::{download_quality, MediaSource};
    use qqmusic_api::modules::song::SongQuality;
    let song = qqmusic_api::Song {
        mid: "track".into(),
        file: qqmusic_api::models::File {
            media_mid: "media".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    for key in ["", "   "] {
        let fixture = Arc::new(Fixture::new(json!({
            "midurlinfo":[{"filename":"F0M0media.mflac", "songmid":"track", "result":0,
                "purl":"F0M0media.mflac?vkey=SYNTHETIC", "ekey":key}]
        })));
        let client = new_client(fixture.clone());
        let source = MediaSource::resolve(&client.song, &song, SongQuality::Flac, None, true)
            .await
            .unwrap();
        assert!(source.encrypted);
        assert!(!source.playable());
        let error = download_quality(&client.song, &song, SongQuality::Flac, None)
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            QmError::Protocol {
                stage: "media-key",
                ..
            }
        ));
        // The transport accepts only CGI JSON; reaching the audio download would panic.
        assert_eq!(fixture.calls.lock().unwrap().len(), 2);
    }
}

#[tokio::test]
async fn media_source_joins_cdn_urls_and_rejects_untrusted_origins() {
    use qqmusic_api::media::MediaSource;
    use qqmusic_api::modules::song::SongQuality;
    let song = qqmusic_api::Song {
        mid: "track".into(),
        file: qqmusic_api::models::File {
            media_mid: "media".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    for (purl, expected) in [
        (
            "/a.mflac",
            Some("https://isure.stream.qqmusic.qq.com/a.mflac"),
        ),
        (
            "https://aqqmusic.tc.qq.com/a.mflac",
            Some("https://aqqmusic.tc.qq.com/a.mflac"),
        ),
        ("https://example.invalid/a.mflac", None),
        ("http://isure.stream.qqmusic.qq.com/a.mflac", None),
        ("https://user@isure.stream.qqmusic.qq.com/a.mflac", None),
    ] {
        let fixture = Arc::new(Fixture::new(json!({
            "midurlinfo":[{"filename":"F0M0media.mflac", "songmid":"track", "result":0,
                "purl":purl, "ekey":"SYNTHETIC_KEY"}]
        })));
        let client = new_client(fixture);
        let result = MediaSource::resolve(&client.song, &song, SongQuality::Flac, None, true).await;
        if let Some(expected) = expected {
            let source = result.unwrap();
            assert_eq!(source.url, expected);
            assert!(source.playable());
        } else {
            assert!(result.is_err());
        }
    }
}

#[test]
fn playback_qrc_clips_each_word_to_an_ordered_interval_inside_the_line() {
    use qqmusic_api::lyric_parser::parse_playback_qrc;
    for raw in [
        "[1000,1000]before(0,100)",
        "[1000,1000]overlap(900,500)",
        "[1000,1000]after(3000,100)tail",
        "[1000,1000]overflow(18446744073709551615,100)",
        "[1000,0]zero(0,100)",
    ] {
        let parsed = parse_playback_qrc(raw).unwrap();
        let line = &parsed.lines[0];
        assert!(!line.words.is_empty());
        for word in &line.words {
            assert!(line.start_ms.unwrap() <= word.start_ms, "{raw}");
            assert!(word.start_ms <= word.end_ms, "{raw}");
            assert!(word.end_ms <= line.end_ms.unwrap(), "{raw}");
        }
    }
    let parsed = parse_playback_qrc("[1000,1000]overlap(900,500)").unwrap();
    assert_eq!(parsed.lines[0].words[0].start_ms, 1000);
    assert_eq!(parsed.lines[0].words[0].end_ms, 1400);
}

#[test]
fn account_playlist_aliases_and_edit_field_presence_are_typed() {
    use qqmusic_api::account::{AccountPlaylistMetadata, OwnedPlaylistEditFields};
    let playlist: AccountPlaylistMetadata = serde_json::from_value(
        json!({"disstid":"123","dirid":4,"dissname":"Name","songnum":5,"nickname":"Owner"}),
    )
    .unwrap();
    assert_eq!(playlist.tid, Some(json!("123")));
    assert_eq!(playlist.dir_id, Some(4));
    assert_eq!(playlist.title.as_deref(), Some("Name"));
    assert_eq!(playlist.track_count, 5);
    let missing: OwnedPlaylistEditFields =
        serde_json::from_value(json!({"dirId":4,"dirName":"Name"})).unwrap();
    assert!(missing.description.is_none());
    let empty: OwnedPlaylistEditFields = serde_json::from_value(
        json!({"dirId":4,"dirName":"Name","desc":"","picUrl":"","tagNameList":[]}),
    )
    .unwrap();
    assert_eq!(empty.tag_names, Some(vec![]));
    assert_eq!(empty.description.as_deref(), Some(""));
}

#[test]
fn playback_wire_filenames_denials_and_cdn_policy_are_library_owned() {
    use qqmusic_api::playback::{normalize_cdn_url, sources_from_items, PlaybackFile};
    for (kind, filename) in [
        (PlaybackFile::Master, "AIM0media.mflac"),
        (PlaybackFile::HiRes, "RS01media.flac"),
        (PlaybackFile::EncryptedLossless, "F0M0media.mflac"),
        (PlaybackFile::Preview, "RS02media.mp3"),
    ] {
        assert_eq!(kind.filename("media"), filename);
        assert_eq!(PlaybackFile::from_filename(filename), Some(kind));
    }
    let filenames = vec!["M500media.mp3".into()];
    let denied =
        serde_json::from_value(json!({"filename":"M500media.mp3","purl":"","result":104003}))
            .unwrap();
    assert!(
        sources_from_items(&filenames, vec![denied], false).unwrap()["M500media.mp3"]
            .url
            .is_none()
    );
    let foreign =
        serde_json::from_value(json!({"filename":"other","purl":"file.mp3","result":0})).unwrap();
    assert!(sources_from_items(&filenames, vec![foreign], false).is_err());
    let absolute = "https://aqqmusic.tc.qq.com/path/file.mp3?vkey=SYNTHETIC";
    assert_eq!(
        normalize_cdn_url("https://isure.stream.qqmusic.qq.com/", absolute).unwrap(),
        absolute
    );
    for url in [
        "https://evil.example/file",
        "https://user@isure.stream.qqmusic.qq.com/file",
        "https://isure.stream.qqmusic.qq.com:444/file",
        "http://isure.stream.qqmusic.qq.com/file",
    ] {
        assert!(normalize_cdn_url(url, "file").is_err());
    }
}

#[tokio::test]
async fn playback_batch_validates_requested_files_before_sending() {
    use qqmusic_api::playback::resolve_files;
    let transport = Arc::new(Fixture::new(
        json!({"midurlinfo":[{"filename":"M500media.mp3","purl":"M500media.mp3?vkey=SYNTHETIC","result":0}],"expiration":100}),
    ));
    let client = new_client(transport.clone());
    let result = resolve_files(
        &client.song,
        "track",
        "media",
        &["M500media.mp3".into()],
        None,
        false,
    )
    .await
    .unwrap();
    assert!(result["M500media.mp3"]
        .url
        .as_deref()
        .unwrap()
        .starts_with("https://isure.stream.qqmusic.qq.com/"));
    for files in [
        vec!["M500other.mp3".into()],
        vec!["AIM0media.mflac".into()],
        vec!["M500media.mp3".into(), "M500media.mp3".into()],
    ] {
        assert!(
            resolve_files(&client.song, "track", "media", &files, None, false)
                .await
                .is_err()
        );
    }
    assert_eq!(transport.calls.lock().unwrap().len(), 1);
}
