//! 歌单相关 API (对应 Python 端 `modules/songlist.py`).

use serde_json::{json, Value};

use super::ApiModule;
use crate::context::RequestOptions;
use crate::error::{QmError, Result};
use crate::models::songlist::*;
use crate::models::Credential;

/// 构建歌单写操作的最小 JSON 参数.
fn build_songlist_oper_param(dirid: i64, song_info: &[(i64, i64)], tid: i64) -> Value {
    json!({
        "dirId": dirid,
        "tid": tid,
        "bFmtUtf8": true,
        "v_songInfo": song_info.iter().map(|(sid, stype)| json!({ "songId": sid, "songType": stype })).collect::<Vec<_>>(),
    })
}

/// 歌单相关 API.
#[derive(Clone, Debug)]
pub struct SonglistApi {
    pub(crate) base: ApiModule,
}

impl SonglistApi {
    pub(crate) fn new(context: std::sync::Arc<crate::context::ApiContext>) -> Self {
        SonglistApi {
            base: ApiModule::new(context),
        }
    }

    /// 获取歌单详细信息和歌曲原始数据.
    #[allow(clippy::too_many_arguments)]
    pub async fn get_detail(
        &self,
        songlist_id: i64,
        dirid: i64,
        num: i64,
        page: i64,
        onlysong: bool,
        tag: bool,
        userinfo: bool,
    ) -> Result<GetSonglistDetailResponse> {
        let data = self
            .base
            .cgi(
                "music.srfDissInfo.DissInfo",
                "CgiGetDiss",
                json!({
                    "disstid": songlist_id,
                    "dirid": dirid,
                    "tag": tag,
                    "song_begin": num * (page - 1),
                    "song_num": num,
                    "userinfo": userinfo,
                    "orderlist": true,
                    "onlysonglist": onlysong,
                }),
                RequestOptions::default(),
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 创建歌单.
    pub async fn create(
        &self,
        dirname: &str,
        credential: Option<&Credential>,
    ) -> Result<CreateDeleteSonglistResp> {
        let mut opts = RequestOptions::default();
        opts.require_login = true;
        opts.retry = crate::RetryClass::Write;
        opts.credential = credential.cloned();
        let data = self
            .base
            .cgi(
                "music.musicasset.PlaylistBaseWrite",
                "AddPlaylist",
                json!({ "dirName": dirname }),
                opts,
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 删除歌单.
    pub async fn delete(
        &self,
        dirid: i64,
        credential: Option<&Credential>,
    ) -> Result<CreateDeleteSonglistResp> {
        let mut opts = RequestOptions::default();
        opts.require_login = true;
        opts.retry = crate::RetryClass::Write;
        opts.credential = credential.cloned();
        let data = self
            .base
            .cgi(
                "music.musicasset.PlaylistBaseWrite",
                "DelPlaylist",
                json!({ "dirId": dirid }),
                opts,
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 添加歌曲到歌单.
    pub async fn add_songs(
        &self,
        dirid: i64,
        song_info: &[(i64, i64)],
        tid: i64,
        credential: Option<&Credential>,
    ) -> Result<bool> {
        let mut opts = RequestOptions::default();
        opts.require_login = true;
        opts.retry = crate::RetryClass::Write;
        opts.credential = credential.cloned();
        opts.preserve_bool = true;
        let result = self
            .base
            .cgi(
                "music.musicasset.PlaylistDetailWrite",
                "AddSonglist",
                build_songlist_oper_param(dirid, song_info, tid),
                opts,
            )
            .await;
        match result {
            Ok(data) => Ok(data.get("retCode").and_then(Value::as_i64).unwrap_or(-1) == 0),
            Err(QmError::CgiApi { code: 80092, .. }) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// 删除歌单中的歌曲.
    pub async fn del_songs(
        &self,
        dirid: i64,
        song_info: &[(i64, i64)],
        tid: i64,
        credential: Option<&Credential>,
    ) -> Result<bool> {
        let mut opts = RequestOptions::default();
        opts.require_login = true;
        opts.retry = crate::RetryClass::Write;
        opts.credential = credential.cloned();
        let result = self
            .base
            .cgi(
                "music.musicasset.PlaylistDetailWrite",
                "DelSonglist",
                build_songlist_oper_param(dirid, song_info, tid),
                opts,
            )
            .await;
        match result {
            Ok(data) => Ok(data.get("retCode").and_then(Value::as_i64).unwrap_or(-1) == 0),
            Err(QmError::CgiApi { code: 80092, .. }) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// 收藏歌曲到 "我喜欢" 歌单 (dirid 固定为 201).
    pub async fn like_song(
        &self,
        song_info: &[(i64, i64)],
        credential: Option<&Credential>,
    ) -> Result<bool> {
        self.add_songs(201, song_info, 0, credential).await
    }

    /// 从 "我喜欢" 歌单移除歌曲.
    pub async fn unlike_song(
        &self,
        song_info: &[(i64, i64)],
        credential: Option<&Credential>,
    ) -> Result<bool> {
        self.del_songs(201, song_info, 0, credential).await
    }

    // ------------------------------------------------------------------
    // 以下接口补充自官方桌面客户端 (Electron ASAR) `common.js`.
    // ------------------------------------------------------------------

    /// ⚠️ **Raw 透传** — 获取指定歌单下的歌曲
    /// (官方桌面端 `PlaylistDetailRead / GetUniformSongDetailInfo`).
    ///
    /// 可用于获取"我喜欢"歌单 (`dirId=201`) 的歌曲详情; 参数与响应 schema
    /// 未经 live 验证, 仅提供透传能力.
    pub async fn raw_get_uniform_song_detail(
        &self,
        param: Value,
        credential: Option<&Credential>,
    ) -> Result<Value> {
        let mut opts = RequestOptions::default();
        opts.credential = credential.cloned();
        self.base
            .cgi(
                "music.musicasset.PlaylistDetailRead",
                "GetUniformSongDetailInfo",
                param,
                opts,
            )
            .await
    }

    /// ⚠️ **Raw 透传** — 按目录 ID 获取歌单歌曲
    /// (官方桌面端 `PlaylistDetailRead / GetSongDetailInfoListByDirId`).
    pub async fn raw_get_song_detail_info_list_by_dirid(
        &self,
        param: Value,
        credential: Option<&Credential>,
    ) -> Result<Value> {
        let mut opts = RequestOptions::default();
        opts.credential = credential.cloned();
        self.base
            .cgi(
                "music.musicasset.PlaylistDetailRead",
                "GetSongDetailInfoListByDirId",
                param,
                opts,
            )
            .await
    }

    /// ⚠️ **Raw 透传** — 检查歌单是否已收藏
    /// (官方桌面端 `PlaylistFavRead / IsPlaylistFan`).
    ///
    /// `param` 通常形如 `{"v_playlistId": [id]}`.
    pub async fn raw_is_playlist_fan(
        &self,
        param: Value,
        credential: Option<&Credential>,
    ) -> Result<Value> {
        let mut opts = RequestOptions::default();
        opts.require_login = true;
        opts.credential = credential.cloned();
        self.base
            .cgi(
                "music.musicasset.PlaylistFavRead",
                "IsPlaylistFan",
                param,
                opts,
            )
            .await
    }

    /// ⚠️ **Raw 透传** — 对歌单歌曲重新排序
    /// (官方桌面端 `PlaylistDetailWrite / SeqSonglist`).
    pub async fn raw_seq_songlist(
        &self,
        param: Value,
        credential: Option<&Credential>,
    ) -> Result<bool> {
        let mut opts = RequestOptions::default();
        opts.require_login = true;
        opts.retry = crate::RetryClass::Write;
        opts.credential = credential.cloned();
        let data = self
            .base
            .cgi(
                "music.musicasset.PlaylistDetailWrite",
                "SeqSonglist",
                param,
                opts,
            )
            .await?;
        Ok(data.get("retCode").and_then(Value::as_i64).unwrap_or(-1) == 0)
    }

    /// ⚠️ **Raw 透传** — 取消收藏长音频 (官方桌面端 `music.favor_system_write / do_favor`).
    pub async fn raw_cancel_fav_audio(
        &self,
        param: Value,
        credential: Option<&Credential>,
    ) -> Result<Value> {
        let mut opts = RequestOptions::default();
        opts.require_login = true;
        opts.retry = crate::RetryClass::Write;
        opts.credential = credential.cloned();
        self.base
            .cgi("music.favor_system_write", "do_favor", param, opts)
            .await
    }
}
