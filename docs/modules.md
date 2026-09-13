# 模块 API 参考

> 所有方法均为 `async fn`，返回 `Result<T, QmError>`。
> 需要登录的接口在未登录时会返回 `QmError::CredentialInvalid`。
>
> **Raw 透传**：以 `raw_` 前缀命名的方法用于桌面客户端协议互操作，直接
> 透传未验证的 `serde_json::Value` 参数与响应，仅作为底层能力；稳定业务代码
> 应优先使用类型化方法，自行封装 DTO。

## 通用能力

### Web 目录与推荐

以下方法在库内固定 Web 请求信封，调用方不需要构造 QQ URL、module/method 或原始响应层级。
它们不等同于同名 Android 服务；原有方法继续保留。

| 入口 | 参数 | 返回 |
| --- | --- | --- |
| `client.discovery.categories` | 无 | 分类卡片，含编码后的 `area_key` |
| `client.discovery.podcasts` | 无 | 播客卡片 |
| `client.discovery.new_mvs` | offset、limit（1–100） | MV 卡片，时长单位为秒 |
| `client.discovery.featured` | 无 | 焦点卡片 |
| `client.discovery.area` | `area_key` | 区域标题、楼层与类型化卡片 |
| `client.top.get_web_detail` | top_id、offset、limit（1–100）、period | 榜单元数据与歌曲 |
| `client.recommend.get_web_home_feed` | page（从 1 开始）、seen_shelves、cached_shelf_ids、显式 Credential | 个性化 Feed 楼层 |
| `client.recommend.get_web_songlists` | offset、limit（1–100） | 公共推荐歌单摘要 |
| `client.recommend.get_web_newsongs` | kind（上游新歌类型） | 公共新歌列表 |
| `client.search.search_songlists` | keyword、num、page（从 1 开始） | Android 歌单搜索分页，兼容数字/字符串字段 |

个性化 Web Feed 必须显式提供凭据，不修改或继承 Client 的默认账户。上述公共 Web 方法
显式使用匿名身份，即使 Client 默认平台为 Android 或已经登录也不带入该账户。
CGI 的空 Cookie header 是隔离信号，自定义 transport 不得移除它再从 Cookie jar 补登录信息。

Web Discovery 与推荐区分合法空列表、未知卡片类型和畸形数据：未知类型保留为 `Other`；
缺失列表或畸形条目返回错误，不转为空成功。歌单搜索兼容方法允许跳过单个非对象条目，
但保留上游 `total`，它不等于当前页有效条目数量。
封面地址属于元数据，不代表下载许可；宿主仍需校验 origin、重定向和资源大小。

### 批量 CGI 请求

多个请求合并为一次 `req_0..req_N` 调用，减少网络往返（对应参考库 `client.gather`）：

```rust
let reqs: Vec<(&str, &str, serde_json::Value)> = mids.iter()
    .map(|m| ("music.trackInfo.UniformRuleCtrl", "CgiGetTrackInfo",
             json!({"ctx": 0, "client": 1, "types": [0], "modify_stamp": [0], "mids": [m]})))
    .collect();
// 每个子请求返回固定形状 CgiReply { code, data }, 不因单个子请求的业务错误码整体失败
let replies = client.request_cgi_batch(&reqs, &Default::default()).await?; // Vec<CgiReply<Value>>
// 或要求全部成功并直接反序列化: let typed: Vec<QuerySongResponse> = client.cgi_batch(&reqs, &Default::default()).await?;
```

### CGI 响应契约（`CgiReply`）

transport 层（`Client::request_cgi` / `ApiContext::request_cgi`）**始终**返回固定形状
`CgiReply { code, data }`，不解释业务错误码（已移除旧的 `allow_error_codes` /
`parse_on_allow` 多态返回）：

