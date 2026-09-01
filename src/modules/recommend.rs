//! 推荐模块 (对应 Python 端 `modules/recommend.py`).

use serde_json::json;
use serde_json::Value;

use super::ApiModule;
use crate::client::HttpOptions;
use crate::context::RequestOptions;
use crate::error::{QmError, Result};
use crate::models::recommend::*;
use crate::models::Credential;
use crate::transport::HttpMethod;

const MAX_RECOMMEND_BATCH: u32 = 30;
const MAX_RECOMMEND_SEED_IDS: usize = 100;
const DAILY_RECOMMENDATION_PAGE: &str = "https://c.y.qq.com/node/musicmac/v6/index.html";
const DAILY_RECOMMENDATION_LABEL: &str = "今日私享";
const DAILY_RECOMMENDATION_SCAN_BYTES: usize = 4_096;

/// 推荐 API.
#[derive(Clone, Debug)]
pub struct RecommendApi {
    pub(crate) base: ApiModule,
}

impl RecommendApi {
    pub(crate) fn new(context: std::sync::Arc<crate::context::ApiContext>) -> Self {
        RecommendApi {
            base: ApiModule::new(context),
        }
    }

    /// 获取首页推荐 Feed.
    pub async fn get_home_feed(
        &self,
        page: i64,
        direction: i64,
        s_num: i64,
        v_cache: &[String],
    ) -> Result<RecommendFeedCardResponse> {
        let data = self
            .base
            .cgi(
                "music.recommend.RecommendFeed",
                "get_recommend_feed",
                json!({
                    "direction": direction,
                    "page": page,
                    "s_num": s_num,
                    "v_cache": v_cache,
                }),
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取猜你喜欢推荐.
    pub async fn get_guess_recommend(
        &self,
        credential: Option<&Credential>,
    ) -> Result<GuessRecommendResponse> {
        self.get_guess_recommend_with_request(&GuessRecommendRequest::default(), credential)
            .await
    }

    /// 获取参数化猜你喜欢推荐.
    pub async fn get_guess_recommend_with_request(
        &self,
        request: &GuessRecommendRequest,
        credential: Option<&Credential>,
    ) -> Result<GuessRecommendResponse> {
        validate_guess_request(request)?;
        let mut opts = RequestOptions::default();
        opts.credential = credential.cloned();
        let data = self
            .base
            .cgi(
                "music.radioProxy.MbTrackRadioSvr",
                "get_radio_track",
                json!({
                    "id": 99,
                    "num": request.limit,
                    "from": request.offset,
                    "scene": 0,
                    "song_ids": request.seed_song_ids,
                }),
                opts,
            )
            .await?;
        require_object_array(&data, "tracks", "recommend.guess.tracks")?;
        Ok(serde_json::from_value(data)?)
    }

    /// 解析当前登录账号的“今日私享”歌单入口.
    ///
    /// QQ 音乐为每个账号返回不同的 `data-rid`; 该值不是稳定的公共歌单 ID.
    /// 此方法只访问固定的 QQ 音乐 Mac 首页，并显式携带调用方提供的凭证.
    pub async fn get_daily_recommendation(
        &self,
        credential: Option<&Credential>,
    ) -> Result<DailyRecommendationResponse> {
        let credential = credential.filter(|credential| {
            !credential.str_musicid().trim().is_empty() && !credential.musickey.trim().is_empty()
        });
        let credential = credential.ok_or_else(|| {
            QmError::CredentialInvalid("daily recommendation requires a signed-in account".into())
        })?;
        let opts = HttpOptions {
            headers: vec![
                ("User-Agent".into(), "QQMusic/21".into()),
                ("Referer".into(), "https://y.qq.com/".into()),
            ],
            credential: Some(credential.clone()),
            ..Default::default()
        };
        let html = self
            .base
            .context
            .request_http(HttpMethod::Get, DAILY_RECOMMENDATION_PAGE, &opts)
            .await?;
        Ok(DailyRecommendationResponse {
            songlist_id: parse_daily_recommendation_songlist_id(&html)?,
        })
    }

    /// 获取雷达推荐.
    pub async fn get_radar_recommend(&self, page: i64) -> Result<RadarRecommendResponse> {
        let page = u32::try_from(page).map_err(|_| {
            QmError::ValueError("radar recommendation page must be a positive integer".into())
        })?;
        self.get_radar_recommend_with_request(
            &RadarRecommendRequest {
                page,
                ..Default::default()
            },
            None,
        )
        .await
    }

    /// 获取参数化雷达推荐.
    pub async fn get_radar_recommend_with_request(
        &self,
        request: &RadarRecommendRequest,
        credential: Option<&Credential>,
    ) -> Result<RadarRecommendResponse> {
        validate_radar_request(request)?;
        let mut opts = RequestOptions::default();
        opts.credential = credential.cloned();
        let data = self
            .base
            .cgi(
                "music.recommend.TrackRelationServer",
                "GetRadarSong",
                json!({
                    "Page": request.page,
                    "ReqType": request.request_type,
                    "FavSongs": request.favorite_song_ids,
                    "EntranceSongs": request.entrance_song_ids,
                }),
                opts,
            )
            .await?;
        require_radar_tracks(&data)?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取推荐歌单.
    pub async fn get_recommend_songlist(
        &self,
        page: i64,
        num: i64,
    ) -> Result<RecommendSonglistResponse> {
        let data = self
            .base
            .cgi(
                "music.playlist.PlaylistSquare",
                "GetRecommendFeed",
                json!({ "From": num * (page - 1), "Size": num }),
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取推荐新歌.
    pub async fn get_recommend_newsong(&self, r#type: i64) -> Result<RecommendNewSongResponse> {
        let data = self
            .base
            .cgi(
                "newsong.NewSongServer",
                "get_new_song_info",
                json!({ "type": r#type }),
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }
}

fn validate_guess_request(request: &GuessRecommendRequest) -> Result<()> {
    if !(1..=MAX_RECOMMEND_BATCH).contains(&request.limit) {
        return Err(QmError::ValueError(format!(
            "guess recommendation limit must be between 1 and {MAX_RECOMMEND_BATCH}"
        )));
    }
    validate_seed_count(request.seed_song_ids.len())
}

fn validate_radar_request(request: &RadarRecommendRequest) -> Result<()> {
    if request.page == 0 {
        return Err(QmError::ValueError(
            "radar recommendation page must be a positive integer".into(),
        ));
    }
    validate_seed_count(request.favorite_song_ids.len())?;
    validate_seed_count(request.entrance_song_ids.len())
}

fn validate_seed_count(count: usize) -> Result<()> {
    if count > MAX_RECOMMEND_SEED_IDS {
        return Err(QmError::ValueError(format!(
            "recommendation seed list may contain at most {MAX_RECOMMEND_SEED_IDS} ids"
        )));
    }
    Ok(())
}

fn require_object_array(data: &Value, key: &'static str, stage: &'static str) -> Result<()> {
    let items = data
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| QmError::Protocol {
            stage,
            message: format!("missing or non-array {key}"),
        })?;
    if items.iter().any(|item| !item.is_object()) {
        return Err(QmError::Protocol {
            stage,
            message: format!("{key} contains a non-object item"),
        });
    }
    Ok(())
}

fn require_radar_tracks(data: &Value) -> Result<()> {
    let items = data
        .get("VecSongs")
        .and_then(Value::as_array)
        .ok_or_else(|| QmError::Protocol {
            stage: "recommend.radar.VecSongs",
            message: "missing or non-array VecSongs".into(),
        })?;
    if items
        .iter()
        .any(|item| item.get("Track").is_none_or(|track| !track.is_object()))
    {
        return Err(QmError::Protocol {
            stage: "recommend.radar.VecSongs.Track",
            message: "VecSongs contains an item without an object Track".into(),
        });
    }
    Ok(())
}

fn parse_daily_recommendation_songlist_id(html: &str) -> Result<i64> {
    let bytes = html.as_bytes();
    for (marker_start, _) in html.match_indices(DAILY_RECOMMENDATION_LABEL) {
        let before_start = marker_start.saturating_sub(DAILY_RECOMMENDATION_SCAN_BYTES);
        let card_start = bytes[before_start..marker_start]
            .windows(b"<li".len())
            .rposition(|window| window.eq_ignore_ascii_case(b"<li"))
            .map_or(before_start, |position| before_start + position);
        if let Some(songlist_id) = find_last_data_rid(&bytes[card_start..marker_start]) {
            return Ok(songlist_id);
        }

        let after_start = marker_start + DAILY_RECOMMENDATION_LABEL.len();
        let scan_end = after_start
            .saturating_add(DAILY_RECOMMENDATION_SCAN_BYTES)
            .min(bytes.len());
        let after_end = bytes[after_start..scan_end]
            .windows(b"</li>".len())
            .position(|window| window.eq_ignore_ascii_case(b"</li>"))
            .map_or(scan_end, |position| after_start + position);
        if let Some(songlist_id) = find_first_data_rid(&bytes[after_start..after_end]) {
            return Ok(songlist_id);
        }
    }
    Err(QmError::Protocol {
        stage: "recommend.daily.data-rid",
        message: "daily recommendation card is missing a valid songlist id".into(),
    })
}

fn find_last_data_rid(bytes: &[u8]) -> Option<i64> {
    let needle = b"data-rid";
    let mut end = bytes.len();
    while end >= needle.len() {
        let position = bytes[..end]
            .windows(needle.len())
            .rposition(|window| window.eq_ignore_ascii_case(needle))?;
        if let Some(songlist_id) = parse_data_rid(bytes, position + needle.len()) {
            return Some(songlist_id);
        }
        end = position;
    }
    None
}

fn find_first_data_rid(bytes: &[u8]) -> Option<i64> {
    let needle = b"data-rid";
    let mut start = 0;
    while start + needle.len() <= bytes.len() {
        let relative = bytes[start..]
            .windows(needle.len())
            .position(|window| window.eq_ignore_ascii_case(needle))?;
        let position = start + relative;
        if let Some(songlist_id) = parse_data_rid(bytes, position + needle.len()) {
            return Some(songlist_id);
        }
        start = position + needle.len();
    }
    None
}

fn parse_data_rid(bytes: &[u8], mut cursor: usize) -> Option<i64> {
    while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
        cursor += 1;
    }
    if bytes.get(cursor) != Some(&b'=') {
        return None;
    }
    cursor += 1;
    while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
        cursor += 1;
    }
    if bytes.get(cursor) == Some(&b'\\') && matches!(bytes.get(cursor + 1), Some(b'\'' | b'"')) {
        cursor += 1;
    }
    if matches!(bytes.get(cursor), Some(b'\'' | b'"')) {
        cursor += 1;
    }
    let digits_start = cursor;
    while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
        cursor += 1;
    }
    if cursor == digits_start || cursor - digits_start > 19 {
        return None;
    }
    std::str::from_utf8(&bytes[digits_start..cursor])
        .ok()?
        .parse::<i64>()
        .ok()
        .filter(|songlist_id| *songlist_id > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex};

    use crate::transport::{ApiTransport, TransportRequest, TransportResponse};

    type CapturedRequest = (String, Vec<(String, String)>);

    #[derive(Debug, Default)]
    struct DailyRecommendationTransport {
        requests: Mutex<Vec<CapturedRequest>>,
    }

    #[async_trait]
    impl ApiTransport for DailyRecommendationTransport {
        async fn execute(&self, request: TransportRequest) -> Result<TransportResponse> {
            self.requests
                .lock()
                .expect("request capture")
                .push((request.url.clone(), request.headers.clone()));
            Ok(TransportResponse {
                status: 200,
                final_url: request.url,
                headers: Vec::new(),
                body: r#"<li><a class="playlist__link" data-rid="7654321"><img></a><h4><a>今日私享</a></h4></li>"#
                    .as_bytes()
                    .to_vec(),
            })
        }
    }

    #[test]
    fn recommendation_request_validation_is_bounded() {
        assert!(validate_guess_request(&GuessRecommendRequest::default()).is_ok());
        assert!(validate_guess_request(&GuessRecommendRequest {
            limit: 0,
            ..Default::default()
        })
        .is_err());
        assert!(validate_guess_request(&GuessRecommendRequest {
            limit: MAX_RECOMMEND_BATCH + 1,
            ..Default::default()
        })
        .is_err());
        assert!(validate_radar_request(&RadarRecommendRequest {
            page: 0,
            ..Default::default()
        })
        .is_err());
        assert!(validate_radar_request(&RadarRecommendRequest {
            entrance_song_ids: vec![1; MAX_RECOMMEND_SEED_IDS + 1],
            ..Default::default()
        })
        .is_err());
    }

    #[test]
    fn daily_recommendation_parser_selects_the_account_card() {
        let html = r#"
            <li><a data-rid="111"><img></a><h4>其他歌单</h4></li>
            <li><a class="playlist__link" data-rid='222'><img></a>
                <h4><a>Food rain的今日私享</a></h4></li>
            <li><a data-rid="333"><img></a><h4>下一歌单</h4></li>
        "#;
        assert_eq!(parse_daily_recommendation_songlist_id(html).unwrap(), 222);
    }

    #[test]
    fn daily_recommendation_parser_rejects_missing_or_invalid_ids() {
        let error = parse_daily_recommendation_songlist_id(
            r#"<li><a data-rid="not-a-number"></a><h4>今日私享</h4></li>"#,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            QmError::Protocol {
                stage: "recommend.daily.data-rid",
                ..
            }
        ));
    }

    #[tokio::test]
    async fn daily_recommendation_requires_and_scopes_explicit_credentials() {
        let transport = Arc::new(DailyRecommendationTransport::default());
        let client = crate::Client::new_with_transport(None, None, transport.clone());
        let missing = client
            .recommend
            .get_daily_recommendation(None)
            .await
            .unwrap_err();
        assert!(matches!(missing, QmError::CredentialInvalid(_)));
        assert!(transport.requests.lock().unwrap().is_empty());

        let credential = Credential {
            musicid: 10_001,
            str_musicid: "10001".into(),
            musickey: "synthetic-key".into(),
            ..Default::default()
        };
        let response = client
            .recommend
            .get_daily_recommendation(Some(&credential))
            .await
            .unwrap();
        assert_eq!(response.songlist_id, 7_654_321);

        let requests = transport.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].0, DAILY_RECOMMENDATION_PAGE);
        let cookie = requests[0]
            .1
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("cookie"))
            .map(|(_, value)| value.as_str())
            .expect("credential cookie");
        assert!(cookie.contains("uin=10001"));
        assert!(cookie.contains("qm_keyst=synthetic-key"));
    }
}
