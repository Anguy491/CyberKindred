# TASK-002 audio fixture license and provenance

| Field | Value |
|---|---|
| Status | Draft |
| Owner | Playback Architecture |
| Last Verified | 2026-09-02 |
| Source of Truth For | `spikes/audio/fixtures/` 中音频内容的来源、许可与生成边界 |
| Related Documents | `../README.md`, `../MANUAL-TEST.md` |

`tone.mp3`、`tone.flac`、`tone.m4a`、`tone.aac`、`tone.wav` 和 `tone.ogg` 的声音内容是本项目于 2026-09-02 程序化生成的单声道变频正弦波，不采样、不改编、也不包含任何第三方录音、音乐、封面或字体。CyberKindred 项目将这些生成内容按 [CC0 1.0 Universal](https://creativecommons.org/publicdomain/zero/1.0/) 放弃权利，允许在测试、研究和再分发中使用。

音频由开发机现有 FFmpeg 生成后提交为固定二进制 fixture；FFmpeg 仅是一次性开发工具，不随探针或产品打包，也不是运行时依赖。各文件的固定 SHA-256 见 `MANIFEST.sha256`。fixture 只覆盖 M1 每类一个短样本；`NFR-COMPAT-002` 要求的每格式至少五个标签/封面组合仍由 `TASK-011` 的产品级 fixture 集承担。

`corrupt.mp3` 是项目原创的纯文本损坏输入，只用于证明单文件错误隔离；它不包含音频内容。
