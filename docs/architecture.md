# 架构与签名

本文档描述 `qqmusic-api` 的内部架构，帮助理解请求如何被构建、签名与发送。

## 总体结构

```
Client
 ├─ ApiContext (Arc)
 │   ├─ reqwest::Client        底层 HTTP 客户端
 │   ├─ platform               默认平台
 │   ├─ version_policy         各平台版本档案 (ct/cv/UA)
 │   ├─ cgi_base_url           CGI 基础地址 (可指向 mock 服务器)
 │   ├─ credential (Mutex)     登录凭证
 │   ├─ device (Mutex)         模拟 Android 设备 (QIMEI/session 唯一状态源)
 │   ├─ state_lock (tokio)     session/QIMEI 申请的 singleflight 锁
 │   └─ limiter                请求限流器
 └─ 各模块 (SongApi / SearchApi / ...)  均持有 Arc<ApiContext>
```

- 模块对象只依赖 `ApiContext`，不依赖 `Client`，因此不存在引用环。
- 所有状态（凭证、设备）通过 `Mutex` 保护，`&self` 方法即可并发调用。
- **`Device` 是 QIMEI / Android session 的唯一状态源**：运行时获取的新值写回
  `Device`，`Client::save_device` / `load_device` 可跨进程复用；session 还记录
  归属账号 `session_musicid`，保证多账号下 session 不串号。
- `state_lock` 保证多个并发 stale 请求只触发一次 session / QIMEI 申请。

## CGI 请求流程

1. 模块方法构造 `param`，调用 `context.request_cgi(module, method, param, opts)`。
2. 若 `require_login`，先校验凭证。
3. `build_api_kwargs` 构建：
   - `payload = { "comm": <comm>, "req_0": { module, method, param } }`
   - URL：普通 `https://u.y.qq.com/cgi-bin/musicu.fcg`；
     需要签名时 `https://u.y.qq.com/cgi-bin/musics.fcg`，并附加 `_` 与 `sign`。
4. 发送 POST，解析响应信封（`parse_cgi_envelope`）：
   - 外层 `code != 0` → `GlobalApi`（transport 级错误）
   - 其余情况**始终**返回固定形状 `CgiReply { code, data }`，不解释业务错误码。
5. 业务层决定如何解释 `code`：
   - 普通接口经 `require_success()`：`2000` → `SignatureRequired`，`2001` → `RateLimited`，
     `1000/104401/104400` → `CredentialExpired`，其他非零 → `CgiApi`，`0` → 返回 `data`
   - 登录等接口直接读取 `code` 自行处理（如 `20276` 验证码、`20271` 验证码错误等）

批量请求 `request_cgi_batch` 返回 `Vec<CgiReply>`，单个子请求的业务错误码不会导致
整体失败。

## comm 公共参数

`versioning::VersionPolicy::build_comm` 按平台构建 `comm`：

- **Android** (`ct=11`, `cv=14090008`)：
  `chid=10003505`, `tmeAppID=qqmusic`, `QIMEI`/`QIMEI36`（自动申请）,
  `OpenUDID`/`udid`, `aid`, `os_ver`, `phonetype`, `devicelevel`, `rom`, 登录字段 `qq`/`authst`/`tmeLoginType`
- **Desktop** (`ct=19`, `cv=2201`)：
  `chid=0`, `uin`, `g_tk`（`hash33(musickey, 5381)`）, `guid`
- **Web** (`ct=24`, `cv=4747474`)：
  `chid=0`, `uin`, `g_tk`/`g_tk_new_20200303`, `format=json`, `notice=0`, `need_new_code=1`

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

歌词接口返回的 `lyric`/`trans`/`roma` 字段可能经过加密：

1. 3DES-EDE（自定义实现，8 字节 ECB 分块，密钥 `!@#)(*$%123ZXC!@!@#)(NHL`）
2. zlib 解压

`models::lyric::GetLyricResponse::parse` 会自动解密，无需手动处理。

## Android session 与单一状态源

Android 平台的 `comm` 需要 `uid` / `sid`。首次请求时调用
`music.getSession.session` 获取并缓存 24 小时。

`Device` 是设备指纹（QIMEI / session）的**唯一状态源**：运行时获取的新
QIMEI / session 都会写回 `Device`（`session_uid`/`session_sid`/
`session_save_time`），`build_comm` 直接读取 `Device`，从而保证
`Client::save_device` / `load_device` 能跨进程复用，且不存在
"context 缓存一份、device 一份"的双状态源问题。

Session 归属发起请求的账号：`Device.session_musicid` 记录申请时的
`musicid`，`ensure_session` 仅在"未过期 **且** 属于同一账号"时复用缓存，
否则重新申请——保证多账号（per-request credential）下 session 不串号。
并发 stale 请求经 `state_lock` singleflight 只触发一次申请。

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
  与部分失败。
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
