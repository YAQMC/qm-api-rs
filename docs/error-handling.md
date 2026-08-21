# 错误处理

所有方法均返回 `Result<T, QmError>`。

## 错误类型

| 变体 | 说明 |
| --- | --- |
| `QmError::Network(NetworkError)` | 网络层错误（连接失败、超时、TLS、取消等），含 `kind` |
| `QmError::Http { status, body }` | HTTP 状态码非 200 |
| `QmError::GlobalApi { code, data }` | CGI 外层信封错误（`code != 0`） |
| `QmError::CgiApi { code, data }` | CGI 子请求错误 |
| `QmError::SignatureRequired` | 接口要求签名但未提供（code 2000） |
| `QmError::RateLimited` | 请求被限流（code 2001） |
| `QmError::CredentialExpired(String)` | 登录凭证已过期（code 1000/104401/104400） |
| `QmError::CredentialInvalid(String)` | 请求需要登录但未提供有效凭证 |
| `QmError::Login { message, code }` | 登录业务错误 |
| `QmError::CredentialRefresh(String)` | 凭证刷新失败 |
| `QmError::Deserialize(String)` | 响应 JSON 反序列化失败 |
| `QmError::Protocol { stage, message }` | 协议/结构错误（含 allowlist 拒绝） |
| `QmError::ApiData(String)` | 响应内容缺失 / 无法解析 |
| `QmError::JsonPath(String)` | JSONPath 提取失败 |
| `QmError::ValueError(String)` | 参数校验失败 |

## 错误分类与重试

`QmError::category() -> ErrorCategory` 提供粗分类（`Network / Auth / Permission /
BadRequest / RateLimit / NotFound / Server / Other`），供展示与重试策略参考；
`QmError::is_retryable() -> bool` 对网络抖动（超时/连接/body）、限流（`2001`/`104604`）、
服务端 5xx 返回 `true`。取消（`NetworkErrorKind::Cancelled`）与请求构造失败不可重试。

```rust
if e.is_retryable() {
    // 指数退避后重试
}
```

## 脱敏

`QmError` 中的响应载荷（`CgiApi.data` / `GlobalApi.data` / `Http.body`）在进入
错误前已做脱敏：截断到 400 字符，并掩码 `qm_keyst` / `musickey` /
`access_token` / `refresh_token` / `p_skey` 等疑似令牌字段。日志中不会出现
完整响应或敏感凭证。

## 批量请求与部分失败

`request_cgi_batch` 返回 `Vec<CgiReply { code, data }>`，单个子请求的业务错误码
不会导致整体失败。需要全部成功时用 `cgi_batch`（任一子请求失败即报错）；
需要处理部分失败时：

```rust
use qqmusic_api::CgiReply;

let replies = client.request_cgi_batch(&reqs, &Default::default()).await?;
let (ok, err) = CgiReply::partition(replies);           // 成功 / 失败分组
let report = CgiReply::report(&replies);                // BatchReport { total, succeeded, failures }
```

## 常见错误码

| 错误码 | 含义 | 建议 |
| --- | --- | --- |
| `2000` | 需要签名 | 请求时设置 `sign = true`（模块方法已处理） |
| `2001` | 限流 | 放慢请求速度，增加间隔 |
| `1000 / 104401 / 104400` | 凭证过期 | 调用 `login.refresh_credential` 或重新登录 |
| `104003` | 无播放权限 | 歌曲需要 VIP 或未购买 |
| `80092` | 歌单操作无变化 | `add_songs` / `del_songs` 会返回 `false` 而非报错 |
| `10007` | 曲谱不存在 | `get_sheet` 已标记为允许，正常返回空 |

## 处理示例

```rust
use qqmusic_api::QmError;

match client.song.get_song_urls(&infos, &SongFileType::Mp3_128, None).await {
    Ok(urls) => { /* 成功 */ }
    Err(QmError::CredentialExpired(_)) => {
        let cred = client.login.refresh_credential(None).await?;
        client.set_credential(cred);
        // 重试
    }
    Err(QmError::RateLimited) => {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        // 重试
    }
    Err(e) => eprintln!("其他错误: {e}"),
}
```

## 需要登录的接口

调用需要登录的接口（如 `songlist.create`、`comment.add_comment`、`user.get_vip_info`）
时，若客户端未设置有效凭证，会返回 `QmError::CredentialInvalid`。

```rust
if client.credential().musickey.is_empty() {
    // 先完成登录
    return;
}
```

## 下一步

- [架构与签名](./architecture.md)
- [模块 API 参考](./modules.md)
