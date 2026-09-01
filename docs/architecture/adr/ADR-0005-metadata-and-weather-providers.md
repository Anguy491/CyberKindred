# ADR-0005: 使用 MusicBrainz/CAA 与 Open-Meteo 进行可选增强

| Metadata | Value |
|---|---|
| Status | Draft |
| Owner | Integration Architecture |
| Last Verified | 2026-09-01 |
| Source of Truth For | 音乐元数据、封面与天气 provider 选型及数据最小化的决策理由 |
| Related Documents | [AI Orchestration](../AI-ORCHESTRATION.md), [Data Model](../DATA-MODEL.md), [External Integrations](../../integrations/EXTERNAL-INTEGRATIONS.md), [Privacy Data Lifecycle](../../security/PRIVACY-DATA-LIFECYCLE.md), [Legal and Licensing](../../security/LEGAL-AND-LICENSING.md) |

## Context

本地文件标签质量不一，节目推荐可受流派/标准标识与封面增强；天气可让串场更贴合时间环境。两者都不是播放所必需，且不得上传音频、路径、精确定位或因外部故障阻断节目。

## Decision

- 使用 MusicBrainz Web Service 按本地 title/artist/album/duration 做候选匹配；不上传音频。全应用限制 1 request/second，设置明确 User-Agent，保存 MBID、规范化标签、置信度与 provenance。
- confidence ≥0.90 且 title/artist 规范化精确匹配才自动采用为增强字段；0.70–0.89 仅作 suggestion；更低结果按 no-match 缓存 24 小时。原始 local tag 永不被外部结果覆盖。
- 只有已确定 MBID 时调用 Cover Art Archive；封面是可清理 cache，保存来源、ETag、license/attribution metadata。
- 使用 Open-Meteo 获取用户手动选择城市坐标的 current weather；不请求 GPS、IP geolocation 或后台位置。坐标出站前四舍五入到四位小数，成功结果缓存 30 分钟且同一位置 30 分钟内最多请求一次；过期值不得进入新 AI context，存储副本最多保留 7 天。
- 任一 provider 失败时使用未过期 cache；无 cache 则省略增强字段，不影响曲目 eligibility、播放或开播。

## Alternatives considered

- **只使用本地标签/封面**：隐私和离线最好，但无法改善缺失/混乱 metadata。
- **Spotify/Apple Music catalog metadata**：可能更丰富，但需要账号授权、catalog 条款与额外身份/数据边界。
- **上传音频 fingerprint 到识别服务**：匹配更强，但会发送音频派生数据并增加条款、隐私和成本。
- **系统 GPS/IP 推断天气位置**：引导更少，但超出“手动城市”隐私边界。
- **把天气作为硬依赖**：会让外部可用性影响核心电台体验。

## Consequences

- 必须遵守 provider User-Agent、限流、缓存和 attribution/许可证要求，并在发布前复核条款。
- 需要保存原始标签与增强标签的 provenance，UI 不应把低置信度 suggestion 当作事实。
- provider cache 可删除/重建；测试必须覆盖 429、timeout、no-match、低置信度和离线。
- 用户可以关闭元数据/天气增强；关闭后不再发新请求，已有 cache 按隐私删除规则清理。
- 更换 provider 或发送更多字段属于 Privacy/Integration 改变，必须更新文档和 ADR。
