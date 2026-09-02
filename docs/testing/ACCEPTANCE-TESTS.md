# CyberKindred Acceptance Tests

| Metadata | Value |
|---|---|
| Status | Approved |
| Owner | Quality Engineering |
| Last Verified | 2026-09-03 |
| Source of Truth For | Documentation Baseline v1 的端到端验收场景库存、证据格式和 beta 发布判定 |
| Related Documents | `docs/product/FRS.md`; `docs/product/NFRS.md`; `docs/testing/TEST-STRATEGY.md`; `docs/testing/TRACEABILITY.md`; `docs/planning/ROADMAP.md` |

## 1. 执行约定

- 本文件定义最终 beta 所需的完整场景库存，不要求在 M1–M6 的每个 checkpoint 全量执行。checkpoint subset 与递延规则以 `TEST-STRATEGY.md` 为准。
- 每个实际执行的场景都保存：版本、Windows 版本、机器/VM 标识、fixture 版本、开始与结束时间、结果和失败证据；未执行场景记为 `Not Run`，不能记为通过。
- `Automated` 场景在隔离环境中使用 fake provider；`Manual-Windows` 使用真实 Windows 行为但不得调用付费服务；`Manual-Live` 只有带 `CYBERKINDRED_LIVE_TEST=1` 标签并由测试人员主动执行时才可访问真实服务。
- 文中的“无声音”通过系统 loopback 捕获和应用事件日志共同判定；“无 secret”通过唯一 canary 值扫描判定。
- Apple Music 验收指 Windows App 的 GSMTC 会话。Web 版只验证“不做 DOM 自动化并给出指引”。
- M1–M6 只有当前 milestone 主路径或 prototype hard gate 失败才阻断 checkpoint；其他 `P0`/`P1` 场景可作为 known gap 递延。M7 beta release candidate 中任何 `P0` 关联场景失败都阻断，`P1` 必须在发布冻结前通过。

## 2. 引导与首次运行

| Test ID | Covers | Mode | Scenario and expected evidence |
|---|---|---|---|
| TEST-ONB-001 | FR-ONB-001, FR-ONB-002, FR-ONB-006 | Automated E2E | 首次启动依次走完七步；在每一步重启并返回。断言 API-002 的 `completedSteps` 始终为固定无重复前缀，恢复到首个未完成步骤、已填内容可编辑、音乐来源可选本地/Apple Music/两者；选择本地或两者时必须通过 API-011 native picker 授权至少一个目录，取消 picker 不伪造授权；Apple Music 路径不显示或请求 Apple Developer/MusicKit 凭据；非敏感画像可接受默认值。城市/日程选“暂不配置”时仍可完成该步且零网络/通知调用；选“配置”时必须取得 API-048→API-049 与 API-033 的专用响应，重启后结果保持。保存逐步 API-003 Ack、API-002 与专用 API 响应快照。 |
| TEST-ONB-002 | FR-ONB-003, FR-ONB-004, NFR-SEC-001 | Automated integration | 分别注入有效、无效、429 和断网 Key 验证结果。仅成功项写入 fake Credential Manager；验证并切换到第二个 canonical origin 后，第一个 origin credential 保留；API-005 只删除指定 origin，full reset 删除全部 origin。候选 credential/model 验证失败、验证成功后设置保存失败及事务回滚在重启后均不得改写旧配置的 integration status；验证与设置成功时同一 usage outcome 必须随设置事务提升并可在重启后恢复。SQLite、WebView 状态、IPC、崩溃报告、日志和导出对 canary 扫描为零命中。 |
| TEST-ONB-003 | FR-ONB-005, NFR-COST-001 | Automated E2E | 加载声音列表；`previewAvailable: false` 的声音不可触发预览。未点击时 TTS 调用为 0，可用声音点击预览只调用固定短句一次，播放中再次点击经 API-038 停止，选择结果持久化；切回 text-only 后 `ttsEnabled` 必须为 false。失败时显示可重试分类且无自动重播。对同一 `operationId` 重放相同 terminal event，前端只应用一次；若同一 operation 出现相冲突 terminal，则停止增量并重取权威状态。 |
| TEST-ONB-004 | FR-ONB-007, FR-RAD-001, NFR-PERF-001 | Automated E2E | 完成页逐项显示已选来源、各外发服务与数据、原始对话 30 天保留、通知后确认开播和记忆可见/可改/可删；隐私确认前“完成”不可用，确认后进入 `RADIO`。在基线机器分别执行冷启动与热启动各 10 次：从进程创建到可交互的 P95 冷启动≤5 s、热启动≤3 s；全部启动与引导完成时 loopback 保持静音。 |

