# 架构与签名

本文档描述 `qqmusic-api` 的内部架构，帮助理解请求如何被构建、签名与发送。

## 总体结构

```
Client
 ├─ ApiContext (Arc)
    │   ├─ ApiTransport          可注入 HTTP 传输 (默认 ReqwestApiTransport)
    │   ├─ platform               默认平台
    │   ├─ version_policy         各平台版本档案 (ct/cv/UA)
    │   ├─ cgi_base_url           CGI 基础地址 (可指向 mock 服务器)
    │   ├─ credential (Mutex)     登录凭证
│   ├─ device (Mutex)         模拟 Android 设备 (设备身份, QIMEI 唯一状态源)
│   ├─ sessions (tokio)       按账号缓存的 Android session (账号运行态)
│   ├─ state_lock (tokio)     session/QIMEI 申请的 singleflight 锁
│   └─ limiter                请求限流器
 └─ 各模块 (SongApi / SearchApi / ...)  均持有 Arc<ApiContext>
```

- 模块对象只依赖 `ApiContext`，不依赖 `Client`，因此不存在引用环。
- 所有状态（凭证、设备）通过 `Mutex` 保护，`&self` 方法即可并发调用。
- **`Device` 只代表设备身份**（android_id/imei/open_udid/qimei 等），运行时获取
  的新 QIMEI 写回 `Device`，`Client::save_device` / `load_device` 可跨进程复用。
- **Android session 是账号运行态**，按 `musicid` 缓存在 `sessions` 中
  （不落在 `Device` 上），`session_for` 返回不可变快照，与 `credential`
  原子一致，多账号并发不串号。
- 每次 `set_device` 在同一把锁下写入新 Device 并递增 `device_epoch`。
  QIMEI / GetSession 等异步请求绑定**开始时**的 epoch；响应返回后若 epoch
  已变，丢弃 stale 结果，绝不把 D0 的 QIMEI/Session 写进 D1。
- `state_lock` 保证多个并发 stale 请求只触发一次 session / QIMEI 申请。

## HTTP 传输 (`ApiTransport`)

所有 CGI / QIMEI / `request_http` / `request_http_bytes` 都走 `ApiContext` 持有的
`Arc<dyn ApiTransport>`，不再把 `reqwest::Client` 暴露为公开发送路径。

- **默认实现** `ReqwestApiTransport`：reqwest **0.12**，`gzip` / `brotli` /
  `cookie_store(true)`（仅服务 ptlogin / 微信等 HTTP 登录跳转）。reqwest 类型只留在私有模块。
- **注入**：`Client::new` / `ApiContext::new` 使用默认实现；
  `new_with_transport(Arc<dyn ApiTransport>)` 注入自定义传输；
  `new_with_transport_config(TransportConfig)` 只改超时/代理/重试而不换实现。
  **`ApiTransport` 不必实现 cookie store**。鉴权 Cookie 与浏览器头由库写在每次
  CGI 请求上，不假设 jar，也不假设调用方补 `Referer`。
- **Timeout**：默认 connect **5s**、总超时 **15s**（`TransportConfig` 可改）。
  单次请求可用 `HttpOptions.timeout` 覆盖总超时（微信二维码长轮询 35s）。
- **Allowlist**：发送前检查 host。生产 HTTPS 至少覆盖
  `u.y.qq.com` / `c.y.qq.com` / `c6.y.qq.com` / `api.tencentmusic.com` /
  `ssl.ptlogin2.qq.com` / `ssl.ptlogin2.graph.qq.com` / `xui.ptlogin2.qq.com` /
  `graph.qq.com` / `y.qq.com` / `open.weixin.qq.com` / `lp.open.weixin.qq.com`，
  以及媒体 CDN / COS 后缀。`cgi_base_url` / `qimei_url` 指向 mock 时自动放行
  该 origin。拒绝返回 `QmError::Protocol { stage: "allowlist", .. }`，不 panic。
- **Redirect**：默认 `FollowValidated`，校验 allowlist 后最多 **3** 跳；
  二维码 / cookie 交换使用 `RedirectMode::None`（返回 30x，不跟随）。
- **Cancellation**：每个请求携带 `tokio-util::sync::CancellationToken`
  （crate 再导出为 `qqmusic_api::CancellationToken`）。默认 transport 在
  send / 读 body / 重试等待时 `select!` 该令牌；取消为
  `NetworkErrorKind::Cancelled`（不可重试）。
- **Retry**：`RetryClass::SafeRead` 对网络抖动 / HTTP 429 / 5xx 额外重试 1 次
  （间隔 250ms）；`Write` / `AuthPoll` 默认不重试。登录写与歌单/评论等状态
  改变已标为 `Write`。

