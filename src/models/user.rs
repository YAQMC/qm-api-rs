//! User API 返回模型定义 (对应 Python 端 `models/user.py`).

use serde::Deserialize;
use serde_json::Value;

use super::base::{Album, MV, Singer, SongList};
use crate::jsonpath_model;

/// 用户歌单列表中的单个歌单摘要.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct UserPlaylistSummary {
    #[serde(flatten)]
    pub base: SongList,
    #[serde(alias = "createTime")]
    pub create_time: i64,
    #[serde(alias = "updateTime")]
    pub update_time: i64,
    pub uin: String,
    pub nick: String,
    #[serde(alias = "bigpicUrl")]
    pub bigpic_url: String,
    #[serde(alias = "albumPicUrl")]
    pub album_pic_url: String,
    pub avatar: String,
    #[serde(alias = "identIcon")]
    pub ident_icon: String,
    #[serde(alias = "layerUrl")]
    pub layer_url: String,
    pub invalid: bool,
    #[serde(alias = "dirShow")]
    pub dir_show: i64,
    #[serde(alias = "fav_cnt")]
    pub create_fav_cnt: i64,
    pub play_cnt: i64,
    pub comment_cnt: i64,
    #[serde(alias = "opType")]
    pub op_type: i64,
    #[serde(alias = "sortWeight")]
    pub sort_weight: i64,
}

jsonpath_model!(UserCreatedSonglistResponse {
    total: "$.total" => i64,
    playlists: "$.v_playlist[*]" => Vec<UserPlaylistSummary>,
    deleted_ids: "$.v_delTid" => Vec<i64>,
    finished: "$.bFinish" => bool,
});

/// 用户收藏歌单列表中的单个条目.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct UserFavSonglistItem {
    #[serde(flatten)]
    pub base: SongList,
    pub uin: String,
    pub nickname: String,
    #[serde(alias = "createtime")]
    pub create_time: i64,
    #[serde(alias = "updateTime")]
    pub update_time: i64,
    #[serde(alias = "orderTime")]
    pub order_time: i64,
    #[serde(alias = "dirShow")]
    pub dir_show: i64,
    #[serde(alias = "dirType")]
    pub dir_type: i64,
    #[serde(alias = "edgeMark")]
    pub edge_mark: String,
    #[serde(alias = "layerUrl")]
    pub layer_url: String,
    #[serde(alias = "albumPicUrl")]
    pub album_pic_url: String,
    #[serde(alias = "opType")]
    pub op_type: i64,
    #[serde(alias = "sortWeight")]
    pub sort_weight: i64,
    pub readtime: i64,
}

jsonpath_model!(UserFavSonglistResponse {
    number: "$.number" => i64,
    total: "$.total" => i64,
    hasmore: "$.hasmore" => i64,
    hide: "$.hide" => bool,
    playlists: "$.v_list" => Vec<UserFavSonglistItem>,
    deleted_ids: "$.v_delTids" => Vec<i64>,
    failed_ids: "$.v_failTids" => Vec<i64>,
});

/// 用户收藏专辑列表中的单个专辑条目.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct UserFavAlbumItem {
    #[serde(flatten)]
    pub base: Album,
    pub songnum: i64,
    pub pubtime: i64,
    pub ordertime: i64,
    pub status: i64,
    pub loc: i64,
    #[serde(alias = "v_singer")]
    pub singers: Vec<Singer>,
}

jsonpath_model!(UserFavAlbumResponse {
    number: "$.number" => i64,
    total: "$.total" => i64,
    hasmore: "$.hasmore" => i64,
    hide: "$.hide" => bool,
    albums: "$.v_list[*]" => Vec<UserFavAlbumItem>,
    failed_album_ids: "$.v_failAlbumId" => Vec<i64>,
});

/// 用户音乐基因页头部卡片信息.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct UserInfoCard {
    #[serde(alias = "HeadUrl")]
    pub head_url: String,
    #[serde(alias = "NickName")]
    pub nick_name: String,
    #[serde(alias = "Signature")]
    pub signature: String,
    #[serde(alias = "EncryptionAccount")]
    pub encryption_account: String,
    #[serde(alias = "Preferences")]
    pub preferences: Value,
}