```rust
use qqmusic_api::CgiReply;

// 普通接口: 成功才返回 data, code != 0 时映射为错误
let data: serde_json::Value = reply.require_success()?;

// 需要解释特殊状态码的接口 (如登录): 直接读取 code
if reply.code == 20276 { /* 需要验证码 */ }
```

`CgiReply::require_success_allowing(&[10007])` 允许透传"携带有效数据的业务状态码"
（如曲谱不存在 10007）。

### 代理

创建客户端时指定 HTTP 代理（`Client::new_with_proxy`）：

```rust
let client = Client::new_with_proxy(None, None, Some("http://127.0.0.1:7890"))?;
```

注入自定义 HTTP 传输（超时 / allowlist / 取消由实现自行保证）：

```rust
use std::sync::Arc;
use qqmusic_api::{ApiTransport, Client};

let transport: Arc<dyn ApiTransport> = Arc::new(/* 你的实现 */);
let client = Client::new_with_transport(None, None, transport);
```

> 说明：当前仅支持 HTTP 代理；未暴露 DNS resolver/resolve 配置。MQTT 手机端
> 扫码登录使用独立的 WebSocket 连接，不经过该代理，也不走 `ApiTransport`。

### 限流

客户端内置令牌桶限流（默认 10 请求/秒、突发容量 50，与参考库一致），
避免触发服务端 `2001` 限流。可通过 `Client::context().limiter` 调整。

### 设备指纹持久化

Android 平台的 QIMEI（设备身份）可持久化，避免每次重启重新申请：

```rust
client.save_device(std::path::Path::new("device.json"))?;
// 下次启动:
let client = Client::new(None, None)?;
client.load_device(std::path::Path::new("device.json"))?;
```

> 说明：Android session 是**账号运行态**（按 `musicid` 缓存在内存中），不属于
> 设备身份，不随 `save_device` 持久化；每次启动会按需重新申请。

### 凭证管理（多账号 + 自动刷新）

`CredentialStore` 提供多账号管理与过期自动刷新，持久化通过可插拔的
`CredentialPersist` 后端委托给宿主：

- 默认 `FileCredentialPersist` 为**明文 JSON，仅限开发环境**；
- 生产环境请实现安全后端（系统 Keychain / 加密文件），再以
  `from_backend` / `with_backend` 注入；
- `Credential` 的 `Debug` 已对令牌字段做 redaction，不会泄漏进日志。

```rust
use qqmusic_api::{CredentialPersist, CredentialStore, FileCredentialPersist};

// 开发环境: 明文 JSON 文件后端
let store = CredentialStore::load(Path::new("accounts.json"))
    .unwrap_or_else(|_| CredentialStore::new())
    .with_path(Path::new("accounts.json"));

// 生产环境: 宿主实现安全后端
struct SecureBackend { /* ... */ }
impl CredentialPersist for SecureBackend {
    fn load(&self) -> qqmusic_api::Result<Option<String>> { /* 系统安全存储读取 */ }
    fn save(&self, data: &str) -> qqmusic_api::Result<()> { /* 加密落盘 */ }
}
let store = CredentialStore::from_backend(SecureBackend { /* ... */ })?;

// 登录成功后添加账号并持久化
store.add(credential)?;                 // 空库自动设为当前账号
store.set_current(musicid)?;            // 切换账号
store.apply_current(&client)?;          // 应用到客户端
store.ensure_current(&client).await?;   // 过期时自动 refresh_credential
```

### QMC 加密音质解密

`qqmusic_api::qmc` 提供 QMCDecode QMCv2 Map/分段 RC4 解密与 EKey 还原，详见 [下载歌曲](./download.md)。

### 歌词解析（LRC / QRC）

`qqmusic_api::lyric_parser` 提供结构化歌词，详见 [下载歌曲](./download.md)。

### HTTP 服务

参考库的 `web/` 目录提供了 HTTP 服务封装；本仓库以示例形式提供
`examples/web_server.rs`（基于 axum）：

