use qqmusic_api::{
    artwork, ApiTransport, CancellationToken, Client, Credential, HttpBody, HttpMethod,
    NetworkErrorKind, QmError, RedirectMode, TransportRequest, TransportResponse,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

const SOURCE: &str = "https://qpic.y.qq.com/synthetic.png";

struct ImageTransport {
    response: TransportResponse,
    calls: AtomicUsize,
    cancel_on_reply: bool,
}
impl ImageTransport {
    fn new() -> Self {
        Self {
            response: TransportResponse {
                status: 200,
                final_url: SOURCE.into(),
                headers: vec![("Content-Type".into(), "Image/PNG; charset=binary".into())],
                body: vec![0, 1, 2, 3],
            },
            calls: AtomicUsize::new(0),
            cancel_on_reply: false,
        }
    }
}
#[async_trait::async_trait]
impl ApiTransport for ImageTransport {
    async fn execute(&self, request: TransportRequest) -> qqmusic_api::Result<TransportResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(request.url, SOURCE);
        assert_eq!(request.method, HttpMethod::Get);
        assert_eq!(request.redirects, RedirectMode::None);
        assert_eq!(request.max_response_bytes, Some(artwork::MAX_ARTWORK_BYTES));
        assert!(matches!(request.body, HttpBody::Empty));
        assert!(request
            .headers
            .iter()
            .any(|(k, v)| k.eq_ignore_ascii_case("cookie") && v.is_empty()));
        assert!(!request
            .headers
            .iter()
            .any(|(k, v)| k.eq_ignore_ascii_case("authorization")
                || v.contains("SYNTHETIC_SESSION_KEY")));
        if self.cancel_on_reply {
            request.cancellation.cancel();
        }
        Ok(self.response.clone())
    }
}
fn client(transport: Arc<ImageTransport>) -> Client {
    Client::new_with_transport(
        Some(Credential {
            musicid: 10001,
            musickey: "SYNTHETIC_SESSION_KEY".into(),
            ..Credential::default()
        }),
        None,
        transport,
    )
}

#[tokio::test]
async fn artwork_is_anonymous_bounded_and_keeps_binary_data() {
    let transport = Arc::new(ImageTransport::new());
    let downloaded =
        artwork::download(&client(transport.clone()), SOURCE, CancellationToken::new())
            .await
            .unwrap();
    assert_eq!(downloaded.mime_type, "image/png");
    assert_eq!(downloaded.bytes, [0, 1, 2, 3]);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn rejected_urls_and_precancelled_calls_never_reach_transport() {
    let transport = Arc::new(ImageTransport::new());
    let client = client(transport.clone());
    for url in [
        "http://qpic.y.qq.com/image",
        "https://example.invalid/image",
        "https://qpic.y.qq.com:8443/image",
        "https://user:pass@qpic.y.qq.com/image",
        "https://y.qq.com/portal/player.html",
    ] {
        assert!(artwork::download(&client, url, CancellationToken::new())
            .await
            .is_err());
    }
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(
        matches!(artwork::download(&client, SOURCE, cancelled).await,
        Err(QmError::Network(error)) if error.kind == NetworkErrorKind::Cancelled)
    );
    assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn invalid_status_type_size_redirect_and_late_cancellation_do_not_succeed() {
    for headers in [
        vec![],
        vec![("Content-Type".into(), "text/html".into())],
        vec![("Content-Type".into(), "image/".into())],
        vec![
            ("Content-Type".into(), "image/png".into()),
            ("content-type".into(), "text/html".into()),
        ],
    ] {
        let mut transport = ImageTransport::new();
        transport.response.headers = headers;
        assert!(artwork::download(
            &client(Arc::new(transport)),
            SOURCE,
            CancellationToken::new()
        )
        .await
        .is_err());
    }
    for status in [301, 302, 307, 401, 403, 404, 500] {
        let mut transport = ImageTransport::new();
        transport.response.status = status;
        assert!(
            matches!(artwork::download(&client(Arc::new(transport)), SOURCE, CancellationToken::new()).await,
            Err(QmError::Http { status: code, .. }) if code == status)
        );
    }
    let mut changed = ImageTransport::new();
    changed.response.final_url = "https://qpic.y.qq.com/other.png".into();
    assert!(matches!(
        artwork::download(&client(Arc::new(changed)), SOURCE, CancellationToken::new()).await,
        Err(QmError::Protocol {
            stage: "artwork",
            ..
        })
    ));
    let mut large = ImageTransport::new();
    large.response.body = vec![0; artwork::MAX_ARTWORK_BYTES + 1];
    assert!(matches!(
        artwork::download(&client(Arc::new(large)), SOURCE, CancellationToken::new()).await,
        Err(QmError::Protocol {
            stage: "response-limit",
            ..
        })
    ));
    let cancelled = ImageTransport {
        cancel_on_reply: true,
        ..ImageTransport::new()
    };
    assert!(
        matches!(artwork::download(&client(Arc::new(cancelled)), SOURCE, CancellationToken::new()).await,
        Err(QmError::Network(error)) if error.kind == NetworkErrorKind::Cancelled)
    );
}