/// 用户听歌报告摘要.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ListeningReport {
    #[serde(alias = "Report")]
    pub report: Vec<Value>,
}

/// 用户音乐基因视图响应.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct UserMusicGeneResponse {
    #[serde(alias = "UserInfoCard")]
    pub user_info_card: UserInfoCard,
    #[serde(alias = "ListeningReport")]
    pub listening_report: ListeningReport,
    #[serde(alias = "SortArray")]
    pub sort_array: Vec<i64>,
    #[serde(alias = "IsVisitAccount")]
    pub is_visit_account: bool,
}

/// 用户主页头部基础信息.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct UserHomepageBaseInfo {
    #[serde(alias = "EncryptedUin")]
    pub encrypted_uin: String,
    #[serde(alias = "Name")]
    pub name: String,
    #[serde(alias = "Avatar")]
    pub avatar: String,
    #[serde(alias = "BackgroundImage")]
    pub background_image: String,
    #[serde(alias = "UserType")]
    pub user_type: i64,
}

jsonpath_model!(UserHomepageResponse {
    base_info: "$.Info.BaseInfo" => UserHomepageBaseInfo,
    singer: "$.Info.Singer" => Value,
    is_followed: "$.Info.IsFollowed" => i64,
    tab_detail: "$.TabDetail" => Value,
});

/// VIP 信息响应中的会员身份明细块.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct VipIdentity {
    pub vip: i64,
    #[serde(alias = "HugeVip")]
    pub huge_vip: i64,
    #[serde(alias = "HugeVipStart")]
    pub huge_vip_start: String,
    #[serde(alias = "HugeVipEnd")]
    pub huge_vip_end: String,
    #[serde(alias = "yearflag")]
    pub year_flag: i64,
    #[serde(alias = "HugeYearFlag")]
    pub huge_year_flag: i64,
    pub twelve: i64,
    #[serde(alias = "twelveStart")]
    pub twelve_start: String,
    #[serde(alias = "twelveEnd")]
    pub twelve_end: String,
    #[serde(alias = "ChildVip")]
    pub child_vip: i64,
    #[serde(alias = "ExpVip")]
    pub exp_vip: i64,
    #[serde(alias = "GroupVipFlag")]
    pub group_vip_flag: i64,
    #[serde(alias = "GroupVipStart")]
    pub group_vip_start: String,
    #[serde(alias = "GroupVipEnd")]
    pub group_vip_end: String,
    #[serde(alias = "CPLoverFlag")]
    pub cp_lover_flag: i64,
    #[serde(alias = "CPLoverStart")]
    pub cp_lover_start: String,
    #[serde(alias = "CPLoverEnd")]
    pub cp_lover_end: String,
    #[serde(alias = "AdVipFlag")]
    pub ad_vip_flag: i64,
    pub eight: i64,
    #[serde(alias = "eightStart")]
    pub eight_start: String,
    #[serde(alias = "eightEnd")]
    pub eight_end: String,
    pub level: i64,
    #[serde(alias = "nextlevel")]
    pub next_level: i64,
    pub icon: String,
    #[serde(alias = "purchaseUrl")]
    pub purchase_url: String,
}

/// VIP 信息响应中的用户权益摘要块.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct VipUserInfo {
    #[serde(alias = "buy_url", alias = "buyurl")]
    pub buy_url: String,
    #[serde(alias = "my_vip_url", alias = "myvipurl")]
    pub my_vip_url: String,
    pub score: i64,
    pub expire: i64,
    #[serde(alias = "music_level")]
    pub music_level: i64,
}