MQTT 登录推送（`mqtt.rs`）仍走独立 WebSocket，不进入 `ApiTransport`。

## CGI 请求流程

1. 模块方法构造 `param`，调用 `context.request_cgi(module, method, param, opts)`。
2. 若 `require_login`，先校验凭证。
3. `build_api_kwargs` 构建：
   - `payload = { "comm": <comm>, "req_0": { module, method, param } }`
   - URL：`sign=false`（默认）走 `musicu.fcg`；`sign=true` 走 `musics.fcg` + zzc
     （`_` 与 `sign` 查询参数）。网页歌词、多数读接口用未签名 `musicu.fcg`。
     `get_dislike_list` 等签名读探针保持 `sign=true`。
4. CGI 出口（`request_cgi` / `request_cgi_batch`）组头后 POST：
   - **Cookie**：按 `Credential` 写入 `uin` / `qqmusic_uin` / `qm_keyst` /
     `qqmusic_key`，不依赖 transport cookie jar。
   - **Referer / Origin**：对 `u.y.qq.com` / `c.y.qq.com` / `c6.y.qq.com`
     （本地 mock CGI 同样）在请求尚未携带时补 `Referer: https://y.qq.com/` 与
     `Origin: https://y.qq.com`，不覆盖 ptlogin / 微信已有的 Referer。
5. 解析响应信封（`parse_cgi_envelope`）：
   - 外层 `code != 0` → `GlobalApi`（transport 级错误）
   - 其余情况**始终**返回固定形状 `CgiReply { code, data }`，不解释业务错误码。
6. 业务层决定如何解释 `code`：
   - 普通接口经 `require_success()`：`2000` → `SignatureRequired`，`2001` → `RateLimited`，
     `1000/104401/104400` → `CredentialExpired`，其他非零 → `CgiApi`，`0` → 返回 `data`
   - 登录等接口直接读取 `code` 自行处理（如 `20276` 验证码、`20271` 验证码错误等）

批量请求 `request_cgi_batch` 返回 `Vec<CgiReply>`，单个子请求的业务错误码不会导致
整体失败。

## comm 公共参数

`versioning::VersionPolicy::build_comm` 按平台构建 `comm`：

- **Android** (`ct=11`, `cv=14090008`)：
  `chid=10003505`, `tmeAppID=qqmusic`, `QIMEI`/`QIMEI36`（自动申请）,
  `OpenUDID`/`udid`, `aid`, `os_ver`, `phonetype`, `devicelevel`, `rom`, 登录字段 `qq`/`authst`/`tmeLoginType`。
  **登录 CGI 靠 `comm.authst`**，即使请求上没有 Cookie 也能登录。
- **Desktop** (`ct=19`, `cv=2201`)：
  `chid=0`, `uin`, `g_tk`（`hash33(musickey, 5381)`）, `guid`。comm 无 `authst`。
- **Web** (`ct=24`, `cv=4747474`)：
  `chid=0`, `uin`, `g_tk`/`g_tk_new_20200303`, `format=json`, `notice=0`, `need_new_code=1`。comm 无 `authst`。

**Web / Desktop 登录 CGI 必须靠 Cookie**（见上，由库写入每次 CGI）。注入
`ApiTransport` 没有 cookie jar 时尤其如此；缺 Cookie 时这两端基本当游客。
Client 默认平台仍是 **Android**，不因此改成 Web。

## 签名算法

需要签名的接口（曲谱、上传、不喜欢列表、歌单写操作等）走 `musics.fcg`，
`sign` 由 `sign::zzc_sign` 计算：

```
hash  = SHA1(payload) (hex, 大写)
part1 = hash[23,14,6,36,16,7,19]
part2 = hash[16,1,32,12,19,27,8,5]
part3 = for i in 0..20: SCRAMBLE[i] ^ int(hash[i*2..i*2+2], 16)
b64   = base64(part3) 去掉 / \ + =
sign  = lower("zzc" + part1 + b64 + part2)
```

> 官方桌面客户端在 `musics.fcg` 上使用 `__TENCENT_CHAOS_VM` 混淆虚拟机生成
> 签名。该 VM 输出的 `sign` 与 `zzc_sign` 通过相同的服务端校验，
> 本实现使用与 Python 参考库一致的简化 SHA1 方案即可通过校验。

## QIMEI（Android 指纹）

首次使用 Android 平台时，会向 `api.tencentmusic.com/tme/trpc/proxy` 申请
QIMEI：

