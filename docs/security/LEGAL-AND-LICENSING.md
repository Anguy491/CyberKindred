# Legal and Licensing Baseline

| Field | Value |
|---|---|
| Status | Approved |
| Owner | Release Steward |
| Last Verified | 2026-09-08 |
| Source of Truth For | MVP 内测的本地音乐责任、服务条款边界、归因、字体与依赖许可门槛 |
| Related Documents | `docs/integrations/EXTERNAL-INTEGRATIONS.md`, `docs/architecture/DEPENDENCY-POLICY.md`, `docs/operations/BUILD-RELEASE.md`, `docs/security/PRIVACY-DATA-LIFECYCLE.md` |

> 本文是工程与发布检查清单，不是法律意见。公开分发、收费、广告、企业部署、跨境数据处理或权利人投诉出现时，应由具备适用法域资质的法律顾问复核。

## 1. Release posture

Documentation Baseline v1 只批准本人和少量受邀用户的非商业 Windows 内测，不批准公开商店上架、付费、广告、音乐转授权、云端转播或品牌联名。任何商业化或公开发布都先完成条款复核、第三方许可清单、隐私通知、代码签名和归因验收；不能把“内测可用”当成“公开发行已获授权”。

## 2. Local music files

- CyberKindred 只索引并播放用户通过 native picker 选择、Windows 当前账户可读取的本地文件。应用不提供音乐下载、上传、P2P、远程串流、转码分发、分享链接或音频导出。
- 用户负责拥有或获准使用其文件。产品文案不得暗示 CyberKindred 为音乐副本授权，也不得鼓励绕过地域、订阅、版权或技术保护措施。
- 扫描器把文件视为只读：不改标签、不重命名、不移动、不删除源音乐。移除曲库与全部重置只删除索引/cache。
- 不实现 DRM 破解、Apple Music cache 提取、浏览器媒体抓取、录制外部服务音频或从受保护流生成本地副本。发现受保护/不支持格式时只报告“不支持”。
- 内测 fixture 必须是项目自制、明确许可或公共领域的短音频；fixture 仓库旁保存来源、作者、许可和修改说明，不提交用户音乐。

## 3. Apple Music and Windows media control

MVP 通过 Windows GSMTC 操作 Apple Music Windows App 已公开给系统的 media session。它不调用 Apple Music API/MusicKit，不接收 Apple ID、developer token 或 user token，不搜索 catalog，不修改 library/playlist，不操控 Web DOM，也不规避 Apple 客户端或订阅限制。

GSMTC 只证明 Windows 暴露了某项控制能力，不授予音乐内容的复制、重分发或品牌权。产品使用中性描述“Apple Music via Windows media controls”，不使用 Apple logo、不声称 Apple 赞助/认证，也不把外部歌曲/封面打包进安装包。用户必须自行安装 Apple Music、登录并维持有效服务资格。

