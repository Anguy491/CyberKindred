# CyberKindred UX Specification

| Metadata | Value |
|---|---|
| Status | Approved |
| Owner | Product Design |
| Last Verified | 2026-09-02 |
| Source of Truth For | 首发信息架构、页面行为、文案状态、键盘路径与 Nothing-inspired OLED 视觉规则 |
| Related Documents | `docs/product/FRS.md`; `docs/product/NFRS.md`; `docs/product/AI-BEHAVIOR.md`; `docs/architecture/RUNTIME-STATE-MACHINES.md`; `docs/contracts/API-CONTRACT.md` |

## 1. 交付边界

本文档是纯文字交互规格，不包含线框图或高保真原型。首发仅验收 OLED 深色主题；保留未来浅色 token 不代表首发必须提供主题切换。所有页面默认简体中文，领域状态或仪表标签可用英文大写，错误必须给出中文可行动说明。

## 2. 字体加载声明

`UX-SYS-001`：安装包必须本地包含并加载以下字体文件，不使用运行时 Google Fonts、远程 CSS 或系统是否已安装字体作为前提：

| Role | Family | Required weights | Fallback | Usage |
|---|---|---|---|---|
| Display | Doto | Variable 400–700 | Space Mono, monospace | 36 px 以上的当前状态、时间或一屏唯一 hero；不用于正文 |
| Body / UI | Space Grotesk | 300, 400, 500, 700 | system-ui, sans-serif | 标题、正文、按钮之外的自然语言 |
| Data / Labels | Space Mono | 400, 700 | Consolas, monospace | 数据、时间、路径摘要、全大写仪表标签与按钮 |

单屏最多使用两个主要字体家族；Doto 只作为唯一 hero 的例外。同屏最多三个字号、两个字重。字体失败时必须使用 fallback 保持可操作，且设置页显示 `[FONT FALLBACK]` 诊断状态。

## 3. Design tokens

### 3.1 色彩

`UX-SYS-002`：OLED 深色 token 固定如下；状态色只施加于值、边框或小型信号，不铺满大面积背景。

| Token | Value | Role |
|---|---|---|
| `--black` | `#000000` | 应用主背景 |
| `--surface` | `#111111` | 次级平面、列表分组 |
| `--surface-raised` | `#1A1A1A` | 下拉/模态等高一层平面 |
| `--border` | `#222222` | 装饰性细分隔 |
| `--border-visible` | `#333333` | 可感知控件边界 |
| `--text-disabled` | `#666666` | 禁用、提示和装饰信息 |
| `--text-secondary` | `#999999` | 标签、元数据 |
| `--text-primary` | `#E8E8E8` | 正文与普通内容 |
| `--text-display` | `#FFFFFF` | 一屏唯一最高级信息 |
| `--accent` / `--error` | `#D71921` | 当前活动信号、破坏性动作或错误；无事件时不使用 |
| `--accent-subtle` | `rgba(215,25,33,0.15)` | 小范围信号底色，不作页面装饰 |
| `--success` | `#4A9E5C` | 已连接、已完成 |
| `--warning` | `#D4A843` | 降级、等待、注意 |
| `--interactive` | `#5B9BF6` | 仅文字链接和选择值，不用于主按钮 |

同屏最多四级灰度文本。错误、成功与警告必须同时给出图形或文字标签，不能只凭颜色区分。

### 3.2 字号与间距

`UX-SYS-003`：字号 token 为 `72/72 px display-xl`、`48/50 px display-lg`、`36/40 px display-md`、`24/29 px heading`、`18/23 px subheading`、`16/24 px body`、`14/21 px body-sm`、`12/17 px caption`、`11/13 px label`。Label 使用 Space Mono、ALL CAPS、`0.08em` 字距。中文全大写不适用时使用短英文状态或中文原样，不人为增加字符间空格。

间距 token 为 2、4、8、16、24、32、48、64、96 px。4–8 px 表示同一小组，16 px 表示组内不同项，32–48 px 表示新分组，64–96 px 表示新上下文。优先用间距，其次分隔线，再次轮廓；只有必要时使用表面卡片。

### 3.3 三层层级

`UX-SYS-004`：每个页面严格只有三层重要性：

