# 下载歌曲（获取播放链接）

## 流程概览

获取可播放的音频链接通常需要以下步骤：

1. 通过搜索 / 歌单 / 排行榜等接口拿到歌曲的 `mid`
2. 调用 `client.song.get_song_urls` 获取 `purl` / `vkey`
3. 将 `purl` 拼接到 CDN 域名后下载

## 获取标准音质播放链接（无需登录）

```rust
use qqmusic_api::modules::song::{SongFileInfo, SongFileType};

let song = ...; // 例如搜索结果中的 song

let urls = client
    .song
    .get_song_urls(
        &[SongFileInfo::new(&song.mid).with_type(SongFileType::Mp3_128)],
        &SongFileType::Mp3_128,
        None,
    )
    .await?;

for item in urls.data {
    if !item.purl.is_empty() {
        // 完整链接 = CDN 域名 + purl
        println!("https://isure.stream.qqmusic.qq.com/{}", item.purl);
        println!("vkey = {}", item.vkey);
    }
}
```

## 音质类型

`modules::song` 中定义了四类文件类型：

| 类型 | 说明 | 示例 |
| --- | --- | --- |
| `SongFileType` | 普通音质 | `Mp3_128`(M500/.mp3)、`Mp3_320`(M800/.mp3)、`Flac`(F000/.flac)、`Master`(AI00/.flac)、`Atmos2`(Q000/.flac) 等 |
| `EncryptedSongFileType` | 加密音质（走 `GetEVkey`） | `Flac`(F0M0/.mflac)、`Master`(AIM0/.mflac)、`Vinyl`(V0M0/.mflac) 等 |
| `SpecialSongFileType` | 试听 / 伴奏 / AI 演奏 | `Try`(RS02)、`Accom`(O801)、`Piano`(AI01) 等 |
| `RingSongFileType` | 彩铃 | `Ring128`(R500/.mp3) 等 |

文件命名规则：`{start_code}{media_mid}{extension}`；若未提供 `media_mid`，
则为 `{start_code}{mid}{mid}{extension}`。

## 获取登录后可用的链接

```rust
let urls = client
    .song
    .get_song_urls(&[SongFileInfo::new(&song.mid)], &SongFileType::Mp3_320, Some(&credential))
    .await?;
```

> 部分音质（如无损 / 臻品）需要绿钻会员权限；未购买时会返回
> `result = 104003`（无权限），`purl` 为空。

## CDN 调度

```rust
let cdn = client.song.get_cdn_dispatch().await?;
println!("可用 CDN: {:?}", cdn.sip);
```

## 曲谱

```rust
// ttype: 0=用户上传, 1=引擎/AI 曲谱, 2=虫虫钢琴
let sheets = client.song.get_sheet(&song.mid, 0).await?;
println!("曲谱数量: {}", sheets.result.len());

let has = client.song.has_sheet(&song.mid).await?;
```

## 加密音质解密 (QMC)

`EncryptedSongFileType` 下载到的 `.mflac` / `.mgg` / `.mmp4` 文件是 QMC 加密的,
需要解密后才能播放. 本库内置 `qqmusic_api::qmc` 模块:

```rust
use qqmusic_api::qmc;
use std::path::Path;

// 方式一: 文件内嵌密钥 (Android QTag / PC V1), 直接解密
let (audio, ext) = qmc::decrypt_file(Path::new("song.mflac"), None)?;
println!("输出格式: {}", ext);  // flac / ogg / mp4 ...

// 方式二: 文件未内嵌密钥, 用 get_song_urls 返回的 ekey
let (audio, ext) = qmc::decrypt_file(Path::new("song.mflac"), Some(&ekey))?;

// 写入磁盘
let out = qmc::decrypt_file_to(Path::new("song.mflac"), Path::new("./out"), None)?;
```

支持的算法 (移植自 unlock-music 官方 Rust 实现):

- **QMCv1** 静态密钥 (`.tkm` / `.bkc*` / 十六进制扩展名等旧格式)
- **QMCv2** Map (短密钥 ≤300 字节) / RC4 (长密钥 >300 字节), 覆盖
  `.mflac` / `.mgg` / `.mgg0` / `.mflac0` / `.mmp4` / `.qmcflac` / `.qmcogg` 等
- **EKey** 解密: V1 (simple key + header) 与 V2 (双层 TEA)
- **Footer** 解析: QTag / STag / PcV1Legacy / MusicEx

> 注意: 仅用于解密你自己合法下载、有权使用的音频文件.

## 歌词解析 (LRC / QRC)

`get_lyric` 返回解密后的 LRC 文本; 需要结构化解析时使用 `qqmusic_api::lyric_parser`:

```rust
use qqmusic_api::lyric_parser::{Lyric, QrcLyric};

let lyric = client.lyric.get_lyric(&song.mid, 1, false, false, false, false).await?;
let parsed = Lyric::parse(&lyric.lyric);
println!("行数: {}", parsed.lines.len());
println!("第 3 秒: {:?}", parsed.line_at(3000));   // Option<&LyricLine{time_ms, text}>

// QRC 逐字歌词 (qrc=true)
let qrc = client.lyric.get_lyric(&song.mid, 1, true, false, false, false).await?;
let parsed_qrc = QrcLyric::parse(&qrc.lyric);
println!("逐字: {:?}", parsed_qrc.line_at(0));
```

## 批量获取歌曲信息

```rust
use qqmusic_api::modules::song::SongQueryInfo;

let tracks = client
    .song
    .query_song(&[SongQueryInfo::by_mid("0039MnYb0qxYhV")])
    .await?;
println!("{}", tracks.tracks[0].name);
```

## VIP 音质 (使用你账号已购的权益, 非破解)

配合 VIP 账号的 `Credential`, 可获取并下载高音质音源:

```rust
// 1. 检查账号是否具备 VIP 权益
let is_vip = client.user.is_vip(Some(&credential)).await?;

// 2. 查看歌曲可用音质 (依据 file 尺寸字段, 从高到低)
let qualities = client.song.available_qualities(&song);
//   如 [Master, Atmos2, Flac, Ogg320, Mp3_320, ...]

// 3. 获取最高可用音质播放链接
//   allow_encrypted=true: 走 CgiGetEVkey, 返回 .mflac/.mgg + ekey
let (quality, urls) = client
    .song
    .get_best_song_url(&song, Some(&credential), true)
    .await?;
//   无权限时 item.result = 104003, purl 为空

// 4. 下载 + QMC 解密 (VIP 完整流程)
let (audio, ext) = client
    .song
    .download_quality(&song, quality, Some(&credential))
    .await?;   // ext: flac / ogg / mp4 ...
std::fs::write(format!("{}.{}", song.mid, ext), &audio)?;
```

完整可运行示例见 [`examples/vip_demo.rs`](../examples/vip_demo.rs):

```bash
export QM_MUSICID=你的musicid
export QM_MUSICKEY=你的musickey
cargo run --example vip_demo
```

`SongQuality` 档位: `Master`(臻品母带) > `AtmosDb`(杜比全景声) > `Atmos2`(臻品音质) >
`Atmos51`/`Atmos71` > `Nac` > `Flac`(SQ 无损) > `Ogg640` > `Ogg320` > `Mp3_320` > ...
支持加密版本 (`has_encrypted`) 的音质在解密后即恢复为原始 FLAC/OGG。

> 说明: 高音质播放/下载需要该账号具备对应绿钻/超级会员权益; 库本身不绕过任何权限校验。

## 下一步

- [错误处理](./error-handling.md)
- [模块 API 参考](./modules.md)
