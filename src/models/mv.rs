//! MV API 返回模型定义 (对应 Python 端 `models/mv.py`).

use serde::Deserialize;
use serde_json::Value;

use super::base::{Singer, MV};
use crate::jsonpath_model;

/// MV 详情条目.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct MvDetail {
    #[serde(flatten)]
    pub base: MV,
    pub cover_pic: String,
    pub duration: i64,
    pub singers: Vec<Value>,
    pub video_switch: i64,
    pub msg: String,
    pub desc: String,
    pub playcnt: i64,
    pub pubdate: i64,
    pub isfav: i64,
    pub gmid: String,
    pub uploader_headurl: String,
    pub uploader_nick: String,
    pub uploader_encuin: String,
    pub uploader_uin: String,
    pub uploader_hasfollow: i64,
    pub uploader_follower_num: i64,
    pub related_songs: Vec<i64>,
}

jsonpath_model!(GetMvDetailResponse {
    data: "$" => std::collections::HashMap<String, MvDetail>,
});

/// 单一路径规格下的 MV 播放地址信息.
#[derive(Clone, Default, Deserialize)]
#[serde(default)]
pub struct MvUrlItem {
    pub url: Vec<String>,
    pub freeflow_url: Vec<String>,
    pub comm_url: Vec<String>,
    pub cn: String,
    pub vkey: String,
    pub expire: i64,
    pub code: i64,
    pub filetype: i64,
    pub m3u8: String,
    #[serde(alias = "newFileType")]
    pub new_file_type: i64,
    pub format: i64,
    #[serde(alias = "fileSize")]
    pub file_size: i64,
}

impl std::fmt::Debug for MvUrlItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fn redact_urls(urls: &[String]) -> String {
            if urls.is_empty() {
                "[]".into()
            } else {
                format!("[redacted; {} urls]", urls.len())
            }
        }
        f.debug_struct("MvUrlItem")
            .field("url", &redact_urls(&self.url))
            .field("freeflow_url", &redact_urls(&self.freeflow_url))
            .field("comm_url", &redact_urls(&self.comm_url))
            .field("cn", &self.cn)
            .field(
                "vkey",
                &if self.vkey.is_empty() {
                    ""
                } else {
                    "[redacted]"
                },
            )
            .field("expire", &self.expire)
            .field("code", &self.code)
            .field("filetype", &self.filetype)
            .field(
                "m3u8",
                &if self.m3u8.is_empty() {
                    ""
                } else {
                    "[redacted]"
                },
            )
            .field("new_file_type", &self.new_file_type)
            .field("format", &self.format)
            .field("file_size", &self.file_size)
            .finish()
    }
}

/// 同一 MV 在不同协议下的播放地址集合.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct MvUrlSet {
    pub mp4: Vec<MvUrlItem>,
    pub hls: Vec<MvUrlItem>,
    pub svp_flag: i64,
    pub duration: i64,
}

jsonpath_model!(GetMvUrlsResponse {
    data: "$" => std::collections::HashMap<String, MvUrlSet>,
});

/// MV 分类列表中的单个 MV 摘要.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct MvListItem {
    #[serde(flatten)]
    pub base: MV,
    pub singers: Vec<Singer>,
    pub subtitle: String,
    pub playcnt: i64,
    pub pubdate: i64,
    pub duration: i64,
    pub picurl: String,
}

jsonpath_model!(GetMvListResponse {
    total: "$.total" => i64,
    items: "$.list" => Vec<MvListItem>,
});
