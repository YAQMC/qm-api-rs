//! 专辑相关 API (对应 Python 端 `modules/album.py`).

use serde_json::json;

use super::ApiModule;
use crate::context::RequestOptions;
use crate::error::Result;
use crate::models::album::*;
#[cfg(feature = "experimental")]
use crate::models::Credential;

/// 专辑相关 API.
#[derive(Clone, Debug)]
pub struct AlbumApi {
    pub(crate) base: ApiModule,
}

impl AlbumApi {
    pub(crate) fn new(context: std::sync::Arc<crate::context::ApiContext>) -> Self {
        AlbumApi {
            base: ApiModule::new(context),
        }
    }

    /// 获取专辑详细信息.
    pub async fn get_detail(&self, value: &str) -> Result<GetAlbumDetailResponse> {
        let param = if value.chars().all(|c| c.is_ascii_digit()) && !value.is_empty() {
            json!({ "albumId": value.parse::<i64>().unwrap_or(0) })
        } else {
            json!({ "albumMId": value })
        };
        let data = self
            .base
            .cgi("music.musichallAlbum.AlbumInfoServer", "GetAlbumDetail", param, RequestOptions::default())
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取专辑歌曲列表.
    pub async fn get_song(&self, value: &str, num: i64, page: i64) -> Result<GetAlbumSongResponse> {
        let mut param = json!({ "begin": num * (page - 1), "num": num });
        if value.chars().all(|c| c.is_ascii_digit()) && !value.is_empty() {
            param["albumId"] = json!(value.parse::<i64>().unwrap_or(0));
        } else {
            param["albumMid"] = json!(value);
        }
        let data = self
            .base
            .cgi("music.musichallAlbum.AlbumSongList", "GetAlbumSongList", param, RequestOptions::default())
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取新碟上架列表.
    pub async fn get_new_album(&self, area: i64, num: i64, page: i64) -> Result<GetNewAlbumResponse> {
        let data = self
            .base
            .cgi(
                "newalbum.NewAlbumServer",
                "get_new_album_info",
                json!({ "area": area, "num": num, "start": num * (page - 1) }),
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// ⚠️ **Experimental** (feature `experimental`) — 收藏专辑到当前登录用户.
    ///
    /// 逆向自官方桌面客户端 ASAR; **请求参数名 (`v_albumId`) 与响应语义均为
    /// 猜测, 未获 ASAR/live 证据**, 不要对真实账号执行破坏性操作. 默认不编译,
    /// 需显式启用 `--features experimental`.
    #[cfg(feature = "experimental")]
    pub async fn fav_album(&self, album_id: &[i64], credential: Option<&Credential>) -> Result<AlbumFavWriteResponse> {
        let mut opts = RequestOptions::default();
        opts.require_login = true;
        opts.credential = credential.cloned();
        let data = self
            .base
            .cgi(
                "music.musicasset.AlbumFavWrite",
                "FavAlbum",
                json!({ "v_albumId": album_id }),
                opts,
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// ⚠️ **Experimental** (feature `experimental`) — 取消收藏专辑.
    ///
    /// 逆向自官方桌面客户端 ASAR; **请求参数名 (`v_albumId`) 与响应语义均为
    /// 猜测, 未获 ASAR/live 证据** (ASAR 中的参数名可能并非 `v_albumId`),
    /// 不要对真实账号执行破坏性操作. 默认不编译, 需显式启用
    /// `--features experimental`.
    #[cfg(feature = "experimental")]
    pub async fn del_fav_album(&self, album_id: &[i64], credential: Option<&Credential>) -> Result<AlbumFavWriteResponse> {
        let mut opts = RequestOptions::default();
        opts.require_login = true;
        opts.credential = credential.cloned();
        let data = self
            .base
            .cgi(
                "music.musicasset.AlbumFavWrite",
                "CancelFavAlbum",
                json!({ "v_albumId": album_id }),
                opts,
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }
}
