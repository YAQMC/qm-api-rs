//! QQ artwork URL contracts and bounded, anonymous image downloads.
//! Hosts own caching and presentation; transport implementations must enforce
//! the request's byte limit while collecting decoded response data.

use crate::{CancellationToken, Client, HttpMethod, HttpOptions, QmError, RedirectMode, Result};
use url::Url;

pub const MAX_ARTWORK_BYTES: usize = 5 * 1024 * 1024;

pub struct DownloadedArtwork {
    pub bytes: Vec<u8>,
    pub mime_type: String,
}

/// Does not normalize an untrusted request into a different URL. Callers may
/// use normalize_url for metadata; download accepts only the final HTTPS URL.
pub async fn download(
    client: &Client,
    source: &str,
    cancellation: CancellationToken,
) -> Result<DownloadedArtwork> {
    if cancellation.is_cancelled() {
        return Err(QmError::cancelled());
    }
    if !is_allowed_url(source) {
        return Err(QmError::ValueError("unsupported artwork URL".into()));
    }
    let opts = HttpOptions {
        headers: vec![
            ("Referer".into(), "https://y.qq.com/".into()),
            ("Cookie".into(), String::new()),
        ],
        redirects: RedirectMode::None,
        max_response_bytes: Some(MAX_ARTWORK_BYTES),
        cancellation: cancellation.clone(),
        ..HttpOptions::default()
    };
    let response = client
        .context
        .request_http_raw(HttpMethod::Get, source, &opts)
        .await?;
    if cancellation.is_cancelled() {
        return Err(QmError::cancelled());
    }
    // A custom transport must not silently follow redirects despite the policy.
    if Url::parse(&response.final_url).ok() != Url::parse(source).ok() {
        return Err(QmError::Protocol {
            stage: "artwork",
            message: "unexpected image redirect".into(),
        });
    }
    if response.status != 200 {
        return Err(QmError::http(
            response.status,
            "artwork download failed".into(),
        ));
    }
    if response.body.len() > MAX_ARTWORK_BYTES {
        return Err(QmError::Protocol {
            stage: "response-limit",
            message: "image exceeds byte limit".into(),
        });
    }
    let mut mime_type = None;
    for (_, value) in response
        .headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("content-type"))
    {
        let mime = value
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        let subtype = mime.strip_prefix("image/").filter(|subtype| {
            !subtype.is_empty()
                && subtype.bytes().all(|b| {
                    b.is_ascii_alphanumeric()
                        || matches!(
                            b,
                            b'!' | b'#' | b'$' | b'&' | b'^' | b'_' | b'.' | b'+' | b'-'
                        )
                })
        });
        if subtype.is_none() || mime_type.as_ref().is_some_and(|prior| prior != &mime) {
            return Err(QmError::ApiData("invalid image content type".into()));
        }
        mime_type = Some(mime);
    }
    Ok(DownloadedArtwork {
        bytes: response.body,
        mime_type: mime_type
            .ok_or_else(|| QmError::ApiData("missing image content type".into()))?,
    })
}

