//! QQ Music web discovery endpoints. Request construction and upstream card
//! interpretation belong here, not in a host application's provider adapter.

use super::ApiModule;
use crate::context::RequestOptions;
use crate::models::discovery::{AreaPage, CategoryCard, FeedCard, FeedShelf, NewMvCard};
use crate::{Credential, Platform, QmError, Result};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Clone, Debug)]
pub struct DiscoveryApi {
    base: ApiModule,
}

pub(crate) fn anonymous_web_read() -> RequestOptions {
    RequestOptions {
        comm: Some(json!({"ct":24,"cv":0})),
        override_comm: true,
        platform: Some(Platform::Web),
        // Public discovery must not inherit an unrelated mutable global account.
        credential: Some(Credential::default()),
        ..Default::default()
    }
}

impl DiscoveryApi {
    pub(crate) fn new(context: std::sync::Arc<crate::context::ApiContext>) -> Self {
        Self {
            base: ApiModule::new(context),
        }
    }

    pub async fn categories(&self) -> Result<Vec<CategoryCard>> {
        let data = self
            .base
            .cgi(
                "music.area.CategoryArea",
                "getCategoryAreaInCategoryPlaylist",
                json!({}),
                anonymous_web_read(),
            )
            .await?;
        Ok(decode_shelf(data)?
            .cards
            .into_iter()
            .filter_map(|card| {
                let start = card.id.find("encArea=")? + 8;
                let key = card.id[start..].split('&').next()?.to_owned();
                (!key.is_empty()).then_some(CategoryCard {
                    area_key: key,
                    title: card.title,
                    cover_url: card.cover_url,
                })
            })
            .collect())
    }

    pub async fn podcasts(&self) -> Result<Vec<FeedCard>> {
        #[derive(Deserialize)]
        struct Response {
            #[serde(rename = "radioList")]
            items: Vec<FeedCard>,
        }
        let data = self
            .base
            .cgi(
                "music.longRadio.recommend",
                "getRadioList",
                json!({"pos":6}),
                anonymous_web_read(),
            )
            .await?;
        let response: Response = serde_json::from_value(data)?;
        Ok(response.items)
    }

    pub async fn new_mvs(&self, offset: u32, limit: u32) -> Result<Vec<NewMvCard>> {
        if !(1..=100).contains(&limit) {
            return Err(QmError::ValueError(
                "MV page size must be between 1 and 100".into(),
            ));
        }
        #[derive(Deserialize)]
        struct Response {
            list: Vec<NewMvCard>,
        }
        let data = self
            .base
            .cgi(
                "MvService.MvInfoProServer",
                "GetNewMv",
                json!({"style":0,"tag":0,"start":offset,"size":limit}),
                anonymous_web_read(),
            )
            .await?;
        let response: Response = serde_json::from_value(data)?;
        Ok(response.list)
    }

    pub async fn featured(&self) -> Result<Vec<FeedCard>> {
        let data = self
            .base
            .cgi(
                "music.musicHall.MusicHallPlatformSvr",
                "GetFocus",
                json!({"Device":{"OS":"3","AppName":"QQ音乐"}}),
                anonymous_web_read(),
            )
            .await?;
        Ok(decode_shelf(data)?.cards)
    }

    pub async fn area(&self, area_key: &str) -> Result<AreaPage> {
        if area_key.is_empty() || area_key.len() > 2048 || area_key.chars().any(char::is_control) {
            return Err(QmError::ValueError("invalid encoded area key".into()));
        }
        let data = self
            .base
            .cgi(
                "music.area.AreaHome",
                "getAreaHomePage",
                json!({"encArea":area_key,"cmd":0}),
                anonymous_web_read(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }
}

fn decode_shelf(value: Value) -> Result<FeedShelf> {
    #[derive(Deserialize)]
    struct Response {
        shelf: FeedShelf,
    }
    Ok(serde_json::from_value::<Response>(value)?.shelf)
}