## 3. 本地电台与曲库

| Test ID | Covers | Mode | Scenario and expected evidence |
|---|---|---|---|
| TEST-LIB-001 | FR-LIB-001, FR-LIB-003, NFR-COMPAT-002 | Automated integration | 扫描各 5 个 MP3、FLAC、M4A/MP4、AAC、WAV、OGG 许可样本以及损坏/不支持样本；验证标签、封面、时长、错误隔离、移动/修改/删除增量结果及不复制原音频。 |
| TEST-LIB-002 | FR-LIB-002, NFR-PERF-002 | Automated performance | 扫描固定 10,000 首曲库，总耗时≤15 分钟、UI 主线程最长任务≤100 ms、进度更新间隔≤500 ms；中途取消，确认≤1 秒被接受且已提交记录一致，再次增量扫描可完成。 |
| TEST-LIB-003 | FR-LIB-004, FR-LIB-005, NFR-PRIV-004, NFR-OFF-002 | Automated contract | 对 MusicBrainz fake 捕获请求，只允许文本标签且不含音频/路径；模拟低置信多候选，确认不覆盖原标签。验证搜索、状态过滤及原始/补全来源展示；阻断 metadata provider 后≤2 秒显示独立降级状态，缓存项仍显示 MusicBrainz 来源和缓存时间。 |
| TEST-LIB-004 | FR-LIB-006, FR-RAD-002 | Automated domain | 固定时钟和随机种子生成候选集；验证重复冷却、反馈、画像、批准记忆规则，且模型输出候选外 ID 时计划被修复或降级为有效真实 ID。 |
| TEST-RAD-001 | FR-RAD-001, FR-RAD-003 | Automated E2E | 用户点击开始 6 首本地节目；验证一次开场、常规串场间隔 2–3 首和所有声音由本次动作授权。后台日程到点但未确认时不调用 LLM/TTS/播放器。 |
| TEST-RAD-002 | FR-RAD-004, FR-RAD-005, NFR-PERF-003 | Automated E2E | 对播放/暂停/上一首/下一首/进度、喜欢/跳过/少说一点各执行 200 次；每次将 UI 显示的 Now Playing、来源、进度、节目状态与权威后端快照逐字段比对，并验证 capability 门控。输入到视觉确认 P95≤100 ms，输入到本地音频动作 P95≤300 ms。反馈事件立即持久化且历史事件不可改写；在下一次候选生成中断言喜欢提高、跳过降低对应偏好；本节目串场变为每 4–6 首至多一次。 |
| TEST-RAD-003 | FR-RAD-006 | Automated integration | 结束本地节目时停止本地/TTS 并记录原因；结束 Apple 陪伴时释放监听。测试结束瞬间第三方会话已被用户改变，确认不停止或改写新状态。 |
| TEST-RAD-004 | FR-RAD-007, NFR-REL-003, NFR-OFF-002 | Automated fault injection | 对 LLM/TTS 分别注入超时、429、5xx、无效 JSON 和离线；每个 Provider 故障在≤2 秒内显示独立降级状态。LLM 结构化失败最多重试一次，TTS 不自动重播且保留文字，本地队列继续，错误只显示一次且其他 Provider 状态不受影响。 |
| TEST-RAD-005 | NFR-REL-001 | Automated soak + manual audio | 固定 fixture 运行 100 次 30 分钟节目；断言无未处理异常/退出、非预期静音不超过 2 秒、相邻曲目转换成功率至少 99%，保存音频探针与事件时间线。 |

## 4. Apple Music 陪伴模式

