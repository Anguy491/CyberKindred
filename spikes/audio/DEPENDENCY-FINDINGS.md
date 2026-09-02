# TASK-002 dependency findings

| Field | Value |
|---|---|
| Status | Approved |
| Owner | Playback Architecture |
| Last Verified | 2026-09-02 |
| Source of Truth For | M1 本地音频探针观察到的 rodio、Symphonia、lofty 职责与版本边界 |
| Related Documents | `README.md`, `../../docs/architecture/ARCHITECTURE.md`, `../../docs/architecture/DEPENDENCY-POLICY.md`, `../../docs/architecture/adr/ADR-0002-local-and-system-media-sources.md` |

## Conclusion

现有架构选择无需新增 ADR：产品边界仍是 rodio 只拥有 Windows 输出/player，直接依赖的 Symphonia 负责容器与 codec 解码，lofty 负责只读标签、封面和文件属性。探针没有引入 native codec、FFmpeg 运行时、网络、写标签或 WebView 音频权限。

| Dependency | Probe responsibility | Product boundary carried forward |
|---|---|---|
| `symphonia 0.6.1`（direct） | 用最小 format/codec features 对 MP3、FLAC、M4A/MP4、AAC、WAV、OGG 逐 packet 完整解码，报告样本数、错误、峰值、耗时和相对实时倍率 | 扫描/播放预检的权威 decoder；流式适配不得把整个用户音频复制到应用数据或一次性载入内存 |
| `lofty 0.25.1` | 只读报告 duration、采样率、声道、bitrate、bit depth，以及标签/封面是否存在；不输出正文 | 标签与内嵌封面唯一 parser；不修改原文件，不替代 Symphonia codec 解码 |
| `rodio 0.22.2` | CPAL 输出设备枚举、默认设备打开、`Player` 的 pause/play/seek、输出重建和 stream error callback | 只在 playback actor 持有一个 output owner；设备恢复后保持暂停，用户再次 Play 才出声 |

## Version graph finding

`cargo tree` 证明 `rodio 0.22.2` 自身依赖 `symphonia 0.5.5`，不能与项目直接固定的 `symphonia 0.6.1` 合并为同一个 crate instance。探针为了快速人工验证 rodio 的交互控制，在可丢弃 `play` 命令里使用 `rodio::Decoder`，因此 lockfile 同时包含两条 Symphonia 版本线；`matrix` 的兼容性结论只来自直接的 0.6.1 解码路径。

这不是产品实现的许可：`TASK-014` 应把直接 Symphonia 0.6.1 的流式样本通过受控 `rodio::Source`/mixer 适配到 rodio 输出，或在进入实现前通过单独依赖变更让版本线对齐。不得把探针中的 `rodio::Decoder` 静默复制到产品路径并声称满足 0.6.1 exact pin。若届时无法形成有界内存、可 seek 的适配器，才需要更新 Dependency Policy、风险和 ADR 后请求用户批准。

## Fixture observation scope

M1 只包含每类一个 12 秒原创生成 fixture、三类内嵌封面（MP3/FLAC/M4A）和一个损坏 MP3。AAC/ADTS 没有通用标签容器，报告零标签是预期结果。该矩阵证明依赖可行性，不替代 `NFR-COMPAT-002` 每格式至少五个标签/封面组合的 M3 产品验收。
