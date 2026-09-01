# Build and Release

| Field | Value |
|---|---|
| Status | Approved |
| Owner | Release Steward |
| Last Verified | 2026-09-02 |
| Source of Truth For | 版本、可复现构建、NSIS 制品、校验与回滚 |
| Related Documents | `DEVELOPMENT-GUIDE.md`, `../testing/TEST-STRATEGY.md`, `../planning/ROADMAP.md` |

## Versioning

- 使用 Semantic Versioning；内测版本从 `0.1.0` 开始。
- Patch：修复且不改变契约；Minor：向后兼容功能/数据迁移；Major：公开契约或用户数据语义不兼容。
- 每个制品对应干净 Git commit、完整 Changelog 和锁定依赖。

## Build profile

- Target: Windows x86_64, per-user NSIS installer.
- Frontend: production Vite bundle with source maps excluded from distributed package unless separately protected.
- Rust: release profile with debuginfo sufficient for local symbolication but不把用户路径/secret写入 panic output。
- WebView2: bootstrapper 检查/安装受支持 runtime；安装失败必须给出明确手动链接。
- 字体 Space Grotesk、Space Mono、Doto 以本地 assets 打包并附许可证。

## Reproducible build checklist

1. 工作区干净且 HEAD 在待发布提交；版本同时更新 Tauri config、package metadata 与 Changelog。
2. 仅使用 committed `pnpm-lock.yaml`、`Cargo.lock` 与固定 toolchain。
3. 在无 live flags、无开发 secret 的新 shell 中安装并运行全部默认质量门槛。
4. `pnpm tauri build` 生成 NSIS；记录 Rust/Node/pnpm/Windows SDK/WebView2 版本。
5. 对 installer 和主 executable 计算 SHA-256；验证安装、首次启动、卸载和数据保留/删除选项。
6. 将制品、hash、test evidence 和 release notes 放在版本化本地 release 目录；未经用户授权不上传。

## Signing and distribution

Documentation Baseline 假设内测制品暂未代码签名。必须向测试者说明 SmartScreen 风险并提供 SHA-256；不得指导关闭系统安全。公开发布前，代码签名、隐私文本、第三方 notices、更新签名和分发渠道成为阻塞门槛。

## Upgrade and rollback

- 每次 DB 迁移先备份数据库并采用 forward-only migration；启动失败时保留原文件和诊断副本。
- 安装包升级不得删除 `%LOCALAPPDATA%\CyberKindred`；只有用户在应用内完成精确二次确认的“清除全部数据”才删除用户数据、迁移备份和全部 CyberKindred origin Credential entries，安装/卸载本身不隐式执行该动作。
- 回滚仅允许目标版本能够读取当前 schema；否则恢复迁移前备份。不得用旧二进制直接打开更高 schema。

## Release evidence

发布记录包含 version、commit、artifact hash、toolchain、supported Windows、WebView2、schema version、quality-gate results、live Apple Music environment、known risks 和 rollback path。