若未来进入 MusicKit，必须另立 ADR、FR/NFR、隐私与 threat-model 变更，并完成 Apple Developer Program、Media ID/private key/developer token、用户授权、品牌与适用协议审查。Apple 官方说明 MusicKit catalog/个性化能力需要相应配置与用户许可。[MusicKit](https://developer.apple.com/musickit/) 当前 MVP 不以 MusicKit Web 或 DOM automation 填补 GSMTC capability 缺口。

## 4. OpenAI BYOK

- 用户使用自己的 API Key，并直接受其 OpenAI 账户、API 条款、usage policy、计费和数据控制约束。CyberKindred 不代售 token、不共享 key、不承诺固定模型长期可用。
- 设置页在首次调用前披露发送的数据类别、可能费用、AI 生成文字/语音身份与关闭方式。应用提供预计/实际 usage facts，但不把估算当作账单承诺。
- Responses 固定 `store:false`；这不构成对 OpenAI 所有日志、abuse monitoring 或法定保留的保证。[OpenAI data controls](https://platform.openai.com/docs/models/default-usage-policies-by-endpoint)
- 自定义兼容 origin 是用户明确选择的独立接收方；UI 必须显示 hostname、数据类别和 credential 隔离。不得把“OpenAI-compatible”表述成 OpenAI 官方运营或背书。
- 模型文字与语音可能不准确；产品不把其呈现为医疗、心理治疗、紧急服务、法律或财务建议。AI behavior 与 UI 始终表明它是 AI。

## 5. MusicBrainz metadata

MusicBrainz core database 采用 CC0；supplementary data 采用 CC BY-NC-SA 3.0，商业使用不能笼统假设所有 API 字段都是 CC0。[MusicBrainz data license](https://musicbrainz.org/doc/About/Data_License)

内测规则：

- 只读查询，提供合规 `User-Agent`，应用 About/Credits 中显示 “Metadata by MusicBrainz” 与链接。
- 缓存记录 provider、MBID、取得时间和置信度；不把 MusicBrainz 商标用于暗示背书。
- 不复制/发布 MusicBrainz documentation；只在本项目中用自己的语言描述接口行为并链接原文。
- 商业/公开发布前，由 release steward 将实际使用的每个字段映射到 core/supplementary 类别；若包含 NC-SA 数据，则取得合适商业许可、满足其条件或移除该字段。该核验是 release blocking gate。

## 6. Cover Art Archive

Cover Art Archive 的公开可访问性不等于每张封面可自由再分发。其官方说明封面来自与 Internet Archive 的合作并要求尊重艺术家/厂牌权利、风险由使用方承担。[Cover Art Archive](https://musicbrainz.org/doc/Cover_Art_Archive)

- MVP 只为用户本机即时显示并短期缓存，不把图片嵌入安装包、数据导出、宣传素材或公共 CDN。
- UI 保存 source URL/MBID，并提供 source/权利投诉指引；删除 cache 不影响 metadata。
- 公开/商业发布前逐项评估展示、缓存与归因方式；无法建立可接受权利依据时，默认禁用远程封面，仅使用用户文件内嵌封面或自制占位图。

## 7. Open-Meteo

截至基线日期，Open-Meteo free/open-access API 条款限定非商业使用并设调用限额；商业产品需要相应订阅或自托管方案。[Terms](https://open-meteo.com/en/terms) API 数据以 CC BY 4.0 提供，显示位置旁必须给出来源链接并注明变更。[Licence](https://open-meteo.com/en/license)

MVP 在城市搜索结果显示 `Location data by GeoNames via Open-Meteo`，在每个天气展示点显示可点击的 `Weather data by Open-Meteo.com`；About/Credits 包含 GeoNames/Open-Meteo 来源与 CC BY 4.0 链接。Open-Meteo 官方 geocoding 文档确认 location data 基于 GeoNames。[Geocoding API](https://open-meteo.com/en/docs/geocoding-api) 任何收费、广告或商业推广构建在发布前切换到允许商业使用的 endpoint/方案并保存采购与条款证据，否则关闭城市搜索和天气。

## 8. Design skill and fonts

| Asset | Baseline license | Distribution rule |
|---|---|---|
| Nothing Design Skill | MIT | 若复制其代码/文档的实质部分，安装包 source notices 保留 copyright 与 MIT permission text。[Upstream license](https://github.com/dominikmartn/nothing-design-skill/blob/main/LICENSE) |
| Space Grotesk | SIL Open Font License 1.1 | 可与软件捆绑；安装包附原 copyright 与完整 OFL，不单独销售字体，不用 Reserved Font Name 命名修改版。[Pinned OFL source](https://github.com/google/fonts/blob/f6b2b7e8545e086ad3f821af21895d732b6485cf/ofl/spacegrotesk/OFL.txt) |
| Space Mono | SIL Open Font License 1.1 | 同上；保留字体二进制来源 commit/hash 与许可文本。[Pinned OFL source](https://github.com/google/fonts/blob/f6b2b7e8545e086ad3f821af21895d732b6485cf/ofl/spacemono/OFL.txt) |
| Doto | SIL Open Font License 1.1 | 同上；保留字体二进制来源 commit/hash 与许可文本。[Pinned OFL source](https://github.com/google/fonts/blob/f6b2b7e8545e086ad3f821af21895d732b6485cf/ofl/doto/OFL.txt) |

字体必须随 app 本地打包，不在运行时从 Google Fonts/CDN 拉取。发布 manifest 记录精确文件名、上游 URL、commit/tag、SHA-256、copyright 和 license；只从附带许可的上游 release/source 取得二进制。M2 的固定清单位于 `src/assets/fonts/MANIFEST.json`。

## 9. Software dependencies

### 9.1 Admission rules

- 允许进入自动审查通道：MIT、Apache-2.0、BSD-2-Clause、BSD-3-Clause、ISC、Zlib、Unicode-DFS、CC0/Public Domain dedication；字体允许 OFL-1.1。
- MPL-2.0、LGPL、CDDL 或带动态链接/文件级 copyleft、专利、NOTICE 等特殊条件的依赖必须由 release steward 记录组合方式和履约步骤后才能进入 lockfile。
- GPL、AGPL、SSPL、BUSL、Commons Clause、source-available、non-commercial、no-derivatives、未知/无许可的生产依赖默认禁止；用户明确批准且取得发布法律意见后才可改变。
- 开发工具许可也要扫描，但不随制品分发时与 runtime 清单分开。禁止从示例、博客或模型输出复制无来源代码。

### 9.2 Required artifacts

每个 release candidate 生成并保存：

1. 锁定的 Rust/JavaScript dependency tree 与 source URL/version/checksum。
2. CycloneDX 或 SPDX SBOM。
3. `THIRD-PARTY-NOTICES`，包含需要的 copyright、license、NOTICE 和 attribution。
4. 安装包内 About/Credits 页与离线 license 目录。
5. license/deny/advisory 扫描报告；unknown、denied、checksum drift 或缺失 notice 使构建失败。

更新依赖时重新运行审查；transitive dependency 与 binary redistributable（WebView2 bootstrapper、NSIS plugin、native codec、DLL）不能因“不直接 import”而跳过。

### 9.3 M7 local audit record

The M7 generator produced a deterministic CycloneDX inventory with 449 components and 449 dependency records, 253 unique notice texts and four locally bundled font records. `cargo audit` reported zero vulnerabilities, `cargo deny` passed advisories/bans/licenses/sources, and the production pnpm audit reported zero vulnerabilities at the High threshold. These are engineering compliance checks, not legal approval. The unsigned validation artifact remains non-distributable while the M7 checkpoint is blocked, and `RISK-012` stays open until the release steward completes the intended beta-use review.

## 10. Product names, privacy and claims

- “Apple Music”“Windows”“OpenAI”“MusicBrainz”“Open-Meteo”仅作兼容性/来源事实说明，遵守各方商标指引，不作为 app 名或暗示背书。
- CyberKindred 不承诺音乐、metadata、天气或 AI 输出准确/持续可用；降级行为在 UI 可见。
- 隐私声明不得使用“完全本地”“不会外发”“匿名”或“零保留”等与实际 provider 请求、IP/坐标披露或第三方日志不符的绝对表述。
- 未成年人、医疗/心理危机或受监管用途不在 MVP 定位；敏感场景按 AI behavior 提供克制的求助引导。

## 11. Release compliance gate

内测制品只有在以下证据齐全时可交付：

- 用户选择的所有外部 provider 条款与 privacy 链接可在 Settings 打开；provider 可独立关闭。
- About/Credits 显示 MusicBrainz/Open-Meteo 归因、字体/设计许可和第三方 notices。
- SBOM、license scan、notice scan 和 fixture provenance 均通过；安装包不含用户文件、key、数据库、日志或测试 credential。
- Apple Music 文案仅描述 GSMTC 系统控制，未宣称 MusicKit/catalog 能力，未包含 Apple logo/音频/封面素材。
- Open-Meteo 构建用途仍为非商业；商业标志为 true 的构建必须使用获许可方案或编译关闭天气。
- 数据 export、诊断包与 crash handling 经 canary 扫描，确认不含 secret、绝对路径或未授权正文。

条款变更、权利人投诉或新地区发布触发重新评估；在结论形成前，相关远程集成通过本地 kill switch 关闭，不以 scraping 或镜像服务绕过。