1. **Primary**：一屏唯一第一注意点，如正在播放曲名、扫描百分比、用户画像标题或设置问题；Doto/Space Grotesk + `--text-display`，保留 48–96 px 呼吸区。
2. **Secondary**：支持当前任务的正文、描述、列表和主操作；Space Grotesk + `--text-primary`，8–16 px 紧密分组。
3. **Tertiary**：导航、时间、来源、诊断和标签；Space Mono + `--text-secondary/disabled`，推向边缘。

任何页面出现两个竞争 hero 都必须降级其中一个。最重要元素不放入有背景的卡片，让它直接位于 OLED 黑色画布上。

### 3.4 组件、图标与动效

`UX-SYS-005`：主按钮为白底黑字 pill，次按钮透明底+`--border-visible` pill，破坏性按钮透明底+红色边框；按钮统一 Space Mono 13 px 全大写、最小 44 px 高。技术型紧凑控件可用 4–8 px 圆角；卡片 12–16 px，任何卡片圆角不超过 16 px。

输入框优先使用下边框，焦点变为 `--text-primary`，错误变红并在下方显示文字。导航与标签使用 Space Mono；图标仅用 1.5 px 单色无填充线条，24 px 画布，不能用 emoji 充当控件。

微交互 150–250 ms，页面状态 300–400 ms，使用 `cubic-bezier(0.25, 0.1, 0.25, 1)`；优先透明度变化，不缩放。启用系统减少动画时直接切换状态。

`UX-SYS-006`：首发明确禁止渐变、阴影、模糊玻璃、骨架屏、应用内 Toast、弹跳/弹簧动画、视差、滚动劫持、斑马纹、多色/填充图标、装饰性大红色背景、可爱吉祥物和多段空状态说明。反馈使用触发点附近的内联 `[SAVED]`、`[ERROR: …]`、`[OFFLINE]` 或 `[LOADING…]`。

## 4. Window and navigation

`UX-NAV-001`：桌面主窗口最小内容尺寸 1024×640；允许缩小到 1280×720 屏幕内完全可操作。窗口顶部/左上显示产品名和连接总状态，主体采用非对称布局，保留明显空白。

`UX-NAV-002`：主导航固定为 `RADIO | LIBRARY | YOU | SETTINGS`。Space Mono 11–12 px；活动项为白色并带一个 2 px 红点或下划线，非活动项为 `--text-disabled`。页面顺序和可访问名称分别为“电台、曲库、了解、设置”。切换页面不停止节目，不清空未发送输入；离开存在未保存表单时在页面内询问保存/放弃。

`UX-NAV-003`：全局键盘规则：

- `Ctrl+1/2/3/4` 依次切换四个主页面。
- `Space` 仅在焦点不位于输入/按钮时切换播放/暂停；Apple Music 不支持相应能力时不执行。
- `Ctrl+L` 聚焦 `RADIO` 文字输入，`Ctrl+K` 聚焦 `LIBRARY` 搜索。
- `Ctrl+,` 打开 `SETTINGS`；`Esc` 关闭当前模态/菜单或取消未提交操作，不结束节目。
- `Tab/Shift+Tab` 按视觉阅读顺序移动；焦点环为 2 px `--text-display` 外轮廓，不能只靠颜色。
- 破坏性操作不设单键快捷方式。任何全局快捷键都不得在文本编辑时覆盖标准输入行为。

## 5. First-run onboarding

`UX-ONB-001`：引导为单窗口分步流程。顶部 tertiary 显示 `SETUP 01/07` 和可返回入口；中部 primary 只呈现当前问题；底部 secondary 提供说明、表单和“继续”。不显示完成百分比条，不允许跳过隐私确认。

`UX-ONB-002`：音乐来源步骤用两个可多选的技术型选项：`LOCAL FILES` 与 `APPLE MUSIC / WINDOWS APP`。Apple Music 选项正文必须写明“读取并控制 Windows 系统媒体会话；不提供精确点歌，不控制网页”。本地目录选择后显示授权目录的显示名和 `[待扫描]`；文件计数只在 API-013 首次扫描完成后由 `LIBRARY` 展示。引导不为取得计数而扫描，也不在常规界面暴露完整路径。

