# Operations Runbook

| Field | Value |
|---|---|
| Status | Approved |
| Owner | Support Steward |
| Last Verified | 2026-09-02 |
| Source of Truth For | 本地故障诊断、恢复、备份和数据清除 |
| Related Documents | `../architecture/DATA-MODEL.md`, `../security/PRIVACY-DATA-LIFECYCLE.md`, `BUILD-RELEASE.md` |

## Safety first

任何诊断先复制而非修改数据，记录 app/schema/Windows/Apple Music/WebView2 版本。日志和导出在分享前必须脱敏。不得要求用户提供 API Key、完整对话、音乐文件或整个数据库。

## Application will not start

1. 确认支持的 Windows 和 WebView2 Runtime。
2. 从版本化安装目录启动并读取最新 redacted log。
3. 若疑似 DB，先备份整个 app data 目录，再使用内置 integrity check；不要手工编辑 SQLite。
4. 若 UI bundle/CSP 失败，重新安装同版本并保留 app data。

## Database corruption or migration failure

1. 停止应用并复制 DB、`-wal`、`-shm` 到带时间戳的恢复目录。
2. 运行只读 integrity check，记录 schema version 和最近 migration。
3. 优先恢复自动 migration backup；若无可用备份，使用应用导出/重建工具而非任意 SQL 修补。
4. 重建会丢失内容必须在执行前列明；Credential Manager secret 不在 DB 备份中。

## OpenAI unavailable

- `401/403`: 标记凭据无效并引导重新测试，绝不显示 key。
- `429`: 尊重 retry-after，停止生成新串场；继续现有/确定性本地队列。
- timeout/5xx: 有界重试后进入降级，不循环弹窗；文字/音乐保持可用。
- TTS 失败: 跳过音频串场并显示文字，不停止音乐。

## Apple Music session unavailable

1. 确认 Apple Music Windows App 已安装、登录且至少开始播放一次。
2. 刷新 session list，检查 SourceAppUserModelId 和 runtime capabilities。
3. 若无会话，显示连接说明并保持 Local Source 可用；不尝试 DOM automation。
4. 若控制返回 false，隐藏对应控制并记录 capability；不持续重试争夺播放。
5. 串场期间会话切换或用户改变状态时不自动恢复。

## Audio output changed or playback stalled

1. 保存队列与当前位置 checkpoint。
2. 重新枚举默认输出并重建 audio sink；失败曲目标记本次不可用后跳过。
3. 设备恢复后从 checkpoint 继续；不得同时创建两个活跃 music sink。

## Library files moved or damaged

- 增量重扫 selected roots；缺失文件标记 unavailable，不立即删除历史和反馈。
- 损坏/不支持文件隔离到扫描报告；不修改源文件。
- 用户确认移除 root 或清理 missing entries 后才删除索引记录。

## Metadata or weather outage

只使用仍在各 provider 新鲜期内的缓存；缓存不存在或已过期时省略增强/天气。MusicBrainz 429 必须停止请求并 backoff，不能提高并发绕过限制。外部服务恢复不应改变用户原始标签。

## Backup, export and reset

- Backup: 应用关闭后复制 SQLite 与非敏感配置；artwork/TTS cache 可重建，可不备份。
- Export: 生成不含 secret 的版本化 JSON，逐类列出 profile、approved memory、proposal status、summary、conversation retention range/count、settings、playback 与 feedback；不包含对话正文、音频或绝对路径。
- Reset conversations: 预览后删除原文、摘要和由这些内容产生的 pending proposals；显式 profile 与 approved memory 不在该类别中。
- Reset all: 停止任务并删除 DB/WAL/SHM、migration backup、cache、生成音频、logs、config、全部 CyberKindred Credential Manager origin entries、日程通知与自启动；不得留下 quarantine、回收站副本或应用可恢复备份。重启后验证首次引导状态，原始音乐和用户主动保存的外部导出保持不变。

## Diagnostic bundle

只包含版本、capability、错误码、迁移状态、性能摘要和 redacted logs；默认排除绝对音乐路径、标题/艺术家、城市、对话、画像和所有 token。