```bash
cargo run --example web_server
curl 'http://127.0.0.1:3000/search?keyword=周杰伦&num=3'
curl 'http://127.0.0.1:3000/song/url?mid=0039MnYb0qxYhV'
```

## 分页

支持连续翻页：`Pager` 通用分页器 + `page` / `offset` 便捷策略。

```rust
use qqmusic_api::{Pager, offset, page};
use serde_json::json;

// offset 策略示例: 拉取排行榜歌曲
let mut pager = offset(
    "offset", "num",
    json!({"topId": tid, "offset": 0, "num": 10}),
    {
        let client = client.clone();
        move |params| {
            let client = client.clone();
            Box::pin(async move {
                Ok::<_, qqmusic_api::QmError>(
                    client.top.get_detail(
                        params["topId"].as_i64().unwrap(),
                        params["num"].as_i64().unwrap(),
                        params["offset"].as_i64().unwrap() / params["num"].as_i64().unwrap() + 1,
                        true,
                    ).await?
                )
            })
        }
    },
    |resp: &qqmusic_api::models::top::TopDetailResponse| resp.songs.len() >= 10,
).with_limit(10);   // 可选: 限制最大页数

let songs = pager.collect_items(|r| r.songs.clone()).await?;   // 跨页展开条目
// 或: let pages = pager.collect().await?;                       // 收集各页响应
```

## song（歌曲）

```rust
// 批量获取歌曲信息
song.query_song(&[SongQueryInfo::by_id(id)]) -> QuerySongResponse
song.query_song(&[SongQueryInfo::by_mid(mid)]) -> QuerySongResponse

// 获取播放链接
song.get_song_urls(&[SongFileInfo], &SongFileType, credential: Option<&Credential>) -> GetSongUrlsResponse
song.get_cdn_dispatch() -> GetCdnDispatchResponse

// 详情 / 相似 / 标签
song.get_detail(value: &str /* id 或 mid */) -> GetSongDetailResponse
song.get_similar_song(songid) -> GetSimilarSongResponse
song.get_labels(songid) -> GetSongLabelsResponse
song.get_other_version(value) -> GetOtherVersionResponse
song.get_producer(value) -> GetProducerResponse

// 关联
song.get_related_songlist(songid, last: &[i64]) -> GetRelatedSonglistResponse
song.get_related_mv(songid, last_mvid: Option<&str>) -> GetRelatedMvResponse

// 曲谱
song.get_sheet(mid, SheetType::User /* 或 EngineAi / ChongChong */) -> GetSheetResponse
song.has_sheet(mid) -> HasSheetMusicResponse

// 收藏数
song.get_fav_num(&[songid]) -> GetFavNumResponse

// 桌面客户端互操作 — Raw 透传 (schema 未 live 验证)
song.raw_is_song_fan_by_mid(param, credential) -> Value   // 检查是否已收藏 (需要登录)
song.raw_get_fav_songlist(param, credential) -> Value      // 收藏歌曲列表 (需要登录)
song.raw_get_url_vkey(param) -> Value                      // 桌面端下载链接 (GetUrl)
```

文件类型（`modules::song`）：
- `SongFileType::{Mp3_128, Mp3_320, Flac, Master, Atmos2, Atmos51, Atmos71, AtmosDb, Nac, Ogg640, Ogg320, Ogg192, Ogg96, Acc192, Acc96, Acc48, DtsX}`
- `EncryptedSongFileType::{Flac, Master, Vinyl, Atmos2, ...}`（走 `GetEVkey`）
- `SpecialSongFileType::{Try, Accom, Piano, Guzheng, ...}`
- `RingSongFileType::{Ring128, Ring96, Ring48}`

`SongFileInfo` 构建器：

```rust
SongFileInfo::new(mid).with_type(SongFileType::Flac).with_song_type(0).with_media_mid(media_mid)
```

## search（搜索）