`UX-ONB-003`：OpenAI Key 输入默认为 password；提供“显示至按下结束”的临时查看动作、`验证` 和 `稍后仅使用本地播放`。验证状态固定为 `[CHECKING…]`、`[VALID]`、`[INVALID KEY]`、`[RATE LIMITED]`、`[OFFLINE]`；错误下方给出下一步，成功后输入值立即回到掩码。

`UX-ONB-004`：声音预览每次由按钮触发；`previewAvailable: false` 时控件禁用并提供文字降级。可预览声音在播放中显示 `[PLAYING]`，再次按下通过 API-038 的 `voice_preview` cancel slice 停止；该可播放/取消路径由 `TASK-017` 随 TTS actor 交付，M2 不伪造播放能力。画像自由文本标为可选。城市选择显示同名城市的国家/地区和时区。日程明确写明“只通知，确认后才播放”。

`UX-ONB-005`：最终确认页只总结已选择内容与外发/保留政策；用户需勾选“我知道节目不会未经确认自动出声”和“我知道原始对话默认保留 30 天”。完成后进入静音的空闲 `RADIO`，焦点位于“开始节目”。

每次“继续”先用 API-003 原子保存当前 step；返回后重新保存已完成 step 只更新该步输入，不清除后续已完成前缀。应用启动先读 API-002，并定位固定顺序中首个未完成 step；画像、来源 mode 与 privacy confirmation 从权威响应恢复，目录、provider、声音、城市和日程的详细值由各自 read API 补齐。WebView session/local storage 不作为引导完成事实。

键盘验收路径为：首次启动→来源多选→目录选择→Key 验证/降级→声音预览→画像→城市/日程→隐私确认→完成，全程可用 `Tab`、`Shift+Tab`、`Space/Enter` 和 `Esc` 完成。

## 6. RADIO

`UX-RAD-001`：空闲页 primary 是“今天想听什么状态？”或最近来源的开始入口；secondary 是来源分段控件、`开始节目` 和文字输入；tertiary 显示时间、天气可用性、下次日程和连接状态。不得用聊天消息列表占据第一视觉焦点。

`UX-RAD-002`：播放中 primary 是当前真实曲名；艺术家、专辑和主播文字为 secondary；来源、进度时间、能力与节目段编号为 tertiary。曲名过长最多两行，保留完整内容的可访问名称。封面可显示但不承担状态表达，缺失时使用低对比点阵占位而非网络随机图片。

`UX-RAD-003`：播放控件顺序固定为上一首、播放/暂停、下一首、进度；下方是喜欢、跳过、少说一点。来源不支持的操作保留布局但呈 disabled，并在聚焦/悬停时显示内联原因，例如 `[UNAVAILABLE: APPLE MUSIC SESSION]`，不能静默失败。

`UX-RAD-004`：主播文字固定显示在 Now Playing 下方，标为 `CYBERKINDRED / 08:42`；TTS 播放时显示 `[SPEAKING]` 和“停止语音”，停止只终止 TTS，不停止音乐。AI 安全提示占据该区域并暂停普通主播交互，直到用户明确关闭/确认。

`UX-RAD-005`：文字输入为一至六行自动增长；`Enter` 发送，`Shift+Enter` 换行，处理中变为 `[THINKING…]` 并提供“取消”。回复与用户消息按时间显示，但最多展开当前节目最近 20 条，较早内容通过“查看本次记录”进入独立可滚动区域。

`UX-RAD-006`：结束节目使用 secondary 按钮，确认内容说明会停止哪些音频、不会删除历史。Apple Music 模式必须持续显示 `COMPANION MODE`、“队列由 Apple Music 控制”和“曲目反应在本机生成”；不能用本地节目计划视觉暗示精确点歌。只有不含 GSMTC/Apple Music 数据的通用段可标记 `[SPEAKING]`。

`UX-RAD-007`：`RADIO` tertiary 区显示本次会话的 LLM 输入/输出 token 与 TTS 字符累计值，标为 `SESSION USAGE`；数值来自本地计数，只用于成本可见性，不显示请求正文、价格承诺或跨会话历史。

## 7. LIBRARY

