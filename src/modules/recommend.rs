//! 推荐模块 (对应 Python 端 `modules/recommend.py`).

use serde_json::json;
use serde_json::Value;

use super::ApiModule;
use crate::context::RequestOptions;
use crate::error::{QmError, Result};
use crate::models::recommend::*;
use crate::models::Credential;

const MAX_RECOMMEND_BATCH: u32 = 30;
const MAX_RECOMMEND_SEED_IDS: usize = 100;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