```rust
search.search_by_type(keyword, SearchType, num, page, selectors: &[SearchSelector], searchid, highlight)
    -> SearchByTypeResponse
// 字段: song/singer/album/songlist/mv/user/audio_alum, nextpage, total_num 等

search.general_search(keyword, page, num, searchid, page_start, highlight) -> GeneralSearchResponse
search.get_hotkey() -> HotkeyResponse
search.complete(keyword) -> CompleteResponse
search.quick_search(keyword) -> QuickSearchResponse
```

`SearchType` 枚举：`Song(0)` `Singer(1)` `Album(2)` `Songlist(3)` `Mv(4)` `Lyric(7)` `User(8)` `Ringtone(10)` `AudioAlbum(15)` `Audio(18)`。

## singer（歌手）

```rust
singer.get_singer_list(area, sex, genre) -> SingerTypeListResponse
singer.get_singer_list_index(area, sex, genre, index, page, num) -> SingerIndexPageResponse
singer.get_info(mid) -> HomepageHeaderResponse        // Android
singer.get_tab_detail(mid, TabType, page, num) -> HomepageTabDetailResponse
singer.get_desc(&[mids], ex_singer, wiki_singer, group_singer, pic, photos) -> SingerDetailResponse
singer.get_similar(mid, number) -> SimilarSingerResponse
singer.get_songs_list(mid, num, page) -> SingerSongListResponse
singer.get_album_list(mid, num, page) -> SingerAlbumListResponse
singer.get_mv_list(mid, num, page) -> SingerMvListResponse
```

枚举：`AreaType` `GenreType` `SexType` `IndexType` `TabType`（位于 `qqmusic_api::modules::singer`）。

## album（专辑）

```rust
album.get_detail(value /* id 或 mid */) -> GetAlbumDetailResponse
album.get_song(value, num, page) -> GetAlbumSongResponse
album.get_new_album(area, num, page) -> GetNewAlbumResponse
```

> ⚠️ `album.fav_album` / `album.del_fav_album` 属于 **Experimental** 写接口
> （`AlbumFavWrite / FavAlbum / CancelFavAlbum`，语义未 live 验证），
> 默认不编译，需启用 `--features experimental`。详见 [experimental](./experimental.md)。

## lyric（歌词）

```rust
lyric.get_lyric(value, song_type, qrc, trans, roma, singing_annotations) -> GetLyricResponse
// 钉 Web + 未签名 musicu.fcg；同时带 songMID/songMid（数字 id 则 songID）
// crypt 默认省略；需要加密字段时用 get_lyric_with_crypt(..., Some(1))
// GetLyricResponse::parse 对加密字段就地解密（不是下游 LyricDocument）
lyric.get_lyric_with_crypt(..., crypt: Option<i64>) -> GetLyricResponse
lyric.get_singing_annotations_info(songid) -> GetSingingAnnotationsInfoResponse
lyric.get_multi_style_trans_lyric(songid) -> BatchGetMultiStyleTransLyricResponse
lyric.is_ai_dict_exists(songid) -> IsAIDictExistsResponse
lyric.get_ai_dict(songid) -> GetAIDictResponse
```

## mv（MV）

```rust
mv.get_detail(&[vid]) -> GetMvDetailResponse
mv.get_mv_urls(&[vid]) -> GetMvUrlsResponse
mv.get_mv_list(area, version, order, num, page) -> GetMvListResponse
```

## top（排行榜）

```rust
top.get_category() -> TopCategoryResponse
top.get_detail(top_id, num, page, tag) -> TopDetailResponse
```

## songlist（歌单）

