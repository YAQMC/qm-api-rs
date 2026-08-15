//! # qqmusic-api
//!
//! 纯 Rust 实现的 QQ 音乐异步 API 客户端, 移植自
//! [L-1124/QQMusicApi](https://github.com/L-1124/QQMusicApi) (Python),
//! 并参考官方桌面客户端 (Electron ASAR) 补充了签名接口、平台参数等细节.
//!
//! ## 快速开始
//!
//! ```no_run
//! use qqmusic_api::Client;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let client = Client::new(None, None)?;
//!     let result = client.search.search_by_type("周杰伦", qqmusic_api::SearchType::Song, 5, 1, &[], None, true).await?;
//!     println!("单曲结果数量: {}", result.song.len());
//!     Ok(())
//! }
//! ```
//!
//! ## 模块概览
//!
//! - `Client::song` —— 歌曲信息、播放链接、曲谱等
//! - `Client::search` —— 搜索 (综合/类型/热搜/补全)
//! - `Client::singer` —— 歌手
//! - `Client::album` —— 专辑
//! - `Client::lyric` —— 歌词 (含 QRC 解密)
//! - `Client::mv` —— MV
//! - `Client::top` —— 排行榜
//! - `Client::songlist` —— 歌单
//! - `Client::comment` —— 评论
//! - `Client::recommend` —— 推荐
//! - `Client::user` —— 用户
//! - `Client::login` —— 登录 (QQ/微信二维码, 手机验证码)
//! - `Client::helper` —— 上传等辅助接口
//! - `Client::private_message` —— 私信

mod client;
mod context;
pub mod credential_store;
mod device;
mod error;
mod jsonpath;
pub mod lyric_parser;
mod mqtt;
mod pagination;
pub mod qmc;
mod qimei;
mod rate_limiter;
mod sign;
mod tripledes;
mod utils;
mod versioning;

pub mod models;
pub mod modules;

pub use client::{CgiOptions, Client, HttpOptions};
pub use credential_store::CredentialStore;
pub use device::{random_imei, Device, OSVersion};
pub use error::{QmError, Result};
pub use models::Credential;
pub use modules::helper_utils::UploadFileSession;
pub use modules::login_utils::{PhoneLoginSession, PollInterval, QRCodeLoginSession};
pub use modules::search::SearchType;
pub use modules::singer::{AreaType, GenreType, IndexType, SexType, TabType};
pub use modules::song::{SongFileInfo, SongFileType, SongQuality};
pub use pagination::{Pager, page, offset};
pub use utils::{calc_md5, get_guid, get_search_id, hash33};
pub use versioning::{Platform, VersionPolicy};

/// 模块子类型便捷导出.
pub use models::{
    Album, File, MV, Pay, Singer, Song, SongList,
};
