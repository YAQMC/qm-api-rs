# QQMusicApi (Rust)

> 纯 Rust 实现的 QQ 音乐异步 API 客户端。

移植自 [L-1124/QQMusicApi](https://github.com/L-1124/QQMusicApi) (Python)，
并参考官方桌面客户端 `qqmusic_1.1.8-1.asar` 解包后的源码，补充了
签名接口（`musics.fcg`）、平台参数（`ct`/`cv`/`comm`）、登录流程等细节。

> [!NOTE]
> **音乐平台不易，请尊重版权，支持正版。**

## 特性

- 🎵 涵盖常见 API：歌曲 / 搜索 / 歌手 / 专辑 / 歌词 / MV / 排行榜 / 歌单 / 评论 / 推荐 / 用户 / 登录 / 私信 / 上传辅助
- 🚀 调用简便，函数命名易懂，方法均带详细文档
- ⚡ 完全异步（`tokio` + 可注入 `ApiTransport`，默认 reqwest 0.12）
- 🔐 内置签名算法（`zzc_sign`）与 QRC 歌词 3DES 解密
- 📱 支持 Android / Desktop / Web 三种平台请求
- 🔁 连续翻页（`Pager`）、批量 CGI 请求（`request_cgi_batch`）、内置限流
- 🔓 内置 **QMC 加密音质解密**（QMCv1/QMCv2 Map/RC4 + EKey TEA）
- 📝 **LRC/QRC 歌词解析器**（`lyric_parser`）
- 👥 **多账号凭证管理**（`CredentialStore` 持久化 + 自动刷新）
- 🌐 **代理支持**（`Client::new_with_proxy`）
- 🧰 附带 axum HTTP 服务示例（`examples/web_server.rs`）与设备指纹持久化

## 安装

在 `Cargo.toml` 中加入依赖：

```toml
[dependencies]
qqmusic-api = { git = "https://github.com/you/qqmusic-api-rs" }
tokio = { version = "1", features = ["full"] }
```

## 快速使用

```rust
use qqmusic_api::{Client, SearchType};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(None, None)?;
    let result = client
        .search
        .search_by_type("周杰伦", SearchType::Song, 5, 1, &[], None, true)
        .await?;
    println!("单曲结果数量: {}", result.song.len());
    Ok(())
}
```

更多示例见 [`examples/demo.rs`](./examples/demo.rs)，运行：

```bash
cargo run --example demo
```

## 文档

- [快速开始](./docs/quickstart.md)
- [登录](./docs/login.md)
- [下载歌曲（获取播放链接）](./docs/download.md)
- [错误处理](./docs/error-handling.md)
- [架构与签名](./docs/architecture.md)
- [模块 API 参考](./docs/modules.md)

## 模块概览

| 模块 | 说明 | 主要方法 |
| --- | --- | --- |
| `client.song` | 歌曲 | `get_song_urls`, `get_detail`, `get_similar_song`, `get_sheet(SheetType)` 等 |
| `client.search` | 搜索 | `search_by_type`, `general_search`, `get_hotkey`, `complete`, `quick_search` |
| `client.singer` | 歌手 | `get_info`, `get_songs_list`, `get_album_list`, `get_similar` 等 |
| `client.album` | 专辑 | `get_detail`, `get_song`, `get_new_album` |
| `client.lyric` | 歌词 | `get_lyric`（自动 QRC 解密）, `get_ai_dict` 等 |
| `client.mv` | MV | `get_detail`, `get_mv_urls`, `get_mv_list` |
| `client.top` | 排行榜 | `get_category`, `get_detail` |
| `client.songlist` | 歌单 | `get_detail`, `create`, `delete`, `add_songs`, `like_song` 等 |
| `client.comment` | 评论 | `get_hot_comments`, `get_new_comments`, `add_comment` 等 |
| `client.recommend` | 推荐 | `get_home_feed`, `get_guess_recommend`, `get_recommend_songlist` |
| `client.user` | 用户 | `get_homepage`, `get_vip_info`, `get_created_songlist`, `fav_songlist`, `add_dislike(DislikeIdType)` 等 |
| `client.login` | 登录 | `get_qrcode`, `check_qrcode`, `send_authcode`, `phone_authorize`, `refresh_credential` |
| `client.helper` | 上传辅助 | `init_upload`, `finish_upload`, `UploadFileSession` |
| `client.private_message` | 私信 | `get_sessions`, `get_messages`, `send_message` 等 |

通用能力：`Pager` / `page` / `offset` 连续翻页，`request_cgi_batch` 批量请求，
内置令牌桶限流，`PhoneLoginSession`、`QRCodeLoginSession` 登录会话工具，
`Client::save_device` / `load_device` 设备指纹持久化，axum HTTP 服务示例
（`examples/web_server.rs`）。

## 覆盖范围

接口覆盖情况对照参考来源：

- **Python 参考库 (L-1124/QQMusicApi)**：全部 14 个模块的业务方法均已移植，含
  `helper_utils.UploadFileSession` 与 `login_utils.PhoneLoginSession / QRCodeLoginSession`。
- **官方桌面客户端 (Electron ASAR)**：补充了桌面端专用接口，包括
  `IsSongFanByMid / GetFavSonglist / GetUrl / GetUniformSongDetailInfo /
  GetSongDetailInfoListByDirId / IsPlaylistFan / SeqSonglist / do_favor /
  GetReplyCommentList / UpdateHotComment / SRFVipQuery_V2 / get_user_baseinfo_v2 /
  get_favor_list / GetAlbumFavInfo / QueryUpdate`（这些未经 live 验证的接口以
  `raw_*` 前缀提供，未验证的写操作另在 feature `experimental` 下）。
- **登录**：QQ / 微信二维码、手机验证码、手机客户端二维码（MQTT 5.0 over WebSocket
  推送，`src/mqtt.rs`）全部支持。
- **第三方客户端能力**：QMC 加密音质解密（`qmc`）、LRC/QRC 歌词解析（`lyric_parser`）、
  多账号凭证管理（`credential_store`，可插拔安全后端）、代理支持（`new_with_proxy`）、
  **VIP 音质**（`MediaSource` 来源描述 + `song.media_source`，播放器直接消费；
  `song.available_qualities` / `get_best_song_url` / `media::download_quality`
  + `user.is_vip`，配合你账号已购的 VIP 权益）。

> 影响排序补充进度：
> ① 加密音质解密 ✅（QMCv1/v2，含参考实现测试向量 + EKey TEA 往返）
> ③ 凭证自动刷新与多账号 ✅（`CredentialStore`，可插拔安全后端，Debug 已 redaction）
> ④ 歌词结构化解析 ✅（`lyric_parser`：LRC + QRC 逐字）
> ⑤ 桌面端完整签名 ✅（`zzc_sign` 与参考逐字节一致，无需 chaos VM）
> ⑥ 代理配置 ✅（`new_with_proxy`，仅 HTTP 代理；DNS 解析器未暴露，
> MQTT 为独立 WebSocket 连接也不走该代理）
> ② VIP 权限 ✅（合规支持：最佳音质选择 + 下载解密 + `is_vip` 检测，权益取决于账号自身）
> ⚠️ 未 live 验证的写接口已移到 feature `experimental`（默认关闭），见 `docs/experimental.md`。
> ⑦ 实时推送、⑧ Web 端 OAuth 登录为长周期项。

## 质量门禁

```bash
cargo fmt --check
cargo test --all-features
cargo clippy --all-targets --all-features -- -D warnings
```## 许可证

本项目采用 **[GNU General Public License v3.0 or later](./LICENSE)**。

本项目仅用于对技术可行性的探索及研究，请勿将其用于任何商业用途或侵犯版权的行为。

## 免责声明

由于使用本项目产生的包括由于本协议或由于使用或无法使用本项目而引起的任何性质的任何直接、间接、特殊、
偶然或结果性损害（包括但不限于因商誉损失、停工、计算机故障或故障引起的损害赔偿，或任何及所有其他商业
损害或损失）由使用者负责。