```rust
songlist.get_detail(songlist_id, dirid, num, page, onlysong, tag, userinfo) -> GetSonglistDetailResponse
songlist.create(dirname, credential) -> CreateDeleteSonglistResp          // 需要登录
songlist.delete(dirid, credential) -> CreateDeleteSonglistResp            // 需要登录
songlist.add_songs(dirid, &[(song_id, song_type)], tid, credential) -> bool
songlist.del_songs(dirid, &[(song_id, song_type)], tid, credential) -> bool
songlist.like_song(&[(song_id, song_type)], credential) -> bool   // 添加到"我喜欢"(dirid=201)
songlist.unlike_song(&[(song_id, song_type)], credential) -> bool

// 桌面客户端互操作 — Raw 透传 (schema 未 live 验证)
songlist.raw_get_uniform_song_detail(param, credential) -> Value       // 歌单歌曲详情 (dirId=201 为"我喜欢")
songlist.raw_get_song_detail_info_list_by_dirid(param, credential) -> Value
songlist.raw_is_playlist_fan(param, credential) -> Value               // 检查歌单是否已收藏 (需要登录)
songlist.raw_seq_songlist(param, credential) -> bool                   // 歌单排序 (需要登录)
songlist.raw_cancel_fav_audio(param, credential) -> Value              // 取消收藏长音频 (需要登录)
```

## comment（评论）

```rust
comment.get_comment_count(biz_id, CommentBizType, biz_sub_type) -> CommentCountResponse
comment.get_hot_comments(biz_id, page_num, page_size, last_seq, CommentBizType, sub_type) -> CommentListResponse
comment.get_new_comments(biz_id, page_num, page_size, last_seq, CommentBizType, sub_type) -> CommentListResponse
comment.get_recommend_comments(biz_id, page_num, page_size, last_seq, CommentBizType, sub_type) -> CommentListResponse
comment.get_moment_comments(biz_id, page_size, last_pos, CommentBizType, sub_type) -> MomentCommentResponse
comment.add_comment(biz_id, content, reply_cmt_id, CommentBizType, sub_type, credential) -> AddCommentResponse
comment.delete_comment(cm_id, credential) -> bool

// 桌面客户端互操作 — Raw 透传 (schema 未 live 验证)
comment.raw_get_reply_comments(param, credential) -> Value   // 回复列表
comment.raw_update_hot_comment(param, credential) -> Value   // 更新热评状态 (需要登录)
```

`CommentBizType`：`Song(0)` `Album(1)` `Mv(2)` `Songlist(3)` `Singer(4)` `Video(5)` `Audio(6)` `AudioAlbum(7)`。
歌曲类型默认子类型 `biz_sub_type = 2`。

## recommend（推荐）

```rust
recommend.get_home_feed(page, direction, s_num, v_cache) -> RecommendFeedCardResponse
recommend.get_guess_recommend(credential) -> GuessRecommendResponse
recommend.get_guess_recommend_with_request(request, credential) -> GuessRecommendResponse
recommend.get_radar_recommend(page) -> RadarRecommendResponse
recommend.get_radar_recommend_with_request(request, credential) -> RadarRecommendResponse
recommend.get_recommend_songlist(page, num) -> RecommendSonglistResponse
recommend.get_recommend_newsong(type) -> RecommendNewSongResponse
```

## user（用户）

