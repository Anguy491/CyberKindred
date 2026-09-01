# ADR-0002: 统一本地音乐与 Windows 系统媒体来源

| Metadata | Value |
|---|---|
| Status | Draft |
| Owner | Playback Architecture |
| Last Verified | 2026-09-01 |
| Source of Truth For | 本地曲库与 Apple Music Windows 系统媒体会话并存及 capability 模型的决策理由 |
| Related Documents | [Architecture](../ARCHITECTURE.md), [Runtime State Machines](../RUNTIME-STATE-MACHINES.md), [Provider Contracts](../../contracts/PROVIDER-CONTRACTS.md), [FRS](../../product/FRS.md) |

## Context

本地音乐允许 CyberKindred 精确选曲和排队；Apple Music Windows App 则拥有自己的账号、授权、catalog 和队列。v1 没有 Apple Developer Program/MusicKit 条件，也不应抓取 Web DOM、cookie 或逆向非公开接口，但仍希望感知当前曲目并执行 App 暴露的基本媒体控制。

## Decision

定义统一 `MusicSourceAdapter`，由两个实现提供不同 capability：

- `LocalMusicSource` 使用 rodio + Symphonia 播放用户批准目录中的音乐，支持精确 queue、play/pause/seek/next/previous 与可靠 position。
- `WindowsMediaSessionSource` 使用 windows-rs 连接 Apple Music Windows App 的 GSMTC session，读取实际 now-playing/timeline/playback info，并只执行会话当下声明支持的控制。

`setQueue` 是可选 capability，只有本地来源实现。Apple Music 模式是“陪伴当前播放”，不是曲库检索或指定点歌。系统媒体的外部状态是权威事实；session/capability/revision 改变会使 pending control 与 TTS resume token 失效。v1 不操控 Apple Music Web DOM，也不使用完整 MusicKit。

## Alternatives considered

- **只支持本地文件**：实现最稳，但不满足用户已在 Windows 使用 Apple Music 的重要场景。
- **MusicKit**：可提供 catalog/queue 能力，但需要 developer token、用户授权、更多合规与服务端/密钥设计，不适合 v1 条件。
- **Apple Music Web DOM automation**：脆弱、权限过大、易受页面更新影响，也会触碰登录态与条款风险。
- **把所有媒体会话当作同一种播放器**：容易误控其他 App，无法兑现 Apple Music 的明确连接体验。

## Consequences

- UI 和节目层必须 capability-driven，不得为 Apple Music 显示未暴露控制，也不得承诺精确选曲。
- 必须做真实 Windows App、有效订阅与打包态 GSMTC 探针；缺少 capability 是正常支持状态。
- TTS 串场需要保守 pause/resume token 与竞态测试，用户/外部 App 操作始终优先。
- 未来加入 MusicKit 或其他平台时可新增 adapter，但改变 Apple Music 用户能力仍需需求、隐私、契约与新 ADR。
