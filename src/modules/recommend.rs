//! 推荐模块 (对应 Python 端 `modules/recommend.py`).

use serde_json::json;

use super::ApiModule;
use crate::context::RequestOptions;
use crate::error::Result;
use crate::models::recommend::*;
use crate::models::Credential;

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
        let mut opts = RequestOptions::default();
        opts.credential = credential.cloned();
        let data = self
            .base
            .cgi(
                "music.radioProxy.MbTrackRadioSvr",
                "get_radio_track",
                json!({
                    "id": 99,
                    "num": 5,
                    "from": 0,
                    "scene": 0,
                    "song_ids": [],
                }),
                opts,
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取雷达推荐.
    pub async fn get_radar_recommend(&self, page: i64) -> Result<RadarRecommendResponse> {
        let data = self
            .base
            .cgi(
                "music.recommend.TrackRelationServer",
                "GetRadarSong",
                json!({ "Page": page, "ReqType": 0, "FavSongs": [], "EntranceSongs": [] }),
                RequestOptions::default(),
            )
            .await?;
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
