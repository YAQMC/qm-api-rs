# 文档索引

本目录包含 `qqmusic-api` (Rust) 的使用文档。

## 目录

- [快速开始](./quickstart.md) — 安装、创建客户端、基础调用
- [登录](./login.md) — QQ / 微信二维码登录、手机验证码、凭证刷新
- [下载歌曲](./download.md) — 获取歌曲播放链接、CDN、曲谱
- [错误处理](./error-handling.md) — 错误类型与常见错误码
- [架构与签名](./architecture.md) — 请求流程、comm 构建、签名算法、平台差异
- [模块 API 参考](./modules.md) — 各模块方法与参数

## 参考

- 上游 Python 库: <https://github.com/L-1124/QQMusicApi>
- 上游文档: <https://l-1124.github.io/QQMusicApi/>
- 官方客户端 (Electron ASAR): `qqmusic_1.1.8-1.asar`

## 生成 rustdoc

```bash
cargo doc --no-deps --open
```