- 请求负载使用 RSA（PKCS1v15，公钥硬编码）加密随机 AES key，
  再用 AES-128-CBC（key == iv）加密业务数据
- 请求头 / 请求体使用 MD5 签名
- 结果缓存 24 小时，**写回 `Device`（`qimei`/`qimei36`/`qimei_save_time`）**，
  因此 `Client::save_device` 可持久化

## QRC 歌词解密

`get_lyric` 固定网页访客信封（Web、未签名 `musicu.fcg`）。返回的 `lyric`/`trans`/`roma`
字段可能经过加密：

1. 3DES-EDE（自定义实现，8 字节 ECB 分块，密钥 `!@#)(*$%123ZXC!@!@#)(NHL`）
2. zlib 解压

`models::lyric::GetLyricResponse::parse` 会自动解密，无需手动处理。

## Android session 与账号状态

Android 平台的 `comm` 需要 `uid` / `sid`。首次请求时调用
`music.getSession.session` 获取并按账号缓存 24 小时。

`Device` 只代表**设备身份**（android_id/imei/open_udid/qimei 等），QIMEI 写回
`Device` 并经 `Client::save_device` / `load_device` 跨进程复用。

Android session 是**账号运行态**：`ApiContext.session_for(platform, credential)`
按 `musicid` 缓存在 per-account 的 `HashMap` 中（不落在 `Device` 上），返回
不可变快照 `Arc<AndroidSession>`。`build_comm` 在 `build_api_kwargs` 中
同时持有 `credential` 与该快照，二者原子一致——两个账号并发请求时
"credential A + session B"的 TOCTOU 竞态不再可能发生。并发 stale 请求经
`state_lock` singleflight 只触发一次申请。

## 手机客户端二维码登录（MQTT）

`src/mqtt.rs` 实现了最小化的 MQTT 5.0 over WebSocket 客户端，用于
`login.checking_mobile_qrcode`：

1. 通过 `CreateQRCode` 获取二维码与 `qrcodeID`
2. 连接 `wss://mu.y.qq.com:443/ws/handshake`，CONNECT 携带
   `auth_method="pass"` 与 `user_property`（`tmeAppID/business/hashTag/...`）
3. 订阅 `management.qrcode_login/{qrcodeID}`（携带 `authorization=tmelogin` 等属性）
4. 接收 PUBLISH 推送，依据用户属性 `type` 解析事件
   （`scanned/canceled/timeout/cookies/loginFailed`）

支持 CONNACK 服务器重定向（reason `0x9C/0x9D` + `serverReference`）。

`try_parse_packet` 保留完整 Fixed Header（含低 4 位标志位），因此 PUBLISH 的
QoS 1/2 会正确跳过 packet id；`parse_properties` 对未知属性按 MQTT 5 属性注册表
跳过其值，避免把后续内容误当作 payload。

## 协议测试

- `parse_cgi_envelope` 单测覆盖成功 / 业务错误码透传 / 全局错误 / 批量 / 缺失
  `req_N` / 空 data。
- `ApiContext::cgi_base_url` 可指向本地 mock 服务器（`context.rs` 测试内的
  axum 服务），端到端验证 `request_cgi` / `request_cgi_batch` 的 envelope 契约
  与部分失败。mock origin 由 transport allowlist 自动放行。CGI mock 同时断言
  默认 `Referer`/`Origin` 与 Credential Cookie。
- `ApiTransport` 单测覆盖未知 host 拒绝、timeout、redirect 0/3 跳、取消、
  写请求不重试。
- 模型 schema drift 测试验证字段缺失/改名时按 `#[serde(default)]` 兜底。
- QIMEI / session 的 Device 缓存命中与并发读一致性有专门测试。
- `mqtt.rs` 含 MQTT 5 报文构造/解析与 QoS 1/2 偏移的位级单测。

## 连续翻页

`pagination::Pager<T>` 封装跨页拉取：

- `new(initial_params, fetch, next_params)` 创建分页器
- `next()` 拉取下一页; `collect()` 收集所有页响应
- `collect_items(extract)` 跨页展开数据项
- `page(...)` / `offset(...)` 提供基于页码 / 偏移量的便捷策略

```rust
let mut pager = qqmusic_api::offset(
    "offset", "num",
    json!({"topId": tid, "offset": 0, "num": 10}),
    { /* fetch closure: 调用模块方法 */ },
    |resp| resp.songs.len() >= 10,
);
let songs = pager.collect_items(|r| r.songs.clone()).await?;
```

## 下一步

- [模块 API 参考](./modules.md)
- [错误处理](./error-handling.md)
