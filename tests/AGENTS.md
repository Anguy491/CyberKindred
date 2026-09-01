# Test Agent Instructions

| Field | Value |
|---|---|
| Status | Draft |
| Owner | QA Steward |
| Last Verified | 2026-09-01 |
| Source of Truth For | `tests/` 内的测试执行约束 |
| Related Documents | `../AGENTS.md`, `../docs/testing/TEST-STRATEGY.md`, `../docs/testing/TRACEABILITY.md` |

- 默认测试必须 hermetic、可重复、可离线运行，不调用真实付费 API 或用户音乐库。
- fixture 必须自制、明确许可或程序生成；不得提交用户音乐、Apple Music 内容或真实对话。
- contract tests 以 `../docs/contracts/schemas/` 为权威；合法与边界样例必须通过，非法样例必须失败。
- 真实 Apple Music、性能和联网测试使用显式标签并记录环境、版本与证据；不得成为普通单元测试前置条件。
- 每个测试引用至少一个 `FR-*` 或 `NFR-*`，并在 `TRACEABILITY.md` 中登记。
- 失败报告应包含可重现步骤、期望、实际和脱敏日志，不以仅更新快照替代行为判断。
