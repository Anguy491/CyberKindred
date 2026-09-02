# Manual-TASK-002 操作手册

| Field | Value |
|---|---|
| Status | Draft |
| Owner | Quality Engineering |
| Last Verified | 2026-09-02 |
| Source of Truth For | `TASK-002` 本地音频解码、播放、seek 与默认设备切换的唯一任务级人工验收步骤 |
| Related Documents | `README.md`, `DEPENDENCY-FINDINGS.md`, `fixtures/LICENSE.md`, `../../docs/testing/TEST-STRATEGY.md` |

## 1. 结论规则

测试人员必须在真实 Windows 输出设备上逐项执行本手册并保存去标识证据。全部勾选后才能把 `TASK-002` 从 `Review` 改为 `Done`；任一失败都记录环境、步骤、预期、实际和脱敏日志，任务退回 `In Progress`。静态构建结果不能替代听感和设备切换结果。

本手册只判定 M1 技术可行性。它不声称完成 UI P95、200 次控制、20 次系统变化、每格式五个 fixture 或产品 playback actor 验收；这些仍由后续 `TEST-RAD-002`、`TEST-APL-004`、`TEST-LIB-001` 和相关任务承担。

## 2. 准备

1. 使用 Windows 10 22H2 或 Windows 11 x64，Rust 1.98.0。
2. 准备两个可明确分辨的输出端点，例如显示器扬声器和有线/蓝牙耳机。
3. 将两个端点音量调到安全低音量；不要使用用户音乐，全部步骤只用仓库生成 fixture。
4. 在仓库根目录打开 PowerShell，创建本地证据目录：

```powershell
New-Item -ItemType Directory -Force .\spikes\audio\evidence-local | Out-Null
```

5. 在 `spikes/audio/evidence-local/RESULT.md` 本地副本记录：测试人代号、开始/结束时间、Windows 版本、机器/VM 去标识代号、Rust 版本、当前 commit 和两个输出设备的显示名。该目录已被 Git 忽略，不提交原始证据。

## 3. 构建和 fixture 哈希矩阵

执行：

```powershell
cargo build --locked --release --manifest-path .\spikes\audio\Cargo.toml
.\spikes\audio\target\release\cyberkindred-audio-probe.exe matrix | Tee-Object .\spikes\audio\evidence-local\matrix.jsonl
```

检查最后一个 `command=matrix` 对象：

- [ ] `status` 为 `pass`，`supportedFormatsPassed` 为 `6`。
- [ ] 六个正常 fixture 的 `hashMatchesManifest` 均为 `true`，`decodedSamples` 大于 0，`recoverablePacketErrors` 为 0。
- [ ] `corruptFixtureIsolated` 与 `manifestComplete` 均为 `true`；损坏文件分类为 `unsupported_or_corrupt_format`，但不影响另外六项。
- [ ] MP3、FLAC、M4A 的 `artworkCount` 至少为 1；其 title/artist/album presence 均为 `true`。AAC 零标签是预期结果。
- [ ] 保存每类 `elapsedMs`、`mediaToDecodeRatioX100` 和 `fileBytes`；这些是探针资源数据，不套用产品性能阈值。

## 4. 初始静音和六格式控制

对 `mp3`、`flac`、`m4a`、`aac`、`wav`、`ogg` 逐个执行以下命令，把 `<ext>` 替换为扩展名：

```powershell
.\spikes\audio\target\release\cyberkindred-audio-probe.exe play .\spikes\audio\fixtures\tone.<ext>
```

每次按以下顺序操作：

1. 启动后等待 3 秒，不输入命令。
2. 输入 `p`，听 2 秒；输入 `p` 暂停；等待 2 秒。
3. 输入 `s 6`，再输入 `r`；确认仍暂停且位置约为 6000 ms。
4. 输入 `p`，确认从较高音高的中段继续；输入 `q`。

逐格式勾选：

