# CyberKindred 依赖政策

| Metadata | Value |
|---|---|
| Status | Approved |
| Owner | Core Architecture & Security |
| Last Verified | 2026-09-02 |
| Source of Truth For | 允许、条件允许和禁止的依赖类别，版本锁定、许可证与新增生产依赖审批流程 |
| Related Documents | [Architecture](ARCHITECTURE.md), [ADR-0001](adr/ADR-0001-tauri-react-rust.md), [ADR-0004](adr/ADR-0004-provider-abstractions.md), [Threat Model](../security/THREAT-MODEL.md), [Legal and Licensing](../security/LEGAL-AND-LICENSING.md), [Development Guide](../operations/DEVELOPMENT-GUIDE.md) |

## 1. 原则

依赖必须服务于明确的 `FR-*`/`NFR-*`/`ARCH-*`，采用最小 feature set，能在 Windows x64 的 hermetic CI 中构建与测试。标准库或现有依赖能安全完成时不新增包。任何依赖都不获得超出其职责的 WebView、文件、网络、secret 或 OS 权限。

状态含义：

- **Approved**：可在已批准任务中直接采用，但仍需版本、许可证与安全检查。
- **Conditional**：满足表中条件并在 PR/任务记录依据后可采用；影响公共契约或架构时须新增 ADR。
- **Prohibited**：v1 不得采用。若确需改变，必须先更新需求、威胁模型、本文与 ADR，并取得用户明确批准。

## 2. Approved 生产依赖

M2 初始化必须采用 §2.3 的 Documentation Baseline v1 exact pins，随后以提交的 lockfile 解析版本为构建审查事实。

### 2.1 WebView/前端

| 依赖 | 用途 | 约束 |
|---|---|---|
| `react`, `react-dom` | UI 渲染 | React 组件不得直接访问文件、HTTP、Credential 或 Windows API。 |
| `typescript` | 静态类型 | `strict=true`，禁止 unchecked IPC cast。 |
| `vite` + 官方 React plugin | 构建 | 仅打包本地资源；生产 CSP 禁止远程脚本和 eval。 |
| `@tauri-apps/api` | typed IPC/event 基础 | 只通过项目封装 client 调用 allowlisted commands。 |
| `@tanstack/react-query` | command snapshot/cache 协调 | 只缓存脱敏 view model；应用重载后以 Rust snapshot 为事实。 |

UI 字体 `Space Grotesk`、`Space Mono`、`Doto` 作为本地静态资产按 SIL Open Font License 打包，不通过 Google Fonts/CDN 运行时加载。Nothing Design 仓库作为设计规范来源，不把其中未审查代码作为生产依赖。

### 2.2 Rust Core