```rust
user.get_homepage(euin, credential) -> UserHomepageResponse      // 未登录时自动使用占位凭证
user.get_vip_info(credential) -> UserVipInfoResponse             // 需要登录
user.get_follow_singers(euin, page, num, credential) -> UserRelationListResponse
user.get_fans(euin, page, num, credential) -> UserRelationListResponse
user.get_friend(page, num, credential) -> UserFriendListResponse
user.get_follow_user(euin, page, num, credential) -> UserRelationListResponse
user.get_created_songlist(uin, credential) -> UserCreatedSonglistResponse
user.get_fav_song(euin, page, num, credential) -> GetSonglistDetailResponse
user.get_fav_songlist(euin, page, num, credential) -> UserFavSonglistResponse
user.fav_songlist(songlist_id, credential) -> bool               // 需要登录
user.unfav_songlist(songlist_id, credential) -> bool             // 需要登录
user.get_fav_album(euin, page, num, credential) -> UserFavAlbumResponse
user.get_fav_mv(euin, page, num, credential) -> UserFavMvResponse
user.get_music_gene(euin, credential) -> UserMusicGeneResponse
user.get_dislike_list(cmd, page, lastid, credential) -> DislikeListData
user.add_dislike(DislikeIdType::Songs /* 或 Singers / Styles */, &[values], credential) -> bool
user.cancel_dislike(DislikeIdType, &[values], credential) -> bool
user.cancel_all_dislike_song(credential) -> bool

// 桌面客户端互操作 — Raw 透传 (schema 未 live 验证)
user.raw_get_user_vip_info(param, credential) -> Value   // 桌面端 VIP 查询 (SRFVipQuery_V2)
user.raw_get_user_base_info(param, credential) -> Value  // 用户基础信息 (get_user_baseinfo_v2)
user.raw_get_favor_list(param, credential) -> Value      // 收藏的电台列表
user.raw_get_collect_album_list(param, credential) -> Value // 收藏专辑列表 (GetAlbumFavInfo)
```

> ⚠️ `user.focus_singer` / `user.fav_mv` 属于 **Experimental** 写接口
> （`cgi_concern_user_v2` 的 `opertype` 正反值、`AddDelFavMV` 的 cmdtype 语义
> 均未 live 验证），默认不编译，需启用 `--features experimental`。
> 详见 [experimental](./experimental.md)。

## account（显式账户读写边界）

`account::read_page` 使用 `AccountRead` 选择收藏歌曲、自建/收藏歌单列表、歌单曲目或最近播放，
返回校验后的 `AccountPage`。`account::write` 使用 `AccountWrite` 选择收藏歌曲、
歌单增删改、歌曲增删及歌单收藏；不再公开 `write_legacy(module, method, param)`。

`AccountRead::PlaylistTracks` 严格校验返回的歌单 TID。自建歌单的 DissInfo 可能仅返回
目录 ID，此时使用 `OwnedPlaylistTracks { tid, dir_id }`；`dir_id` 必须取自当前账户已验证的
自建歌单列表，不能从本次待验证响应中推导。目录别名必须一致，显式 TID 冲突仍然拒绝。

`OwnedPlaylists` 按 `offset/limit` 生成包含结束行的列表区间；`CollectedPlaylists`
从本次显式凭据取加密账户标识。翻页按原始响应行数推进，不按宿主筛选后的条目数推进。
响应偏移、页大小、总数和 `hasmore`/`bFinish` 必须一致；非空无总数页可继续，明确结束标志
可终止未知总数列表。缺失列表、重复第一页、零进展和矛盾结束标志不会被伪装成空列表成功。

两者都要求传入单次请求的 `Credential` 和 `CancellationToken`，不继承 Client 的默认账户。
歌单收藏还要求该凭据的 `encrypt_uin` 非空；不得用普通 UIN 或其他账户的身份代替。
写入使用固定 Web 请求契约，即使 Client 默认平台是 Android 也不先创建 Android session。

写请求不自动重放，返回的 `CgiReply` 保留业务码供宿主解释。网络错误、响应格式错误、
取消都不能证明远端没有产生副作用；宿主应按账户代次和写入语义决定是否执行安全读取对账，
不能把重新发送写操作当作通用恢复方式。取消之后不发布成功结果，也不撤销已发生的远端操作。

## artwork（图片）

`artwork::album(mid)` 与 `artwork::from_url(url)` 返回 URL 和已知尺寸变体；
`artwork::download(&client, url, cancellation)` 返回 `DownloadedArtwork`，不接受账户凭据。
下载只接受已验证 HTTPS 图片 URL，发送显式空 Cookie，不继承 Client 的登录账户；
禁止重定向并校验最终 URL、成功状态和 `image/*` Content-Type。