| Test ID | Covers | Mode | Scenario and expected evidence |
|---|---|---|---|
| TEST-APL-001 | FR-APL-001 | Manual-Windows | 依次验证 App 未运行、仅 Web 打开、App 有会话、会话消失和能力受限；只接受 Apple Music Windows App GSMTC 会话，Web 场景不得注入/读取 DOM。保存会话诊断和截图。 |
| TEST-APL-002 | FR-APL-002, FR-APL-003, NFR-PERF-004 | Manual-Windows | 在真实 App 切换曲目、播放状态和可用控制各 50 次；显示仅来自会话的真实字段，未知字段不补写，控件随 capability 变化，49/50 次在 2 秒内且全部在 5 秒内。 |
| TEST-APL-003 | FR-APL-004, NFR-PRIV-004 | Manual-Live | 用户在 Apple Music 手动换歌，验证曲目感知反应由本地确定性文字模板产生，不创建或宣称远程精确队列；代理捕获中 GSMTC 元数据/播放事件及 OpenAI 请求数均为 0。保存 Program Plan、本地模板输出和曲目事件序列。 |
| TEST-APL-004 | FR-APL-005, NFR-REL-004 | Manual-Windows | 仅以不含 GSMTC/Apple Music 数据的通用文字执行 TTS，含 GSMTC 元数据的文本必须在提交前被拒绝。覆盖正常 TTS 暂停/恢复、用户在 TTS 中保持暂停、会话切换、会话消失、休眠/唤醒和设备切换；只有原会话且用户未改变状态时恢复，5 秒内进入明确状态。 |

## 5. 对话、记忆与人格

| Test ID | Covers | Mode | Scenario and expected evidence |
|---|---|---|---|
| TEST-CHAT-001 | FR-CHAT-001 | Automated E2E | 发送心情、偏好和“少说一点”；确认原始用户消息、简短回应、上下文输入和本节目频率变化，并保持输入发送前可编辑。 |
| TEST-CHAT-002 | FR-CHAT-002 | Automated safety | 请求查看屏幕、使用麦克风、读取定位或操控 Apple 内部推荐；响应明确说明限制并提供文字替代，不声称不存在的访问能力。 |
| TEST-CHAT-003 | FR-CHAT-003 | Automated integration | 在 LLM 流式请求中取消；后端中止或丢弃结果，不创建 AI 回复或记忆提案，用户原文标记已取消且音乐不中断。 |
| TEST-MEM-001 | FR-MEM-001, FR-MEM-002, NFR-PRIV-005 | Automated contract + E2E | 同时创建画像、趋势、未决提案和批准记忆；验证分区与来源。未批准提案不进入新节目上下文，批准后下一次组装才进入。 |
| TEST-MEM-002 | FR-MEM-003 | Automated E2E | 对提案依次执行批准、编辑后批准、拒绝；只使用最终批准文本，拒绝项不再次出现，原提案只保留最小审计状态。 |
| TEST-MEM-003 | FR-MEM-004, NFR-PRIV-005 | Automated regression | 编辑、停用、启用、删除记忆；每次下一轮上下文立即反映。删除后运行 20 轮摘要/提案生成，不引用或同义复活该内容。 |
| TEST-MEM-004 | FR-MEM-005, FR-DAT-001, NFR-PRIV-002 | Automated time-control | 可控时钟覆盖 29 天 23:59、30 天整、清理失败与重启重试；验证每次启动运行维护任务，并将虚拟时钟推进 24 小时确认周期任务至少运行一次。原文到期删除，合规摘要带覆盖区间且可查看/删除，DST 不延长保留期。 |
| TEST-AI-001 | FR-RAD-003, FR-CHAT-002, FR-MEM-002 | Automated evaluation | 使用固定安全/敏感场景集检查人格与边界：不冒充人类、医生或危机热线，不制造依赖，不把推断当事实，串场短而可跳过；每次模型变更重跑并人工抽检失败样本。 |

## 6. 日程、天气、设置与数据控制