/// VIP 信息视图响应.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct UserVipInfoResponse {
    #[serde(alias = "auto_down", alias = "autoDown", alias = "autodown")]
    pub auto_down: i64,
    #[serde(alias = "canRenew")]
    pub can_renew: i64,
    #[serde(alias = "max_dir_num", alias = "maxDirNum", alias = "maxdirnum")]
    pub max_dir_num: i64,
    #[serde(alias = "max_song_num", alias = "maxSongNum", alias = "maxsongnum")]
    pub max_song_num: i64,
    #[serde(alias = "song_limit_msg", alias = "songLimitMsg")]
    pub song_limit_msg: String,
    pub svip: i64,
    pub star: i64,
    #[serde(alias = "starstart")]
    pub star_start: String,
    #[serde(alias = "starend")]
    pub star_end: String,
    pub ystar: i64,
    #[serde(alias = "ystarstart")]
    pub ystar_start: String,
    #[serde(alias = "ystarend")]
    pub ystar_end: String,
    pub identity: VipIdentity,
    pub userinfo: VipUserInfo,
}

/// 关注或粉丝列表中的单个用户条目.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct RelationUser {
    #[serde(alias = "MID")]
    pub mid: String,
    #[serde(alias = "EncUin")]
    pub enc_uin: String,
    #[serde(alias = "Name")]
    pub name: String,
    #[serde(alias = "Desc")]
    pub desc: String,
    #[serde(alias = "AvatarUrl")]
    pub avatar_url: String,
    #[serde(alias = "FanNum")]
    pub fan_num: i64,
    #[serde(alias = "IsFollow")]
    pub is_follow: bool,
}

jsonpath_model!(UserRelationListResponse {
    total: "$.Total" => i64,
    users: "$.List[*]" => Vec<RelationUser>,
    has_more: "$.HasMore" => bool,
    last_pos: "$.LastPos" => String,
    msg: "$.Msg" => String,
    lock_flag: "$.LockFlag" => i64,
    lock_msg: "$.LockMsg" => String,
});

/// 好友列表中的单个好友条目.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct FriendEntry {
    #[serde(alias = "EncryptUin")]
    pub encrypt_uin: String,
    #[serde(alias = "UserName")]
    pub user_name: String,
    #[serde(alias = "AvatarUrl")]
    pub avatar_url: String,
    #[serde(alias = "IsFollow")]
    pub is_follow: bool,
}

/// 好友列表视图响应.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct UserFriendListResponse {
    #[serde(alias = "Friends")]
    pub friends: Vec<FriendEntry>,
    #[serde(alias = "HasMore")]
    pub has_more: bool,
}

/// 用户收藏 MV 列表中的单个条目.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct UserFavMvItem {
    #[serde(flatten)]
    pub base: MV,
    #[serde(alias = "picUrl")]
    pub picurl: String,
    pub playcount: i64,
    pub publish_date: i64,
    #[serde(alias = "singerId")]
    pub singer_id: i64,
    #[serde(alias = "singerMid")]
    pub singer_mid: String,
    #[serde(alias = "singerName")]
    pub singer_name: String,
    pub status: i64,
}

/// 用户收藏 MV 列表视图响应.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct UserFavMvResponse {
    pub code: i64,
    #[serde(alias = "subCode", alias = "subcode")]
    pub sub_code: i64,
    pub msg: String,
    #[serde(alias = "mvlist")]
    pub mv_list: Vec<UserFavMvItem>,
}

/// 不喜欢列表中的单个条目.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct DislikeItem {
    #[serde(alias = "ID")]
    pub id: String,
    #[serde(alias = "Name")]
    pub name: String,
    #[serde(alias = "Img")]
    pub img: String,
    #[serde(alias = "IdType")]
    pub id_type: i64,
    #[serde(alias = "Time")]
    pub time: i64,
}

/// GetDislikeList 响应数据.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct DislikeListData {
    #[serde(alias = "Retcode")]
    pub retcode: i64,
    #[serde(alias = "Msg")]
    pub msg: String,
    #[serde(alias = "Singers")]
    pub singers: Vec<DislikeItem>,
    #[serde(alias = "Songs")]
    pub songs: Vec<DislikeItem>,
    #[serde(alias = "Styles")]
    pub styles: Vec<DislikeItem>,
    #[serde(alias = "Page")]
    pub page: i64,
    #[serde(alias = "Token")]
    pub token: String,
}
