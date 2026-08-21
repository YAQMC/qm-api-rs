# Experimental 接口（feature `experimental`）

部分桌面互操作写接口仅凭"命名对称性"推断出端点与方法名，
**尚未经过 live write 验证**。为避免把"协议观察到 endpoint"误认为
"当前协议已验证"，这些接口默认不编译，需显式启用：

```bash
cargo build --features experimental
# 或作为依赖: qqmusic-api = { features = ["experimental"] }
```

## 原则

- 先 read-only probe，写操作最后验证。
- 不对真实用户数据做猜测性的 destructive request。
- 在 live test 确认参数/响应语义之前，以下接口视为 Research Bucket。

## 当前 Experimental 接口

| 接口 | 模块/方法 | 原因 |
| --- | --- | --- |
| `album.fav_album` | `AlbumFavWrite / FavAlbum` | 请求参数名 (`v_albumId`) 为猜测，未获 live 证据 |
| `album.del_fav_album` | `AlbumFavWrite / CancelFavAlbum` | 参数名可能并非 `v_albumId`，语义未验证 |
| `user.focus_singer(action: ConcernAction, mid)` | `Concern.ConcernSystemServer / cgi_concern_user_v2` | `ConcernAction` 正反值仍需要 live-test |
| `user.fav_mv(vid, action: MvFavAction)` | `MVFavWrite / AddDelFavMV` | 请求 payload 为猜测；桌面端请求含 `cmdtype` 字段，`MvFavAction` 语义未验证 |

## 如何晋升为 Stable

1. 用测试账号实际调用写接口（仅当确认不会造成破坏时）。
2. 依据桌面端互操作观察 + live write 校验请求参数名、`ConcernAction`/
   `MvFavAction` 正反语义、响应 `code`。
3. 移除 `#[cfg(feature = "experimental")]` 与本文档中的对应条目。
