# Local audio probe

| Field | Value |
|---|---|
| Status | Draft |
| Owner | Playback Architecture |
| Last Verified | 2026-09-02 |
| Source of Truth For | `TASK-002` 探针用途、边界、命令和证据入口 |
| Related Documents | `MANUAL-TEST.md`, `DEPENDENCY-FINDINGS.md`, `../../docs/planning/BACKLOG.md` |

该 crate 是 M1 可丢弃技术探针，不进入 CyberKindred 产品运行路径。它用直接依赖的 Symphonia 0.6.1 完整解码六类生成 fixture，用 lofty 0.25.1 只读检查属性、标签与封面，并用 rodio 0.22.2/CPAL 枚举默认输出、建立播放器以及执行播放、暂停、seek 和默认设备重建。

探针启动后始终先保持暂停。切换默认设备会先暂停旧播放器，在保存位置重建新输出，并继续保持暂停；只有测试人员再次输入 `p` 才会出声。它不监听麦克风、不访问网络、不修改音频文件，也不输出绝对路径或标签正文。

## Requirements

- Windows 10 22H2 或 Windows 11 x64。
- Rust 1.98.0。
- 人工设备切换场景需要至少两个可用输出端点。
- 请先把系统音量调低；fixture 是 12 秒单声道变频正弦波。

## Commands

构建 release 探针：

```powershell
cargo build --locked --release --manifest-path .\spikes\audio\Cargo.toml
```

对固定哈希清单执行六格式解码/元数据矩阵，并确认损坏文件被隔离：

```powershell
.\spikes\audio\target\release\cyberkindred-audio-probe.exe matrix
```

枚举输出设备；设备 ID 只输出 SHA-256：

```powershell
.\spikes\audio\target\release\cyberkindred-audio-probe.exe devices
```

交互播放一个 fixture：

```powershell
.\spikes\audio\target\release\cyberkindred-audio-probe.exe play .\spikes\audio\fixtures\tone.flac
```

交互命令为 `p` 播放/暂停、`s <秒>` 绝对 seek、`d` 重建到当前 Windows 默认输出、`l` 枚举设备、`r` 输出状态、`q` 停止退出。

单文件只读检查：

```powershell
.\spikes\audio\target\release\cyberkindred-audio-probe.exe inspect .\spikes\audio\fixtures\tone.m4a
```

## Evidence and status

任务级验收只按 [Manual-TASK-002](MANUAL-TEST.md) 人工执行。`cargo fmt`、`cargo clippy` 和 `cargo build` 只证明实现可格式化、可 lint、可编译，不替代真实扬声器、听感、暂停/seek 或设备切换证据。人工证据未回填前，`TASK-002` 保持 `Review`，不得标记 `Done`。

原始运行记录写到已忽略的 `spikes/audio/evidence-local/`，不得提交含机器名、用户名、绝对路径或个人音频信息的证据。fixture 内容来源与许可见 [fixtures/LICENSE.md](fixtures/LICENSE.md)。