`UX-LIB-001`：未配置目录时 primary 为“还没有本地曲库”，secondary 只有一句“选择包含你有权使用的音乐文件的目录”和“选择目录”；不得使用插画或多段说明。

`UX-LIB-002`：扫描时 primary 是 Doto 百分比或已处理数；secondary 是分段进度条、发现/成功/错误数和“取消”；tertiary 是当前文件显示名与估计状态。使用 `[SCANNING…]`，禁止骨架屏。取消后显示 `[SCAN INCOMPLETE]` 与“继续扫描”。

`UX-LIB-003`：曲库列表顶部为搜索与状态过滤；行按“标题/艺术家—专辑—时长—来源/匹配状态”展示，不使用斑马纹。活动行仅用 `--surface-raised` 与左侧 2 px 红色信号。完整本地路径只在用户主动展开诊断信息时显示。

`UX-LIB-004`：曲目详情区分 `LOCAL TAGS` 与 `MUSICBRAINZ MATCH`，显示置信度和来源时间。低置信匹配为黄色值 `[REVIEW]`，不会假装覆盖成功。无网络时保留本地曲库并显示 `[OFFLINE: METADATA PAUSED]`。

`UX-LIB-005`：键盘路径：`Ctrl+K` 搜索，`ArrowUp/Down` 移动列表，`Enter` 打开详情，`Esc` 返回列表；扫描、筛选与目录管理均有标准 Tab 路径。删除目录索引的文字必须明确“不会删除音乐文件”。

## 8. YOU

`UX-YOU-001`：primary 是“CyberKindred 目前如何了解你”；secondary 依次为画像、偏好趋势、待确认记忆、已批准记忆、会话摘要；tertiary 显示更新时间和来源。各类必须通过标题和状态文字区分，不能只靠颜色。

`UX-YOU-002`：Memory Proposal 行提供“批准”“编辑并批准”“拒绝”。批准为普通主操作，拒绝为 ghost，不使用破坏性红色；操作后原位置显示 `[APPROVED]` 或 `[REJECTED]`，不弹 Toast。

`UX-YOU-003`：Approved Memory 行显示内容、来源、创建/最近使用时间、启用状态，提供编辑、停用和删除。删除采用 480 px 内模态，说明后果并要求二次确认；焦点锁定模态、`Esc` 取消、关闭后回到触发按钮。

`UX-YOU-004`：空状态分别为“暂无待确认记忆”“还没有长期摘要”，每项最多一句解释。删除后的区域显示 `[DELETED]` 直到焦点离开，后续不再出现该文本。

## 9. SETTINGS

`UX-SET-001`：页面按 `AI & VOICE`、`PLAYBACK`、`APPLE MUSIC`、`CONTEXT`、`SCHEDULE`、`APP`、`PRIVACY & DATA` 分组。primary 为当前选中组标题，左侧/顶部 tertiary 目录只显示组名；不得把每个设置装进独立卡片。

`UX-SET-002`：Provider 状态按项显示 `[CONNECTED]`、`[DEGRADED]`、`[OFFLINE]`、`[NOT CONFIGURED]` 与最近成功时间。Key 永远为掩码，只有“替换”和“删除”。Base URL 与模型 ID 作为高级设置折叠区，保存前明确列出变更。

`UX-SET-003`：Apple Music 组先显示当前 Windows App 会话和能力清单，再显示连接指引。无 App/无会话/会话受限分别使用不同文字；绝不提供“登录 MusicKit”或“控制网页”入口。

`UX-SET-004`：Schedule Rule 以星期、本地时间、时区、下一触发和启用开关展示。创建/编辑后在表单附近显示 `[SAVED]`。任何规则旁固定显示“到点只通知”。自启动开关旁固定显示“启动后保持静音”。

`UX-SET-005`：`PRIVACY & DATA` 首屏显示数据类别、数量、保留期与外发服务；导出是 secondary 操作，分类删除与全部重置是分开的破坏性操作。全部重置模态必须列出数据库及迁移/恢复备份、缓存、生成音频、日志、密钥、日程和设置，明确操作不可恢复，将“不删除你的音乐文件或主动保存的导出”单独成行，并要求输入 `DELETE CYBERKINDRED DATA`；该确认短语以 `API-CONTRACT.md` 为唯一事实源。