const ALBUM_SIZES: [u32; 4] = [150, 300, 500, 800];
const CDN_HOSTS: &[&str] = &[
    "y.gtimg.cn",
    "qpic.y.qq.com",
    "music-file.y.qq.com",
    "q.qlogo.cn",
    "thirdwx.qlogo.cn",
    "thirdqq.qlogo.cn",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtworkVariant {
    pub url: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtworkSource {
    pub url: String,
    pub variants: Vec<ArtworkVariant>,
}

fn safe_mid(mid: &str) -> bool {
    !mid.is_empty()
        && mid != "unknown"
        && mid.len() <= 64
        && mid.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

/// Only established album sizes are advertised; an unknown MID has no source.
pub fn album(mid: &str) -> Option<ArtworkSource> {
    let mid = mid.trim();
    if !safe_mid(mid) {
        return None;
    }
    let variants = ALBUM_SIZES.into_iter().map(|size| ArtworkVariant {
        url: format!("https://y.gtimg.cn/music/photo_new/T002R{size}x{size}M000{mid}.jpg?max_age=2592000"),
        width: size, height: size,
    }).collect::<Vec<_>>();
    Some(ArtworkSource {
        url: variants[1].url.clone(),
        variants,
    })
}

pub fn is_allowed_url(value: &str) -> bool {
    let Ok(url) = Url::parse(value) else {
        return false;
    };
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && url.host_str().is_some_and(|host| {
            CDN_HOSTS.contains(&host)
                || (host == "y.qq.com"
                    && ["/m/resource/calendar/", "/music/common/upload/"]
                        .iter()
                        .any(|prefix| url.path().starts_with(prefix)))
        })
}

/// Upgrade known HTTP CDN URLs without treating arbitrary upstream URLs as trusted.
pub fn normalize_url(value: &str) -> Option<String> {
    let value = value.trim();
    let upgraded = if value.starts_with("//") {
        format!("https:{value}")
    } else if let Some(rest) = value.strip_prefix("http://") {
        format!("https://{rest}")
    } else {
        value.to_owned()
    };
    is_allowed_url(&upgraded).then_some(upgraded)
}

/// Decode only the canonical photo path. Matching a filename on another CDN
/// must not cause a playlist image to be replaced with an unrelated album.
fn photo_parts(url: &Url) -> Option<(&str, u32, u32, &str)> {
    if url.host_str() != Some("y.gtimg.cn") {
        return None;
    }
    let file = url.path().strip_prefix("/music/photo_new/")?;
    let (kind, rest) = file.split_once('R')?;
    if !matches!(kind, "T001" | "T002" | "T003") {
        return None;
    }
    let (dimensions, mid) = rest.split_once("M000")?;
    let (width, height) = dimensions.split_once('x')?;
    let width = width.parse::<u32>().ok()?;
    let height = height.parse::<u32>().ok()?;
    if width == 0 || height == 0 {
        return None;
    }
    let mid = mid.strip_suffix(".jpg")?;
    let canonical = mid
        .rsplit_once('_')
        .filter(|(_, suffix)| {
            !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
        })
        .map_or(mid, |(mid, _)| mid);
    safe_mid(canonical).then_some((kind, width, height, canonical))
}

pub fn from_url(source: &str) -> Option<ArtworkSource> {
    let normalized = normalize_url(source)?;
    let url = Url::parse(&normalized).ok()?;
    let mut variants = Vec::new();
    if let Some((kind, width, height, mid)) = photo_parts(&url) {
        if kind == "T002" {
            return album(mid);
        }
        variants.push(ArtworkVariant {
            url: normalized.clone(),
            width,
            height,
        });
    }
    Some(ArtworkSource {
        url: normalized,
        variants,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn album_contract_keeps_default_and_verified_sizes() {
        let artwork = album(" ALBUM123 ").unwrap();
        assert_eq!(
            artwork.url,
            "https://y.gtimg.cn/music/photo_new/T002R300x300M000ALBUM123.jpg?max_age=2592000"
        );
        assert_eq!(
            artwork
                .variants
                .iter()
                .map(|v| (v.width, v.height))
                .collect::<Vec<_>>(),
            vec![(150, 150), (300, 300), (500, 500), (800, 800)]
        );
        for mid in [
            "",
            "unknown",
            "../bad",
            "mid?token=x",
            "a/b",
            "mid#fragment",
            "你好",
        ] {
            assert!(album(mid).is_none());
        }
        assert!(album(&"a".repeat(65)).is_none());
    }

    #[test]
    fn provider_urls_normalize_only_recognized_album_images() {
        let canonical = album("ALBUM123").unwrap();
        assert_eq!(
            from_url("http://y.gtimg.cn/music/photo_new/T002R300x300M000ALBUM123_1.jpg"),
            Some(canonical)
        );
        let chart = from_url("//y.gtimg.cn/music/photo_new/T003R500x500M000CHART123.jpg").unwrap();
        assert_eq!(chart.variants.len(), 1);
        assert_eq!(chart.variants[0].width, 500);
        for source in [
            "https://qpic.y.qq.com/T002R300x300M000ALBUM123.jpg",
            "https://y.gtimg.cn/other/T002R300x300M000ALBUM123.jpg",
            "https://y.gtimg.cn/music/photo_new/T002RnonsenseM000ALBUM123.jpg",
            "https://music-file.y.qq.com/songlist/cover?imageView2/4/w/600/h/600",
            "https://y.qq.com/m/resource/calendar/0901_300.jpg",
        ] {
            let result = from_url(source).unwrap();
            assert_eq!(result.url, source);
            assert!(
                result.variants.is_empty(),
                "do not invent sizes or substitute the image"
            );
        }
    }

    #[test]
    fn artwork_allowlist_rejects_scheme_authority_and_path_confusion() {
        for source in [
            "https://evil.example/a.jpg",
            "http://y.gtimg.cn/a.jpg",
            "https://y.gtimg.cn.evil.example/a.jpg",
            "https://user:pass@y.gtimg.cn/a.jpg",
            "https://y.gtimg.cn:444/a.jpg",
            "https://sub.y.gtimg.cn/a.jpg",
            "https://y.qq.com/portal/player.html",
            "file:///cover.jpg",
            "data:image/png;base64,eA==",
            "https://y.qq.com/m/resource/calendar/../../../portal/player.html",
        ] {
            assert!(!is_allowed_url(source), "{source}");
        }
        assert_eq!(
            normalize_url(" http://qpic.y.qq.com/cover.jpg "),
            Some("https://qpic.y.qq.com/cover.jpg".into())
        );
        assert!(normalize_url("http://evil.example/cover.jpg").is_none());
        assert!(is_allowed_url("https://y.gtimg.cn:443/a.jpg"));
    }
}
