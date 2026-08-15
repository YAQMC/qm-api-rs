//! 排行榜相关 API (对应 Python 端 `modules/top.py`).

use serde_json::json;

use super::ApiModule;
use crate::context::RequestOptions;
use crate::error::Result;
use crate::models::top::*;

/// 排行榜相关 API.
#[derive(Clone, Debug)]
pub struct TopApi {
    pub(crate) base: ApiModule,
}

impl TopApi {
    pub(crate) fn new(context: std::sync::Arc<crate::context::ApiContext>) -> Self {
        TopApi {
            base: ApiModule::new(context),
        }
    }

    /// 获取所有排行榜分类.
    pub async fn get_category(&self) -> Result<TopCategoryResponse> {
        let data = self
            .base
            .cgi(
                "music.musicToplist.Toplist",
                "GetAll",
                json!({}),
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取排行榜详情及其歌曲列表.
    pub async fn get_detail(
        &self,
        top_id: i64,
        num: i64,
        page: i64,
        tag: bool,
    ) -> Result<TopDetailResponse> {
        let mut param = json!({
            "topId": top_id,
            "offset": num * (page - 1),
            "num": num,
        });
        if tag {
            param["withTags"] = json!(true);
        }
        let mut opts = RequestOptions::default();
        opts.preserve_bool = tag;
        let data = self
            .base
            .cgi("music.musicToplist.Toplist", "GetDetail", param, opts)
            .await?;
        Ok(serde_json::from_value(data)?)
    }
}