`UX-SET-006`：`CONTEXT` 的城市搜索结果附近持续显示可访问文字链接 `Location data by GeoNames via Open-Meteo`；`RADIO` 的天气值或详情附近显示 `Weather data by Open-Meteo.com`。离线缓存仍显示来源和观测时间，链接遵守 `--interactive` 文字链接样式，不使用第三方徽标替代文字。

## 10. State language and failure behavior

| ID | State | Required presentation |
|---|---|---|
| UX-STA-001 | Loading | 使用 `[LOADING…]`、机械分段 spinner 或带数字的分段进度；已知进度必须显示数字；不使用骨架屏。 |
| UX-STA-002 | Empty | 96 px 以上留白、`--text-secondary` 短标题、一句 `--text-disabled` 说明和至多一个操作；无吉祥物、哭脸或多段营销文案。 |
| UX-STA-003 | Error | 在触发点附近显示 `[ERROR: CATEGORY]`、一句原因和一个可行动作；表单错误在字段下；不使用 Toast、红色满屏或只给错误码。 |
| UX-STA-004 | Offline | 全局 tertiary 显示 `[OFFLINE]`，各功能只标记自身丢失能力；本地播放和数据管理保持普通可用外观，不将整页禁用。 |
| UX-STA-005 | Degraded | 用黄色状态值说明“什么不可用/仍可做什么”，例如 `[TTS UNAVAILABLE — TEXT CONTINUES]`；不得伪装成功。 |
| UX-STA-006 | Capability missing | 控件 disabled、可聚焦读取原因且不执行；能力恢复后原位启用，不移动布局。 |
| UX-STA-007 | Success | 操作附近显示 `[SAVED]`、`[CONNECTED]` 或 `[EXPORTED]` 3–5 秒，焦点与屏幕阅读器可感知；关键持久状态不自动消失。 |
| UX-STA-008 | Destructive confirmation | 说明对象、不可逆结果与不受影响内容；默认焦点位于取消，执行按钮不得仅靠红色表达风险。 |

## 11. Windows notifications and background

`UX-WIN-001`：节目日程通知标题为“CyberKindred / 你的节目时间到了”，正文可包含日程名和当前可用来源，动作固定为“开始节目”“10/30/60 分钟后”“忽略”。通知正文不包含原始对话、记忆详情或敏感画像。

`UX-WIN-002`：点击通知动作“开始节目”本身即为明确确认，应用将主窗口置前、显示所用来源并开始；点击通知本体只打开 `RADIO`，不等价于确认。忽略、关闭或通知过期都保持静音。

`UX-WIN-003`：托盘菜单只提供“打开 CyberKindred”“播放/暂停（能力允许时）”“结束节目”“退出”。启动、自启动、休眠恢复和更新设置均不得产生测试音或主播语音。

## 12. Responsive, accessibility and copy

`UX-A11Y-001`：1280×720 下主操作无水平滚动；更窄窗口可让内容区垂直滚动，但导航和当前播放的主要控制保持可达。200% 缩放时不截断按钮文本，表格转为分层行，不压缩为不可读列。

`UX-A11Y-002`：Now Playing 更新使用 polite live region；错误与安全提示使用 assertive；每次曲目时间更新不宣告。封面 alt 为空或为“专辑封面：{album}”，不能重复已紧邻的标题/艺术家。

`UX-A11Y-003`：文案使用直接动词：“开始节目”“停止语音”“删除记忆”。不使用“魔法”“读懂一切”“永远陪你”或把技术限制归因于用户。未知状态写“尚未获得”，不用“无”。

## 13. Screen acceptance checklist

M1–M6 prototype checkpoint 只要求为当前 milestone 的主路径与实际演示的错误/降级状态提供代表性截图，并做主路径键盘 smoke；缺失的状态矩阵与 Narrator 证据登记到 M7。M7 beta 验收时，每个页面必须提供默认、加载、空、错误、离线、降级与能力缺失中适用的状态截图，以及仅键盘和 Windows Narrator 的操作证据。任何阶段，审查者在模糊视图下都应能识别唯一 primary；若两个元素竞争、页面依赖卡片堆叠或红色成为装饰，视为不符合三层层级。
