# Rust Core Agent Instructions

| Field | Value |
|---|---|
| Status | Approved |
| Owner | Core Steward |
| Last Verified | 2026-09-02 |
| Source of Truth For | `src-tauri/` 内的 Rust 执行约束 |
| Related Documents | `../AGENTS.md`, `../docs/architecture/ARCHITECTURE.md`, `../docs/security/THREAT-MODEL.md` |

- 修改前阅读 `../docs/architecture/`、`../docs/contracts/`、`../docs/security/` 与相关 ADR。
- 所有前端可调用 command 采用最小权限、输入校验、稳定错误码与明确超时；不得向前端返回 secret。
- 运行路径不得使用 `unwrap` 或 `expect`；错误必须映射到 `ERR-*` 并脱敏记录。
- 音频、TTS、Apple Music 和日程行为必须遵守 `RUNTIME-STATE-MACHINES.md`；不得绕过 capability 检查。
- 数据库结构变化必须有向前迁移、回滚/恢复说明和数据保留测试。
- 文件访问限制在用户显式选择的曲库目录与应用数据目录；禁止隐式扩大递归范围。
- 网络调用集中在 provider adapters，使用超时、限流、缓存与可测试的 transport abstraction。
