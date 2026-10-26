# QQ 协议与宿主边界

QQ 的 endpoint、module/method、业务类型、请求字段、响应别名、时间单位及媒体文件名属于 `qm-api-rs`。宿主通过类型化接口消费结果，不再自行拼 CGI 或依赖 `raw_*`。

| 能力 | 库接口 |
| --- | --- |
| 歌曲热门、最新与回复 | `client.comment.get_song_comments(&SongCommentsRequest)` |
| 当前账号资料 | `client.user.get_current_profile(&Credential, CancellationToken)` |
| 歌单与收藏写入回执 | `account::write_checked`、`WriteOutcome` |
| 账号歌单元数据别名 | `account::AccountPlaylistMetadata`、`OwnedPlaylistEditFields` |
| 播放候选文件名及 vkey 响应 | `playback::PlaybackFile`、`resolve_files` |
| CDN 校验与相对地址拼接 | `playback::normalize_cdn_url` |
| 歌词文本编码 | `models::lyric::decode_lyric_text` |
| XML/QRC、绝对逐字时间、LRC/纯文本 | `lyric_parser::parse_playback_qrc`、`parse_playback_lrc` |

评论页码从 1 开始，`last_seq` 是不透明的服务端序号。歌曲的业务类型是 `1`；回复按服务端线程顺序返回。公共评论读取显式使用匿名凭据，不继承客户端账号。

无状态宿主通过 `SongCommentsRequest::new(...).with_cursor(cursor)` 恢复分页，
通过 `SongCommentsPage::next_cursor(&request)` 取得下一页游标。编码、歌曲/排序/父评论/批次作用域检查、
序号推进和终止条件均由库拥有；宿主不解析或修改游标。兼容迁移前 YAQMC 的 v1 游标编码。

`SongCommentsRequest.limit` 是每次请求期望的大小（1–50），不是服务端返回条数的保证。
实测《晴天》热门评论在请求 20 条时返回 21 条；库保留完整页面，不因额外条目拒绝整页或直接截断，
下一页使用完整页面末条的 `seq_no`。没有按总评论数或固定页数提前停止；页码受 `u32` 表示范围约束。

需要连续读取时使用 `client.comment.song_comments_pages(request, cancellation)`：每次
`next().await` 才取一页，不预取、不累计整首歌的评论；消费后释放该页即可保持内存与总页数无关。
结束标志、空页、空或未推进的序号、页码溢出及错误都会停止迭代。`collect()` 仍会由调用方累积所有页，
不适合大评论区。单页读取也可用 `get_song_comments_with_cancellation` 显式取消。

防止卡顿的边界是每个解压后的响应最多 4 MiB，而非评论条数：默认传输按网络块读取并逐块检查字节、
超时及取消，CGI 在解析前再次检查字节长度，JSON 和评论转换在 Tokio 阻塞任务池执行。
注入的 `ApiTransport` 也必须在读取过程中执行 `max_response_bytes`；事后检查无法消除第三方传输已产生的分配。
取消后不再等待解析结果，但已经开始的阻塞解析会完成当前这个有字节上限的页面。
单页处理时间为 O(B + n)，空间为 O(B + n)，B 为本页响应字节、n 为本页条目数；不随累计读取页数增长。

宿主如需要比上游页面更小的 UI 批次，应分批消费完整结果并保存未显示的条目；不能直接丢弃多余条目，
也不能把 `limit` 当成上游严格承诺。库不改变宿主自己的 IPC、缓存或渲染策略。

资料查询只接受当前账号的映射项，不以其他账号资料兜底。`WriteOutcome::Unknown` 表示回执不足以确认结果；调用方应安全读取核对，不能自动重复写入。播放 URL 和 ekey 不出现在 Debug 中；未获授权或缺少 ekey 的来源保持不可用。

新的歌词解析入口保持 `[start,duration]word(start,duration)` 的毫秒语义，并支持 XML 包装与小时级 LRC 标签。原有 `Lyric`/`QrcLyric` 接口保留兼容，宿主应使用 `parse_playback_*` 获取绝对时间。

宿主保留自己的缓存、凭据安全存储、账号生命周期、写后核对、音质选择、歌词界面 DTO、翻译对齐和播放器状态。通用 `ApiTransport` 注入允许宿主管理自己的 HTTP 实现；QQ 请求的选择与编码由库负责。
