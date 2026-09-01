# Security Policy

| Field | Value |
|---|---|
| Status | Approved |
| Owner | Security Steward |
| Last Verified | 2026-09-02 |
| Source of Truth For | 仓库安全底线与安全问题处理流程 |
| Related Documents | `docs/security/THREAT-MODEL.md`, `docs/security/PRIVACY-DATA-LIFECYCLE.md` |

## Supported phase

项目尚处于文档基线阶段。进入实现后，只保障 `main` 最新提交与明确标记的内测制品；未发布分支不承诺安全更新。

## Never commit

- OpenAI、Apple 或其他服务的 API Key、private key、token、cookie。
- 用户对话、画像导出、数据库、日志或本地音乐。
- 包含真实用户目录的配置、截图或测试快照。

疑似 secret 必须停止提交、撤销凭据并检查历史；不得仅从最新文件删除后继续使用原凭据。

## Reporting

内测阶段通过私下联系项目所有者报告，包含影响、受影响版本、最小复现和建议缓解。不要在公开 issue 中附 secret、个人数据或可利用细节。

## Handling

安全问题先隔离和验证，再建立 `RISK-*` 与对应修复任务。修复必须包含回归测试、威胁模型更新和受影响数据说明。未经用户授权不得发布公告、制品或远端 issue。