| 依赖 | 用途 | 必须启用/禁用的边界 |
|---|---|---|
| `tauri` 2.x | 桌面 shell、IPC、window/tray | 最小 feature；capabilities 文件只开放列明命令。 |
| 官方 Tauri `dialog`, `notification`, `autostart`, `single-instance` plugins | 系统选择框、通知、自启动、单实例 | 从 Rust application service 封装；不把通用 fs/shell 权限暴露给 WebView。 |
| `tokio` | async runtime、channel、取消协调 | 只启用实际需要的 rt/time/sync/net/macros；阻塞音频/文件工作进入 bounded blocking pool。 |
| `serde`, `serde_json` | DTO/受控 JSON | secret 类型禁止 Serialize；未知公共契约字段按 schema 策略拒绝。 |
| `thiserror` | 内部 typed error | 对外先映射稳定 `ERR-*`，不透传 cause/body。 |
| `tracing`, `tracing-subscriber` | 结构化日志 | 使用 redact layer；禁止 body/header/path 字段。 |
| `sqlx` (`sqlite-bundled`, `runtime-tokio`, `migrate`, `macros`) | SQLite repository/migration | 禁用 default features；0.9 的 SQLite-only 路径不启用 TLS/MySQL/PostgreSQL/load-extension；查询在 repository；单 writer 配置。 |
| `reqwest` (`rustls`, `json`) | provider HTTPS | `0.13` 使用实际 feature 名 `rustls`；禁用 default/native-tls 与自动 redirect，响应通过有界 chunk 读取，未使用的 `stream` feature 不启用；统一 timeout、限流与脱敏；不允许 UI 提供任意 URL。 |
| `url` | HTTPS/base URL 与 provider URL 验证 | 生产拒绝非 HTTPS、credential-in-URL 和非官方/非用户明确确认的当前 configured origin。 |
| `rodio` | 本地 output/sink | 只在 playback actor 中持有。 |
| `symphonia` | MP3/FLAC/M4A/MP4/AAC/WAV/OGG probe/decode | 只启用所需 codec/container；不写原文件。 |
| `lofty` | 本地 tag/embedded cover 读取 | 只读打开；metadata 修改功能不进入 v1。 |
| `windows` (`windows-rs`) | GSMTC、Credential Manager、power/必要 Win32 API | feature 精确到所用 namespace；所有 handle RAII。Credential Manager FFI 的 `unsafe` 只允许存在于内部 `crates/windows-credential` infrastructure crate，经窄 safe API 暴露；产品 crate 继续 `forbid(unsafe_code)`。 |
| `uuid` (`v7`, `serde`) | 主键/request ID | ID 在 Rust 生成，不信任 UI 提供的 owner ID。 |
| `chrono`, `chrono-tz` | UTC、IANA timezone、DST 日程计算 | 数据库存 UTC ms + IANA zone；不依赖 OS locale 字符串运算。 |
| `sha2`, `hex` | cache/source/content hash | 不作为密码哈希；不把 secret 放入普通 hash。 |
| `secrecy`, `zeroize` | 进程内 secret 包装与尽力清零 | 包装类型不实现 Clone/Debug/Serialize；使用范围最小化。 |
| `jsonschema` | machine contract validation | 固定支持仓库 schema draft；编译 schema 失败使启动/测试失败。 |
| `mime`, `infer` | provider 音频/封面格式检查 | MIME 与 magic bytes 必须同时校验。 |

项目不采用通用 OpenAI SDK：v1 使用受控 `reqwest` adapter 实现 Responses 与 Audio Speech 所需最小 surface，便于强制 `store:false`、schema、timeout、日志脱敏和测试 fixture。

### 2.3 Documentation Baseline v1 exact pins