| Test ID | Covers | Mode | Scenario and expected evidence |
|---|---|---|---|
| TEST-SCH-001 | FR-SCH-001, FR-SCH-004 | Automated time-control | 创建/编辑/启停/删除每周规则；重启保持。用 IANA 时区覆盖普通日期、DST 不存在和重复当地时间，验证下一触发显示与确定性一次触发规则。 |
| TEST-SCH-002 | FR-SCH-002, NFR-COST-001 | Manual-Windows + automated | 到点通知含开始/稍后/忽略；无人操作、忽略和过期均无声音及付费调用，只有点击开始才进入节目。使用计数 fake 运行虚拟 24 小时，分别覆盖后台扫描、空闲托盘和 Apple Music 纯监听，全部付费调用数为 0；再确认用户开始节目、文字请求、声音预览和显式连接测试分别只产生对应的一次授权调用。 |
| TEST-SCH-003 | FR-SCH-003 | Automated time-control | 同一次触发先延后 10 分钟再改 30 分钟，仅保留一个延后；到点只通知一次且原重复规则不改变。另验证 60 分钟选项。 |
| TEST-WEA-001 | FR-WEA-001, NFR-PRIV-004 | Automated E2E | 搜索同名城市并明确选择，保存城市/地区/坐标/时区；Geocoding 请求只含用户输入的搜索文字、语言和结果上限。搜索结果显示可访问的 `Location data by GeoNames via Open-Meteo` 文字归属链接。 |
| TEST-WEA-002 | FR-WEA-002, NFR-PRIV-004, NFR-OFF-002 | Automated contract | 捕获 Open-Meteo Forecast 请求只含所选坐标和所需天气参数；成功缓存含观测时间。请求失败时使用未过期缓存并显示来源/观测时间；无缓存或缓存过期时省略天气、AI 不猜测且≤2 秒显示独立降级状态。天气值附近显示可访问的 `Weather data by Open-Meteo.com` 文字归属链接。 |
| TEST-SET-001 | FR-SET-001, FR-SET-004 | Automated E2E | 查看/修改 provider、Base URL、模型和声音，分别测试连接；Key 只可替换/删除。`llmModelId` 实际变化时，API-008 以合并后的候选 origin/model 在 60 秒路径完成固定 pre-save capability probe：成功才原子保存整个 patch，auth/rate-limit/timeout/invalid-response 失败时所有字段与 revision 不变；相同 model 或不含 model 的 patch 保持 5 秒且 provider 调用为 0。各集成独立显示状态、最近成功时间及无 secret 重试。 |
| TEST-SET-002 | FR-SET-002 | Automated E2E | 切换默认来源、串场密度和 TTS；TTS 关闭后开场/串场仅显示文字且音乐正常，不产生 speech 请求或 TTS 音频。 |
| TEST-SET-003 | FR-SET-003 | Manual-Windows | 新安装自启动默认关闭；用户启用后下次登录启动但静音，关闭后任务/注册项移除。托盘启停行为与设置一致且结果可见。 |
| TEST-DAT-001 | FR-DAT-002 | Automated E2E | 生成对话、记忆、曲库、播放、天气和元数据后打开数据概览；分类型显示数量、保留期、存储类别和外发服务，不显示完整路径或密钥。 |
| TEST-DAT-002 | FR-DAT-003, NFR-PRIV-003 | Automated integration | 导出版本化制品并逐项核对清单：含要求的数据类和 schema 版本，不含 canary secret、音频、未授权文件或多余绝对路径。 |
| TEST-DAT-003 | FR-DAT-004, NFR-PRIV-003 | Automated integration | 分别预览并删除画像/记忆、对话/摘要、播放历史、缓存和曲库索引；逐项与隐私数据生命周期清单比对，验证作用域准确、依赖派生数据清理且原始音乐文件哈希不变。 |
| TEST-DAT-004 | FR-DAT-005, NFR-PRIV-003 | Manual-Windows + automated | 二次确认全部重置；验证数据库及 migration/recovery backup、缓存、生成音频、应用日志、Credential Manager 凭据、日程、自启动与设置均被不可恢复地移除。重启回到引导；扫描应用目录与凭据无用户 canary，音乐目录和主动保存的导出保持。 |

## 7. 横切质量验收

