//! 下游 crate 不写 `reqwest` 也能使用公开 API.
use std::sync::Arc;

use qqmusic_api::{
    ApiTransport, CancellationToken, Client, HttpMethod, RedirectMode, RetryClass,
    TransportRequest, TransportResponse,
};

#[test]
fn public_types_are_named_without_reqwest() {
    let _m = HttpMethod::Get;
    let _r = RetryClass::Write;
    let _d = RedirectMode::None;
    let _t = CancellationToken::new();
    let req = TransportRequest::new(HttpMethod::Post, "https://u.y.qq.com/cgi-bin/musicu.fcg");
    assert_eq!(req.retry, RetryClass::SafeRead);
    let _ = Client::new(None, None);
    let _status = std::mem::size_of::<TransportResponse>();
    fn accept_transport(_: Arc<dyn ApiTransport>) {}
    let _ = accept_transport as fn(Arc<dyn ApiTransport>);
}