下列版本在 2026-09-02 通过 [npm registry](https://www.npmjs.com/) 与 [crates.io](https://crates.io/) 核对，是 M2 必须写入 manifest 的直接依赖版本。`Cargo.lock`/`pnpm-lock.yaml` 解析出的传递版本才是构建事实；若 M1 探针证明任一 pin 不兼容，只能通过单独依赖变更更新本节、风险与验证证据，不得在脚手架命令中静默选择其他版本。

| npm package | Exact version | Role |
|---|---:|---|
| `react`, `react-dom` | `19.2.8` | UI runtime |
| `@tauri-apps/api` | `2.11.1` | typed IPC/event client |
| `@tanstack/react-query` | `5.102.8` | snapshot/query coordination |
| `typescript` | `7.0.2` | compiler |
| `vite` | `8.2.2` | frontend build |
| `@vitejs/plugin-react` | `6.1.1` | React transform |
| `@tauri-apps/cli` | `2.11.4` | desktop build CLI |
| `@tauri-apps/plugin-dialog` | `2.7.3` | dialog JS bindings |
| `@tauri-apps/plugin-notification` | `2.4.0` | notification JS bindings |
| `@tauri-apps/plugin-autostart` | `2.5.1` | autostart JS bindings |
| `@types/react`, `@types/react-dom` | `19.2.18`, `19.2.5` | React/DOM compile-time types |
| `@types/node` | `24.13.3` | Node/Vite configuration compile-time types |
| `oxlint` | `1.81.0` | TypeScript/React static lint；只作为开发依赖，不启用 optional type-aware plugin |
| `ajv`, `ajv-formats` | `8.20.0`, `3.0.1` | Node-only Draft 2020-12 contract tests；不得打入 WebView runtime bundle |

本地字体不通过 npm 解析：Doto、Space Grotesk 和 Space Mono 固定取自 `google/fonts` commit `f6b2b7e8545e086ad3f821af21895d732b6485cf`，文件级来源、copyright、OFL-1.1 文本与 SHA-256 记录在 `src/assets/fonts/MANIFEST.json`。升级该 commit 必须同时重新核对许可证和所有二进制哈希。

| Rust crate | Exact version | Rust crate | Exact version |
|---|---:|---|---:|
| `tauri` | `2.11.5` | `tauri-build` | `2.6.3` |
| `tokio` | `1.53.1` | `tauri-plugin-dialog` | `2.7.3` |
| `tauri-plugin-notification` | `2.4.0` | `tauri-plugin-autostart` | `2.5.1` |
| `tauri-plugin-single-instance` | `2.4.4` | `serde` | `1.0.229` |
| `serde_json` | `1.0.151` | `thiserror` | `2.0.20` |
| `tracing` | `0.1.44` | `tracing-subscriber` | `0.3.23` |
| `sqlx` | `0.9.0` | `reqwest` | `0.13.4` |
| `url` | `2.5.8` | `rodio` | `0.22.2` |
| `symphonia` | `0.6.1` | `lofty` | `0.25.1` |
| `windows` | `0.62.2` | `uuid` | `1.26.0` |
| `chrono` | `0.4.45` | `chrono-tz` | `0.10.4` |
| `sha2` | `0.11.0` | `hex` | `0.4.3` |
| `secrecy` | `0.10.3` | `zeroize` | `1.9.0` |
| `jsonschema` | `0.52.1` | `mime` | `0.3.17` |
| `infer` | `0.22.0` |  |  |

`TASK-002` 的依赖图确认 `rodio 0.22.2` 内部仍解析到 `symphonia 0.5.5`，与项目 direct pin `symphonia 0.6.1` 并存且不能类型级合并。M1 探针只在可丢弃交互播放中使用 `rodio::Decoder`；产品路径必须保持 `rodio` 负责 output/player、direct `symphonia 0.6.1` 负责流式 decode、`lofty 0.25.1` 负责只读标签/封面的 ARCH-005 边界。`TASK-014` 若不能以有界内存、可 seek 的 `Source`/mixer adapter 落实该边界，须在实现前按依赖变更流程对齐版本或提交 ADR，不得静默复制探针的双 decoder 路径。

## 3. Approved 开发与测试依赖

| 依赖/工具 | 用途 | 约束 |
|---|---|---|
| `vitest`, React Testing Library, `@testing-library/user-event`, `jsdom` | TypeScript 单元/组件测试 | 禁止真实网络；fake IPC。 |
| `playwright` | 浏览器态前端流程、截图与 axe 驱动 | 只验证 React 页面；不把浏览器结果当作打包后的 Tauri/Windows 行为证据。 |
| Repository-owned Node 24 W3C WebDriver harness + `tauri-driver` | 打包后的 Windows desktop E2E | harness 只使用 Node built-ins 与 loopback W3C WebDriver；不引入浏览器自动化 npm 依赖，不在安装期下载 driver；使用 fake provider，真实 Apple Music 用例显式标签、默认跳过。 |
| `axe-core` / `@axe-core/playwright` | 自动无障碍规则 | 仅 dev dependency，不进入安装包；保留 MPL-2.0 notices，不修改上游文件。 |
| Rust `proptest` | 状态机/候选/保留 property tests | 固定 seed 可复现失败。 |
| Rust `wiremock` | provider integration fake | 只绑定 loopback 随机端口。 |
| Rust `tempfile` | 测试目录/数据库 | 自动清理，不指向用户目录。 |
| Rust `insta` | 脱敏 DTO/prompt shape snapshot | snapshot 不得含用户数据或绝对路径。 |
| `cargo-deny`, `cargo-audit`, `pnpm audit` | license/advisory/duplicate 检查 | CI 和 release 必跑；例外必须有到期日与风险记录。 |
| `cargo-nextest` | Rust test runner | 可选执行工具，不改变测试语义。 |

测试包同样固定：`vitest 4.1.11`、`@vitest/coverage-v8 4.1.11`、`@testing-library/dom 10.4.1`、`@testing-library/react 16.3.3`、`@testing-library/user-event 14.6.6`、`@testing-library/jest-dom 7.0.1`、`jsdom 30.0.1`、`ajv 8.20.0`、`ajv-formats 3.0.1`、`playwright 1.62.1`、`axe-core/@axe-core/playwright 4.13.0`；Rust dev-dependencies 固定 `proptest 1.11.0`、`wiremock 0.6.5`、`tempfile 3.27.0`、`insta 1.48.0`。Desktop E2E client 固定为 repository-owned Node 24 built-in harness，不解析 WebdriverIO/Selenium npm tree；AJV 仅在 Node contract suite 中编译 schema，禁止把其动态代码生成器导入 `src/` 或 WebView bundle。CI toolchain binary 固定为 `tauri-driver 2.0.6`、`cargo-nextest 0.9.143`、`cargo-deny 0.20.2`、`cargo-audit 0.22.2` 与 `cargo-llvm-cov 0.9.0`；M2 在工具清单记录 registry source、安装版本与本机 executable SHA-256，它们不进入应用依赖图。

M2 `RISK-015` review removed the pinned WebdriverIO 9.31.5 development chain after the locked tree reported unmitigated High advisories. The replacement preserves direct Windows `tauri-driver` coverage through the standardized W3C protocol, adds no dependency or application capability, and keeps renderer-only Playwright evidence explicitly separate.

## 4. Conditional 依赖

| 类别/候选 | 使用条件 |
|---|---|
| 新 Tauri 官方 plugin | 能证明手写实现风险更高；capability 最小；前端不获得通用 fs/shell/process；更新 threat model。 |
| `keyring` 等 credential wrapper | `windows-rs` 直接实现经 probe 证明不可维护，且 wrapper 在 Windows 明确使用 Credential Manager、无明文 fallback、无 secret logging；需 ADR。 |
| 状态管理库（如 Zustand/Redux） | React Query + reducer 无法满足经 profile 证明的状态复杂度；不得复制 Rust authoritative state；需前端架构说明。 |
| JSON Schema code generation | 生成结果可复现、CI 验证无 drift、不会让生成器替代仓库 schema 的权威性；仅开发依赖。 |
| Crash reporting/telemetry SDK | 先新增 opt-in 产品需求、隐私政策、数据字典、redaction 测试与关闭路径；须用户批准。v1 默认没有遥测。 |
| Auto updater/signing service | 发布范围确定、密钥托管与回滚方案批准后采用；须用户批准和 ADR。 |
| 新外部 provider SDK | 比最小 HTTP adapter 明显降低协议/安全风险；必须支持 custom HTTP client、timeout、proxy policy、redaction 与 hermetic fake；须 provider ADR。 |
| 原生 codec 或 C/C++ binding | Symphonia 无法覆盖已批准格式且有测试文件证明；需维护、签名、漏洞响应与许可证评估；须用户批准。 |
| GPL/LGPL/MPL 组件 | 法律文档确认分发义务、动态/静态链接方式及 source offer；MPL 文件级义务可接受后采用，LGPL/GPL 需用户批准。 |
| 非官方 Web/API reverse engineering 包 | 仅可用于一次性、本地、read-only feasibility probe 且不纳入产品或版本控制；产品采用一律禁止。 |

## 5. Prohibited 依赖与能力

- Electron、另一个桌面 runtime 或在本地启动通用 Web server 替代已选 Tauri IPC。
- `ffmpeg` binary bundling、下载即执行的 codec、未签名闭源 native DLL，除非按 Conditional 流程重新批准。
- 通用 shell/process execution plugin、向 WebView 开放任意文件系统、SQLite、HTTP、Credential 或 PowerShell。
- Apple Music Web DOM 自动化、browser cookie/token 抓取、非官方 Apple Music/网易云逆向 API 客户端。
- 把 API Key 写入 `.env`（发行态）、localStorage、IndexedDB、SQLite、日志或崩溃报告的库。
- 运行时 CDN 字体、远程 JavaScript、动态插件下载、`eval`/`new Function`、任意 remote module。
- AGPL/SSPL/Commons Clause 或无法确定许可证/来源的生产组件；未授权音频、封面和字体 fixture。
- 默认上传 analytics/session replay/屏幕/麦克风/文件路径的 SDK。
- 绕过 Rust provider/source/state-machine 边界、允许模型直接执行系统动作的 agent/tool framework。

## 6. 版本与锁定

- Node 固定 `24.19.0`，pnpm 固定 `11.16.0`；M2 在仓库 tool version 文件与 `packageManager` 写入这些 exact release，变更必须作为独立工具链任务。
- Rust 固定 `1.98.0`；M2 在 `rust-toolchain.toml` 写入该 exact release，MSRV 等于该 release，升级作为独立任务。
- `package.json` 的直接 production/dev dependency 使用 exact version，不用 `*`、`latest`、git branch 或未锁 URL。`pnpm-lock.yaml` 必须提交且 CI 使用 `--frozen-lockfile`。
- `Cargo.toml` 直接依赖使用明确 semver minor；`Cargo.lock` 对桌面应用必须提交，CI 使用 `--locked`。禁止 git/path production dependency，workspace 内部 path 除外。
- 更新按单一生态、单一目的拆分；不得自动合并。patch/minor 运行全量门槛，major 先评估 contract/behavior/migration 并视影响新增 ADR。
- Tauri CLI、Rust crate 和 JS API 保持同一兼容 release line；rodio/Symphonia format feature 变更必须重跑音频 fixture matrix。

## 7. 许可证与供应链门槛

1. 允许默认许可证族：MIT、Apache-2.0、BSD-2/3-Clause、ISC、Unicode-3.0、Zlib、SIL-OFL-1.1；其他许可证按 Conditional 流程。
2. `cargo-deny` 与 JS license report 必须覆盖 transitive dependency；release 生成第三方 notices 与 CycloneDX SBOM。
3. 包只从 crates.io、npm registry 和官方 Rust/Node distribution 获取；启用 lockfile integrity/checksum。禁止 install script，确需时逐包 allowlist 并说明输出与网络行为。
4. `cargo audit`、RustSec、`pnpm audit` 的 known exploited/critical/high 未缓解漏洞阻止 release。临时例外必须在 Risk Register 记录影响、不可达证据、owner、到期日（最长 30 天）。
5. 发布构建使用 clean checkout、frozen/locked dependencies；构建期间除 registry cache miss 外不访问任意网络，应用资源全部本地。

## 8. 新增依赖审批清单

任务 owner 在变更前记录：对应需求/ARCH ID、为何现有依赖不足、production/dev、维护活跃度、transitive 数量、许可证、advisory、网络/文件/secret/unsafe 权限、binary size、替代方案、移除策略和测试证据。

以下任一情况属于**高风险生产依赖**，必须暂停并询问用户：新增外部数据目的地；secret/credential；浏览器/DOM automation；shell/process；任意文件访问；native/unsafe/codec；遥测；自动更新/签名；强 copyleft/未知许可证；远程代码/模型工具执行；扩大 Tauri capability。获批后还必须更新 Threat Model、Legal and Licensing、Risk Register 与 ADR。

## 9. CI 验证

- Rust：`cargo fmt --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo nextest run --locked`、`cargo deny check`、`cargo audit`。
- JS：`pnpm install --frozen-lockfile`、typecheck、lint、unit/E2E、`pnpm audit`、license report。
- 自定义 policy test 拒绝 production git dependency、非 HTTPS remote asset、未批准 Tauri capability、secret-named SQLite setting、CSP 中的 remote script/eval。
- release diff 必须包含 lockfile/SBOM/notices 的可解释变化；无对应 manifest 变更的 lockfile 大范围漂移阻止合并。