- [ ] MP3：启动静音；play/pause/seek/stop 可听且状态正确；无 stream error。
- [ ] FLAC：启动静音；play/pause/seek/stop 可听且状态正确；无 stream error。
- [ ] M4A/MP4：启动静音；play/pause/seek/stop 可听且状态正确；无 stream error。
- [ ] AAC：启动静音；play/pause/seek/stop 可听且状态正确；无 stream error。
- [ ] WAV：启动静音；play/pause/seek/stop 可听且状态正确；无 stream error。
- [ ] OGG：启动静音；play/pause/seek/stop 可听且状态正确；无 stream error。
- [ ] 所有控制事件的 `latencyMs` 小于 300 ms。这里只记录 CLI 到 rodio 控制返回的探针延迟，不把它冒充 UI/真实音频 P95。

## 5. 默认输出设备切换

先记录设备：

```powershell
.\spikes\audio\target\release\cyberkindred-audio-probe.exe devices | Tee-Object .\spikes\audio\evidence-local\devices-before.jsonl
.\spikes\audio\target\release\cyberkindred-audio-probe.exe play .\spikes\audio\fixtures\tone.flac
```

然后：

1. 确认启动静音，输入 `p` 开始播放。
2. 在 Windows“系统 > 声音 > 输出”把默认输出改为第二个端点。
3. 回到探针输入 `d`。探针应立即暂停旧 player，保存 position，打开当前默认输出，在保存位置建立新 player，并保持暂停。
4. 等待 3 秒，确认没有声音；检查事件 `default_device_switched` 的 `paused=true`、设备哈希改变、`detail=new_default_device_opened_paused`、`latencyMs<5000`。
5. 输入 `p`，确认声音来自新端点；输入 `r`，确认位置继续推进且 `streamErrorCount=0`。
6. 把 Windows 默认输出切回原端点，重复 `d`、静音等待和 `p`；最后输入 `q`。

判定：

- [ ] 两次设备哈希都随系统默认端点改变。
- [ ] 每次 `d` 后均保持暂停，未自行恢复声音。
- [ ] 再次输入 `p` 后只从新默认端点出声，position 从保存位置继续。
- [ ] 两次重建均在 5 秒内返回明确成功/失败事件，无未分类错误。

## 6. 错误隔离

执行：

```powershell
.\spikes\audio\target\release\cyberkindred-audio-probe.exe play .\spikes\audio\fixtures\corrupt.mp3
.\spikes\audio\target\release\cyberkindred-audio-probe.exe play .\spikes\audio\fixtures\tone.wav
```

- [ ] 损坏输入以非零退出，输出 `unsupported_or_corrupt_format`/`preflight_failed`，没有打开输出或发声。
- [ ] 紧接着正常 WAV 仍可按 §4 播放、暂停、seek 和停止，证明坏文件未污染后续会话。

## 7. CPU 与内存记录

在一个 PowerShell 窗口运行 FLAC 并循环播放/暂停/seek；在第二个 PowerShell 窗口执行以下命令采样 10 秒：

```powershell
$probeProcess = Get-Process -Name cyberkindred-audio-probe
1..10 | ForEach-Object {
    $probeProcess.Refresh()
    [pscustomobject]@{
        At = Get-Date -Format o
        CpuSeconds = $probeProcess.CPU
        WorkingSetBytes = $probeProcess.WorkingSet64
        PrivateBytes = $probeProcess.PrivateMemorySize64
    }
    Start-Sleep -Seconds 1
} | Export-Csv -NoTypeInformation .\spikes\audio\evidence-local\resources.csv
```

- [ ] `resources.csv` 有 10 个有效样本，且进程在采样期间没有退出、无持续增长到系统失去响应。
- [ ] 将工作集/私有内存最大值和 CPU 增量抄入本地 `RESULT.md`。M1 只记录数据，不以它替代 `NFR-PERF-005` 的正式 WPR 门槛。

## 8. 最终签字

- [ ] 上述全部项目通过，证据文件可打开且没有用户音乐、绝对用户路径、用户名或其他个人信息。
- [ ] `DEPENDENCY-FINDINGS.md` 中的双 Symphonia 版本事实已在结果中复核：matrix 报告 direct 0.6.1，rodio transitive 0.5.5。
- [ ] 测试人员在本地 `RESULT.md` 写明 `Manual-TASK-002: PASS`、结束时间和去标识签名；若失败则写 `FAIL` 及最小复现。

完成后把去标识汇总（不含原始设备标识、绝对路径或个人数据）提供给维护者，由维护者更新 Backlog、Traceability、ADR-0002 M1 validation note 和 Changelog；不要直接提交 `evidence-local/`。