默认上限为 5 MiB。`HttpOptions::max_response_bytes` 会传递到 `TransportRequest`，
默认 transport 在解码后的响应块累计超过上限前停止，不把大响应完整读入内存再拒绝。
自定义 transport 必须遵守该字段；图片 API 也会复核返回体大小。未设置该字段的既有 API
保留原来的响应策略。取消及不合格响应不会返回图片成功结果；宿主负责缓存及 UI 映射。

## login（登录）

```rust
login.get_qrcode(QRLoginType) -> QR                       // QQ / WX / Mobile
login.check_qrcode(&QR) -> QRLoginResult
login.send_authcode(phone, is_encrypted, country_code) -> PhoneAuthCodeResult
login.phone_authorize(phone, is_encrypted, auth_code) -> Credential
login.check_expired(credential) -> bool
login.refresh_credential(credential) -> Credential
login.logout(credential) -> ()
```

详见 [登录](./login.md)。

## helper（上传辅助）

```rust
helper.init_upload(bus_id, &[InitUploadFileDict], credential) -> InitUploadResponse   // 需要登录 + 签名
helper.finish_upload(bus_id, &[FinishUploadResultDict], credential) -> FinishUploadResponse
helper.raw_query_update(cv) -> Value   // 客户端更新检查 (官方桌面端, Raw 透传)

// 完整 COS 上传流程 (helper_utils::UploadFileSession)
use qqmusic_api::UploadFileSession;
use std::path::PathBuf;

let session = UploadFileSession::new(client.helper.clone(), "songlist");
let objects = session.upload(&[PathBuf::from("cover.png")]).await?;
println!("URL: {}", objects[0].url.url);
```

`InitUploadFileDict { file_sha1, file_name, file_size }` 可由 `UploadFileSession::get_file_info(path)` 计算。

## 登录会话工具

```rust
use qqmusic_api::{PhoneLoginSession, QRCodeLoginSession, QRLoginType};

// 手机验证码会话
let mut phone = PhoneLoginSession::new(client.login.clone(), "13800138000", false, 86);
phone.send_authcode().await?;
let credential = phone.authorize("123456").await?;

// 二维码登录会话
let mut session = QRCodeLoginSession::new(client.login.clone(), QRLoginType::Qq);
let qr = session.get_qrcode().await?;              // 拿到二维码给用户扫
// GUI 实时事件: session.next_event().await?
// 便利收集: session.iter_events().await?  (等到结束才返回 Vec)
let credential = session.wait_qrcode_login().await?;
client.set_credential(credential);
```

## private_message（私信）

> 全部接口固定使用 Android 平台，需要登录。已提供类型化返回模型。

```rust
private_message.get_sessions(...) -> PrivateSessionListResponse
private_message.delete_session(session_id, super_msg_flag, credential) -> PrivateOperationResponse
private_message.get_messages(...) -> PrivateMessageListResponse
private_message.send_message(...) -> PrivateSendMessageResponse
private_message.delete_message(session_id, msg_id, super_msg_flag, credential) -> PrivateOperationResponse
private_message.clear_session(session_id, super_msg_flag, credential) -> PrivateOperationResponse
private_message.set_config(config_type, config_value, credential) -> PrivateOperationResponse
private_message.get_config(config_type, config_value, credential) -> PrivateConfigResponse
private_message.get_musician_message_card(enc_uin, credential) -> PrivateMusicianCardResponse
private_message.report_card_message_action(...) -> PrivateOperationResponse
private_message.get_chat_entries(scenes, ...) -> PrivateChatEntriesResponse
private_message.get_media_message_details(session_id, msg_ids, credential) -> PrivateMediaMessageDetailsResponse
private_message.mark_all_messages_read(cmd_flag, encrypt_uin, credential) -> PrivateOperationResponse
private_message.get_safety_hint(enc_uin, close, credential) -> PrivateSafetyHintResponse
private_message.raw_get_friendship_badge(target_enc_uin, credential) -> Value   // Raw 透传
```
