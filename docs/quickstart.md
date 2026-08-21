# 快速开始

## 添加依赖

```toml
[dependencies]
qqmusic-api = { git = "https://github.com/YAQMC/qm-api-rs" }
tokio = { version = "1", features = ["full"] }
```

## 创建客户端

```rust
use qqmusic_api::{Client, Platform, Credential};

// 方式一: 匿名访问 (默认 Android 平台)
let client = Client::new(None, None)?;

// 方式二: 指定平台
let client = Client::new(None, Some(Platform::Web))?;

// 方式三: 携带登录凭证
let credential = Credential {
    musicid: 12345678,
    str_musicid: "12345678".into(),
    musickey: "xxxxx".into(),
    login_type: 2, // 1=微信, 2=QQ
    ..Default::default()
};
let client = Client::new(Some(credential), None)?;
```

> **提示**：Android 平台在首次请求时会自动申请 QIMEI 指纹与 session，
> 因此首次调用会稍慢，后续请求会命中缓存。

## 基础调用

```rust
use qqmusic_api::{Client, SearchType};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(None, None)?;

    // 类型搜索 (返回当前页结果)
    let resp = client
        .search
        .search_by_type("周杰伦", SearchType::Song, 5, 1, &[], None, true)
        .await?;
    for song in resp.song {
        let names: Vec<_> = song.base.singer.iter().map(|s| s.name.as_str()).collect();
        println!("{} - {}", song.base.name, names.join(" / "));
    }

    // 快速搜索
    let quick = client.search.quick_search("周杰伦").await?;
    println!("歌曲命中数: {}", quick.song.count);

    // 热搜词
    let hot = client.search.get_hotkey().await?;
    println!("热搜第一位: {}", hot.vec_hotkey.first().map(|h| h.query.as_str()).unwrap_or(""));

    Ok(())
}
```

## 设置 / 切换登录凭证

```rust
client.set_credential(new_credential);
let current = client.credential();
```

## 平台差异

| 平台 | 常量 | 说明 |
| --- | --- | --- |
| Android | `Platform::Android` | 默认；需要 QIMEI 与 session；搜索、歌手主页等接口固定使用 |
| Desktop | `Platform::Desktop` | PC 客户端，`ct=19` |
| Web | `Platform::Web` | H5 网页，`ct=24`；歌曲详情接口固定使用 |

## 下一步

- [登录](./login.md)
- [下载歌曲](./download.md)
- [模块 API 参考](./modules.md)