| Test ID | Covers | Mode | Scenario and expected evidence |
|---|---|---|---|
| TEST-REL-001 | NFR-REL-002 | Automated destructive-fixture | 在数据库写入、扫描提交、节目状态和 TTS 缓存阶段各强制结束 5 次；重启后数据库可读或进入恢复模式，全部保持静音。只操作一次性测试目录。 |
| TEST-SEC-001 | NFR-SEC-002, NFR-SEC-004 | Automated security | 静态检查 CSP/capabilities/前端请求；代理核对所有外连为 HTTPS 白名单。向请求、对话、画像和路径注入 canary，日志/错误栈敏感命中为 0。 |
| TEST-SEC-002 | NFR-SEC-003 | Automated Windows integration | 对授权目录边界、`..`、junction/symlink、权限撤回和导出目标执行攻击 fixture；越界均被拒绝且无外部文件改变。 |
| TEST-SEC-003 | NFR-SEC-005 | Automated + review | 对锁文件运行依赖漏洞和许可证审计；Critical/High 为 0，来源与许可证可追溯，锁文件变化均对应已审查任务。 |
| TEST-PRIV-001 | NFR-PRIV-001, NFR-PRIV-004 | Automated security + contract | 静态审计 manifest、Tauri capabilities 和依赖，确认没有屏幕、前台应用、麦克风、设备定位、通讯录权限或采集代码，且遥测/广告 SDK 数为 0；运行代理确认这些数据无出站。捕获 OpenAI 请求并逐字段验证只含生成所需上下文、`store: false`；注入 GSMTC canary 后确认 OpenAI、MusicBrainz、天气及其他全部出站请求命中数为 0，Apple Music 数据只在本机事件流中出现。 |
| TEST-A11Y-001 | NFR-A11Y-001, NFR-A11Y-003 | Automated + Narrator | 仅键盘完成引导、四主页面、节目、记忆、设置和重置；无焦点陷阱。axe 严重/高影响为 0，Narrator 能读出名称/角色/状态和适度 live region。 |
| TEST-A11Y-002 | NFR-A11Y-002, NFR-A11Y-004 | Automated visual | 校验每个 token 组合：正文/控件文字对比度≥4.5:1，大号文字≥3:1，非文字控件边界/状态≥3:1，且灰度下信息仍明确；100%–200% 缩放时目标至少 44×44 CSS px。启用减少动画后去除非必要过渡且无每秒超过 3 次闪烁。 |
| TEST-COMPAT-001 | NFR-COMPAT-001 | Manual VM | 在干净 Windows 10 22H2 与 Windows 11 标准用户 VM 安装 NSIS、完成引导、本地播放、升级与卸载；不要求管理员权限，卸载数据提示正确。 |
| TEST-COMPAT-002 | NFR-COMPAT-003 | Automated screenshots + Manual-Windows | 在 1280×720–3840×2160、100%–200% 缩放跑主状态截图矩阵，关键控件无永久遮挡；当前支持 Apple Music App 的未暴露能力安全隐藏。 |
| TEST-COST-001 | NFR-COST-002 | Automated contract | 注入超长画像、历史、候选和 TTS 文本；输入不超过 24,000 tokens、输出上限 4,000、候选最多 200、TTS 每段最多 500 Unicode 字符，UI 计数一致。 |
| TEST-MAINT-001 | NFR-MAINT-001, NFR-MAINT-003 | Automated CI | 运行 `scripts/verify-docs.ps1`、Rust/TypeScript schema 契约、lint 和静态规则；合法/边界示例通过、非法失败、ID 可追踪、契约变更有 Changelog。对生产运行路径执行 AST/静态检查：未处理 `unwrap`/`expect` 为 0、无 owner 的浮动异步任务为 0、未映射到稳定错误分类的错误出口为 0。 |
| TEST-MAINT-002 | NFR-MAINT-002 | Automated CI | 汇总 Rust/TypeScript 覆盖率：核心域行至少 80%、分支至少 70%；状态机、清理、候选、secret 边界和 Apple 恢复竞态场景清单 100%。 |
| TEST-OFF-001 | NFR-OFF-001 | Automated E2E | 阻断全部网络后完成曲库浏览、确定性队列、播放、反馈、设置、记忆管理、导出和重置；无持续重复错误或必需网络依赖。 |
| TEST-OFF-002 | NFR-OFF-003 | Automated network-control | 断网制造各 provider 失败后恢复；10 秒内只自动刷新非付费状态，不自动重放文本/TTS/其他可能计费或出声动作，显式重试后才调用。 |
| TEST-RESOURCE-001 | NFR-PERF-005 | Manual performance | 分别采集空闲托盘与本地播放 10 分钟 WPR：前者工作集≤250 MB、CPU 5 分钟均值<1%；后者工作集≤500 MB、CPU均值<10%。 |

## 8. Checkpoint 与发布证据索引

M1–M6 每次 checkpoint 在 `artifacts/test-evidence/milestones/<milestone>/manifest.json` 记录 candidate commit、选中的 `TEST-*` 或 milestone-specific smoke、结果、证据相对路径、known gaps、目标 milestone 与 checkpoint 结论。未选中测试记为 `Not Run`；实际失败记为 `Failed`，即使 checkpoint 为 `Passed with known gaps` 也不能改写为 `Passed`。hard gates 通过且文档同步后自动进入下一 milestone。

M7 beta candidate 在 `artifacts/test-evidence/<version>/manifest.json` 记录全部适用 `TEST-*` 的结果、证据相对路径与校验和。目录属于本地/CI 制品，不提交可能含个人信息、Apple Music 元数据或绝对路径的原始证据；仓库只保留去标识汇总。发布门槛豁免必须先改变需求基线并取得用户批准，不能把 `skipped` 或 `Not Run` 当作通过。
