//! 用户相关 API (对应 Python 端 `modules/user.py`).

use serde_json::{json, Value};

use super::ApiModule;
use crate::context::RequestOptions;
use crate::error::Result;
use crate::models::songlist::GetSonglistDetailResponse;
use crate::models::user::*;
use crate::models::Credential;

/// 占位凭证 (用于无需登录但仍需凭证的公共接口).
fn placeholder_credential() -> Credential {
    Credential {
        musicid: 1,
        str_musicid: "1".into(),
        musickey: "placeholder-musickey".into(),
        encrypt_uin: "00000000000000000000000000000000".into(),
        login_type: 1,
        ..Default::default()
    }
}

/// 用户相关 API.
#[derive(Clone, Debug)]
pub struct UserApi {
    pub(crate) base: ApiModule,
}

impl UserApi {
    pub(crate) fn new(context: std::sync::Arc<crate::context::ApiContext>) -> Self {
        UserApi {
            base: ApiModule::new(context),
        }
    }

    fn resolve_placeholder_credential(&self, credential: Option<&Credential>) -> Credential {
        if let Some(c) = credential {
            return c.clone();
        }
        let current = self.base.credential();
        if current.musicid != 0 && !current.musickey.is_empty() {
            current
        } else {
            placeholder_credential()
        }
    }

