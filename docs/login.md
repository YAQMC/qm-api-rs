# 登录

`client.login` 提供四类登录方式：

1. **QQ 二维码登录**（HTTP 轮询）
2. **微信二维码登录**（HTTP 轮询）
3. **手机客户端二维码登录**（MQTT 推送，需要手机 QQ 音乐 App 扫码）
4. **手机验证码登录**

## QQ 二维码登录

```rust
use qqmusic_api::models::login::{QRLoginType, QRCodeLoginEvents};

// 1. 获取二维码
let qr = client.login.get_qrcode(QRLoginType::Qq).await?;
println!("qrsig = {}", qr.identifier);
std::fs::write("qrcode.png", &qr.data)?; // 展示给用户扫描

// 2. 轮询检查状态 (建议间隔 2~3 秒)
loop {
    let result = client.login.check_qrcode(&qr).await?;
    match result.event {
        QRCodeLoginEvents::Scan => { /* 等待扫描 */ }
        QRCodeLoginEvents::Conf => { /* 已扫描, 等待确认 */ }
        QRCodeLoginEvents::Done => {
            let credential = result.credential.unwrap();
            client.set_credential(credential.clone());
            println!("登录成功: musicid={}", credential.musicid);
            break;
        }
        QRCodeLoginEvents::Timeout => { /* 二维码过期, 重新获取 */ }
        QRCodeLoginEvents::Refuse => { /* 用户拒绝 */ }
    }
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
}
```

## 微信二维码登录

```rust
let qr = client.login.get_qrcode(QRLoginType::Wx).await?;
// identifier 为 uuid, data 为二维码图片
```

流程与 QQ 一致，调用 `check_qrcode` 即可。

## 手机客户端二维码登录（MQTT）

通过 `mu.y.qq.com` 的 MQTT 5.0 over WebSocket 长连接接收扫码推送：

```rust
use qqmusic_api::models::login::QRLoginType;

// 1. 获取手机客户端登录二维码 (identifier 为 qrcodeID)
let qr = client.login.get_qrcode(QRLoginType::Mobile).await?;
std::fs::write("mobile_qrcode.png", &qr.data)?;

// 2. 使用会话工具等待扫码完成
let mut session = QRCodeLoginSession::new(client.login.clone(), QRLoginType::Mobile);
let credential = session.wait_qrcode_login().await?;
client.set_credential(credential);
```

底层也暴露 `login.checking_mobile_qrcode(&qr, timeout)`，返回 `Vec<QRLoginResult>`
事件序列（`Scan` → `Conf` → `Done` / `Refuse` / `Timeout`）。

> 事件推送消息类型: `scanned`(已扫码) / `canceled`(取消) / `timeout`(超时) /
> `cookies`(登录成功, 携带凭证) / `loginFailed`(失败)。

## 手机验证码登录

```rust
// 1. 发送验证码 (明文手机号)
let result = client.login.send_authcode("13800138000", false, 86).await?;
// event: Send / Captcha(需要滑块) / Frequency(过于频繁)

// 2. 使用验证码换取凭证
let credential = client.login.phone_authorize("13800138000", false, "123456").await?;
client.set_credential(credential);
```

加密手机号场景：

```rust
// 若手机号已加密, 传入 is_encrypted = true, 字段为 encryptedPhoneNo
client.login.send_authcode("encrypted-phone", true, 86).await?;
```

## 检查 / 刷新 / 登出

```rust
// 检查凭证是否过期
let expired = client.login.check_expired(None).await?;

// 刷新凭证 (返回新凭证, 不会自动写入客户端)
let new_credential = client.login.refresh_credential(None).await?;
client.set_credential(new_credential);

// 登出
client.login.logout(None).await?;
```

## 登录会话工具

```rust
use qqmusic_api::{PhoneLoginSession, QRCodeLoginSession, QRLoginType};

// 手机验证码会话
let mut phone = PhoneLoginSession::new(client.login.clone(), "13800138000", false, 86);
phone.send_authcode().await?;
let credential = phone.authorize("123456").await?;

// 二维码登录会话
let mut session = QRCodeLoginSession::new(client.login.clone(), QRLoginType::Qq)
    .try_with_timeout(300.0)?;              // 非法超时返回错误, 不会 panic
let qr = session.get_qrcode().await?;       // 二维码数据用于展示

// GUI: 逐个事件实时更新 (QQ / 微信 HTTP 轮询)
loop {
    let result = session.next_event().await?;
    match result.event {
        QRCodeLoginEvents::Scan => { /* 等待扫描 */ }
        QRCodeLoginEvents::Conf => { /* 已扫码, 等待确认 */ }
        QRCodeLoginEvents::Done => {
            client.set_credential(result.credential.unwrap());
            break;
        }
        QRCodeLoginEvents::Timeout | QRCodeLoginEvents::Refuse => break,
    }
}

// 便利 API: 收集直到终端状态后一次性返回 (不是实时流)
let mut session = QRCodeLoginSession::new(client.login.clone(), QRLoginType::Qq);
let _events = session.iter_events().await?;
```

手机客户端 MQTT 登录请使用 `wait_qrcode_login` / `checking_mobile_qrcode`
（`timeout` 是整个二维码生命周期的总时限, 连接期间会按 Keep Alive 发送 ping）。

## 登录错误码

| 错误码 | 含义 | Rust 错误 |
| --- | --- | --- |
| `1000 / 104401 / 104400` | 鉴权参数无效或过期 | `QmError::CredentialExpired` |
| `20261` | 登录参数错误 | `QmError::Login` |
| `20271` | 验证码错误 | `QmError::Login` |
| `20277 / 20278` | 账号受限 | `QmError::Login` |
| `20279` | 登录设备数量超限 | `QmError::Login` |
| `20450` | 账号已被封禁 | `QmError::Login` |
| `104604` | 操作过于频繁 | `QmError::Login` |

## 下一步

- [下载歌曲](./download.md)
- [错误处理](./error-handling.md)