    /// 获取用户主页头部及统计信息.
    pub async fn get_homepage(
        &self,
        euin: &str,
        credential: Option<&Credential>,
    ) -> Result<UserHomepageResponse> {
        let target = self.resolve_placeholder_credential(credential);
        let mut opts = RequestOptions::default();
        opts.credential = Some(target);
        let data = self
            .base
            .cgi(
                "music.UnifiedHomepage.UnifiedHomepageSrv",
                "GetHomepageHeader",
                json!({ "uin": euin, "IsQueryTabDetail": 1 }),
                opts,
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取当前登录账号的 VIP 会员信息.
    pub async fn get_vip_info(
        &self,
        credential: Option<&Credential>,
    ) -> Result<UserVipInfoResponse> {
        let mut opts = RequestOptions::default();
        opts.require_login = true;
        opts.credential = credential.cloned();
        let data = self
            .base
            .cgi("VipLogin.VipLoginInter", "vip_login_base", json!({}), opts)
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 检查账号是否拥有 VIP 权益 (绿钻 / 超级会员 / 星级会员).
    ///
    /// 需要登录凭证. 返回 `true` 表示具备 VIP 音质访问能力.
    pub async fn is_vip(&self, credential: Option<&Credential>) -> Result<bool> {
        let info = self.get_vip_info(credential).await?;
        Ok(info.identity.vip > 0
            || info.identity.huge_vip > 0
            || info.svip > 0
            || info.star > 0
            || info.ystar > 0)
    }

    /// 获取用户关注的歌手列表.
    pub async fn get_follow_singers(
        &self,
        euin: &str,
        page: i64,
        num: i64,
        credential: Option<&Credential>,
    ) -> Result<UserRelationListResponse> {
        let mut opts = RequestOptions::default();
        opts.require_login = true;
        opts.credential = credential.cloned();
        let data = self
            .base
            .cgi(
                "music.concern.RelationList",
                "GetFollowSingerList",
                json!({ "HostUin": euin, "From": (page - 1) * num, "Size": num }),
                opts,
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取用户粉丝列表.
    pub async fn get_fans(
        &self,
        euin: &str,
        page: i64,
        num: i64,
        credential: Option<&Credential>,
    ) -> Result<UserRelationListResponse> {
        let mut opts = RequestOptions::default();
        opts.require_login = true;
        opts.credential = credential.cloned();
        let data = self
            .base
            .cgi(
                "music.concern.RelationList",
                "GetFansList",
                json!({ "HostUin": euin, "From": (page - 1) * num, "Size": num }),
                opts,
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取好友列表.
    pub async fn get_friend(
        &self,
        page: i64,
        num: i64,
        credential: Option<&Credential>,
    ) -> Result<UserFriendListResponse> {
        let mut opts = RequestOptions::default();
        opts.require_login = true;
        opts.credential = credential.cloned();
        let data = self
            .base
            .cgi(
                "music.homepage.Friendship",
                "GetFriendList",
                json!({ "PageSize": num, "Page": page - 1 }),
                opts,
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取关注的用户列表.
    pub async fn get_follow_user(
        &self,
        euin: &str,
        page: i64,
        num: i64,
        credential: Option<&Credential>,
    ) -> Result<UserRelationListResponse> {
        let mut opts = RequestOptions::default();
        opts.require_login = true;
        opts.credential = credential.cloned();
        let data = self
            .base
            .cgi(
                "music.concern.RelationList",
                "GetFollowUserList",
                json!({ "HostUin": euin, "From": (page - 1) * num, "Size": num }),
                opts,
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取用户创建的歌单列表.
    pub async fn get_created_songlist(
        &self,
        uin: i64,
        credential: Option<&Credential>,
    ) -> Result<UserCreatedSonglistResponse> {
        let mut opts = RequestOptions::default();
        opts.credential = credential.cloned();
        let data = self
            .base
            .cgi(
                "music.musicasset.PlaylistBaseRead",
                "GetPlaylistByUin",
                json!({ "uin": uin.to_string() }),
                opts,
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取用户收藏的歌曲列表 (dirid=201).
    pub async fn get_fav_song(
        &self,
        euin: &str,
        page: i64,
        num: i64,
        credential: Option<&Credential>,
    ) -> Result<GetSonglistDetailResponse> {
        let mut opts = RequestOptions::default();
        opts.credential = credential.cloned();
        let data = self
            .base
            .cgi(
                "music.srfDissInfo.DissInfo",
                "CgiGetDiss",
                json!({
                    "disstid": 0,
                    "dirid": 201,
                    "tag": true,
                    "song_begin": num * (page - 1),
                    "song_num": num,
                    "userinfo": true,
                    "orderlist": true,
                    "enc_host_uin": euin,
                }),
                opts,
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取用户收藏的外部歌单列表.
    pub async fn get_fav_songlist(
        &self,
        euin: &str,
        page: i64,
        num: i64,
        credential: Option<&Credential>,
    ) -> Result<UserFavSonglistResponse> {
        let mut opts = RequestOptions::default();
        opts.credential = credential.cloned();
        let data = self
            .base
            .cgi(
                "music.musicasset.PlaylistFavRead",
                "CgiGetPlaylistFavInfo",
                json!({ "uin": euin, "offset": (page - 1) * num, "size": num }),
                opts,
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 收藏歌单.
    pub async fn fav_songlist(
        &self,
        songlist_id: i64,
        credential: Option<&Credential>,
    ) -> Result<bool> {
        let cred = credential
            .cloned()
            .unwrap_or_else(|| self.base.credential());
        let mut opts = RequestOptions::default();
        opts.require_login = true;
        opts.credential = credential.cloned();
        let data = self
            .base
            .cgi(
                "music.musicasset.PlaylistFavWrite",
                "FavPlaylist",
                json!({ "uin": cred.encrypt_uin, "v_playlistId": [songlist_id] }),
                opts,
            )
            .await?;
        let result = data.get("result").and_then(Value::as_i64).unwrap_or(-1);
        let failed = data
            .get("v_failedPlaylistId")
            .and_then(Value::as_array)
            .map(|a| a.iter().any(|v| v.as_i64() == Some(songlist_id)))
            .unwrap_or(false);
        Ok(result == 0 && !failed)
    }

    /// 取消收藏歌单.
    pub async fn unfav_songlist(
        &self,
        songlist_id: i64,
        credential: Option<&Credential>,
    ) -> Result<bool> {
        let cred = credential
            .cloned()
            .unwrap_or_else(|| self.base.credential());
        let mut opts = RequestOptions::default();
        opts.require_login = true;
        opts.credential = credential.cloned();
        let data = self
            .base
            .cgi(
                "music.musicasset.PlaylistFavWrite",
                "CancelFavPlaylist",
                json!({ "uin": cred.encrypt_uin, "v_playlistId": [songlist_id] }),
                opts,
            )
            .await?;
        let result = data.get("result").and_then(Value::as_i64).unwrap_or(-1);
        let failed = data
            .get("v_failedPlaylistId")
            .and_then(Value::as_array)
            .map(|a| a.iter().any(|v| v.as_i64() == Some(songlist_id)))
            .unwrap_or(false);
        Ok(result == 0 && !failed)
    }

    /// 获取用户收藏的专辑列表.
    pub async fn get_fav_album(
        &self,
        euin: &str,
        page: i64,
        num: i64,
        credential: Option<&Credential>,
    ) -> Result<UserFavAlbumResponse> {
        let mut opts = RequestOptions::default();
        opts.credential = credential.cloned();
        let data = self
            .base
            .cgi(
                "music.musicasset.AlbumFavRead",
                "CgiGetAlbumFavInfo",
                json!({ "euin": euin, "offset": (page - 1) * num, "size": num }),
                opts,
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取用户收藏的 MV 列表.
    pub async fn get_fav_mv(
        &self,
        euin: &str,
        page: i64,
        num: i64,
        credential: Option<&Credential>,
    ) -> Result<UserFavMvResponse> {
        let mut opts = RequestOptions::default();
        opts.require_login = true;
        opts.credential = credential.cloned();
        let data = self
            .base
            .cgi(
                "music.musicasset.MVFavRead",
                "getMyFavMV_v2",
                json!({ "encuin": euin, "pagesize": num, "num": page - 1 }),
                opts,
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取用户的音乐基因数据.
    pub async fn get_music_gene(
        &self,
        euin: &str,
        credential: Option<&Credential>,
    ) -> Result<UserMusicGeneResponse> {
        let mut opts = RequestOptions::default();
        opts.credential = credential.cloned();
        let data = self
            .base
            .cgi(
                "music.recommend.UserProfileSettingSvr",
                "GetProfileReport",
                json!({ "VisitAccount": euin }),
                opts,
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 获取用户不喜欢列表.
    #[allow(clippy::too_many_arguments)]
    pub async fn get_dislike_list(
        &self,
        cmd: i64,
        page: i64,
        lastid: i64,
        credential: Option<&Credential>,
    ) -> Result<DislikeListData> {
        let lastid_fields = [("SingersLastid", 2), ("SongLastid", 3), ("StyleLastid", 4)];
        let mut param = json!({ "Cmd": cmd, "Page": page });
        if lastid != 0 {
            if let Some((key, _)) = lastid_fields.iter().find(|(_, c)| *c == cmd) {
                param[key] = json!(lastid);
            }
        }
        let mut opts = RequestOptions::default();
        opts.require_login = true;
        opts.credential = credential.cloned();
        opts.sign = true;
        let data = self
            .base
            .cgi(
                "music.feedback.FeedbackBlack",
                "GetDislikeList",
                param,
                opts,
            )
            .await?;
        Ok(serde_json::from_value(data)?)
    }

    /// 添加不喜欢.
    pub async fn add_dislike(
        &self,
        id_type: DislikeIdType,
        values: &[i64],
        credential: Option<&Credential>,
    ) -> Result<bool> {
        let key = id_type.as_key();
        let id_type_val = id_type as i64;
        let items: Vec<Value> = values
            .iter()
            .map(|v| json!({ "ID": v.to_string(), "IdType": id_type_val }))
            .collect();
        let mut opts = RequestOptions::default();
        opts.require_login = true;
        opts.credential = credential.cloned();
        let data = self
            .base
            .cgi(
                "music.feedback.FeedbackBlack",
                "AddDislike",
                json!({ key: items }),
                opts,
            )
            .await?;
        Ok(data.get("Retcode").and_then(Value::as_i64).unwrap_or(-1) == 0)
    }

    /// 取消不喜欢.
    pub async fn cancel_dislike(
        &self,
        id_type: DislikeIdType,
        values: &[i64],
        credential: Option<&Credential>,
    ) -> Result<bool> {
        let key = id_type.as_key();
        let id_type_val = id_type as i64;
        let items: Vec<Value> = values
            .iter()
            .map(|v| json!({ "ID": v.to_string(), "IdType": id_type_val }))
            .collect();
        let mut opts = RequestOptions::default();
        opts.require_login = true;
        opts.credential = credential.cloned();
        let data = self
            .base
            .cgi(
                "music.feedback.FeedbackBlack",
                "CancelDislike",
                json!({ key: items }),
                opts,
            )
            .await?;
        Ok(data.get("Retcode").and_then(Value::as_i64).unwrap_or(-1) == 0)
    }

    /// 清空所有不喜欢歌曲.
    pub async fn cancel_all_dislike_song(&self, credential: Option<&Credential>) -> Result<bool> {
        let mut opts1 = RequestOptions::default();
        opts1.require_login = true;
        opts1.credential = credential.cloned();
        opts1.preserve_bool = true;
        let data1 = self
            .base
            .cgi(
                "music.feedback.FeedbackBlack",
                "CancelAllDislike",
                json!({ "ISOnlyGetToken": true }),
                opts1,
            )
            .await?;
        let token = data1
            .get("Token")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let mut opts2 = RequestOptions::default();
        opts2.require_login = true;
        opts2.credential = credential.cloned();
        let data2 = self
            .base
            .cgi(
                "music.feedback.FeedbackBlack",
                "CancelAllDislike",
                json!({ "DelType": 3, "Token": token }),
                opts2,
            )
            .await?;
        Ok(data2.get("Retcode").and_then(Value::as_i64).unwrap_or(-1) == 0)
    }

    // ------------------------------------------------------------------
    // 以下接口补充自官方桌面客户端 (Electron ASAR) `common.js`.
    // ------------------------------------------------------------------

    /// ⚠️ **Raw 透传** — 查询 VIP 会员信息
    /// (官方桌面端 `userInfo.VipQueryServer / SRFVipQuery_V2`).
    ///
    /// 与 `get_vip_info` (VipLogin) 不同, 该接口为桌面端专用; 参数与响应
    /// schema 未经 live 验证, 仅提供透传能力.
    pub async fn raw_get_user_vip_info(
        &self,
        param: Value,
        credential: Option<&Credential>,
    ) -> Result<Value> {
        let mut opts = RequestOptions::default();
        opts.require_login = true;
        opts.credential = credential.cloned();
        self.base
            .cgi("userInfo.VipQueryServer", "SRFVipQuery_V2", param, opts)
            .await
    }

    /// ⚠️ **Raw 透传** — 获取用户基础信息
    /// (官方桌面端 `userInfo.BaseUserInfoServer / get_user_baseinfo_v2`).
    ///
    /// `param` 通常形如 `{"vec_uin": ["xxx"], "need_profile": 1}`.
    pub async fn raw_get_user_base_info(
        &self,
        param: Value,
        credential: Option<&Credential>,
    ) -> Result<Value> {
        let mut opts = RequestOptions::default();
        opts.require_login = true;
        opts.credential = credential.cloned();
        self.base
            .cgi(
                "userInfo.BaseUserInfoServer",
                "get_user_baseinfo_v2",
                param,
                opts,
            )
            .await
    }

    /// ⚠️ **Experimental** (feature `experimental`) — 关注 / 取消关注歌手
    /// (官方桌面端 `Concern.ConcernSystemServer / cgi_concern_user_v2`).
    ///
    /// - `action`: `ConcernAction::Follow`(0) / `Unfollow`(1) (正反值尚未经 live test 确认)
    /// - `mid`: 歌手 MID
    ///
    /// 默认不编译, 需显式启用 `--features experimental`.
    #[cfg(feature = "experimental")]
    pub async fn focus_singer(
        &self,
        action: ConcernAction,
        mid: &str,
        credential: Option<&Credential>,
    ) -> Result<bool> {
        let mut opts = RequestOptions::default();
        opts.require_login = true;
        opts.credential = credential.cloned();
        let data = self
            .base
            .cgi(
                "Concern.ConcernSystemServer",
                "cgi_concern_user_v2",
                json!({
                    "opertype": action as i64,
                    "source": 0,
                    "userinfo": { "usertype": 1, "userid": mid },
                }),
                opts,
            )
            .await?;
        Ok(data.get("code").and_then(Value::as_i64).unwrap_or(-1) == 0)
    }

    /// ⚠️ **Experimental** (feature `experimental`) — 收藏 / 取消收藏 MV
    /// (官方桌面端 `music.musicasset.MVFavWrite / AddDelFavMV`).
    ///
    /// - `action`: `MvFavAction::Fav`(0) / `Unfav`(1) (cmdtype 语义未验证)
    ///
    /// **请求 payload 为猜测**: ASAR 证据显示该接口实际携带 `cmdtype` 字段,
    /// 与当前实现 (`vid`/`opType`) 可能不符; 默认不编译, 需显式启用
    /// `--features experimental`.
    #[cfg(feature = "experimental")]
    pub async fn fav_mv(
        &self,
        vid: &str,
        action: MvFavAction,
        credential: Option<&Credential>,
    ) -> Result<Value> {
        let mut opts = RequestOptions::default();
        opts.require_login = true;
        opts.credential = credential.cloned();
        self.base
            .cgi(
                "music.musicasset.MVFavWrite",
                "AddDelFavMV",
                json!({ "vid": vid, "opType": action as i64 }),
                opts,
            )
            .await
    }

    /// ⚠️ **Raw 透传** — 获取收藏的电台列表
    /// (官方桌面端 `music.favorSystemRead.FavorSystem / get_favor_list`).
    pub async fn raw_get_favor_list(
        &self,
        param: Value,
        credential: Option<&Credential>,
    ) -> Result<Value> {
        let mut opts = RequestOptions::default();
        opts.require_login = true;
        opts.credential = credential.cloned();
        self.base
            .cgi(
                "music.favorSystemRead.FavorSystem",
                "get_favor_list",
                param,
                opts,
            )
            .await
    }

    /// ⚠️ **Raw 透传** — 获取收藏专辑列表
    /// (官方桌面端 `music.musicasset.AlbumFavRead / GetAlbumFavInfo`).
    pub async fn raw_get_collect_album_list(
        &self,
        param: Value,
        credential: Option<&Credential>,
    ) -> Result<Value> {
        let mut opts = RequestOptions::default();
        opts.credential = credential.cloned();
        self.base
            .cgi(
                "music.musicasset.AlbumFavRead",
                "GetAlbumFavInfo",
                param,
                opts,
            )
            .await
    }
}
