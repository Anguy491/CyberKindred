# CyberKindred Requirements Traceability Matrix

| Metadata | Value |
|---|---|
| Status | Approved |
| Owner | Engineering Lead & Quality Engineering |
| Last Verified | 2026-09-02 |
| Source of Truth For | 每个 FR/NFR 到 UX、架构、契约、测试、任务和里程碑的逐项追踪关系 |
| Related Documents | `docs/product/FRS.md`; `docs/product/NFRS.md`; `docs/product/UX-SPEC.md`; `docs/architecture/ARCHITECTURE.md`; `docs/contracts/API-CONTRACT.md`; `docs/testing/ACCEPTANCE-TESTS.md`; `docs/planning/BACKLOG.md` |

## 1. 使用规则

本矩阵只建立引用，不重新定义需求或契约。表中 `Contract` 的 schema 文件名均位于 `docs/contracts/schemas/`；provider interface 指 `PROVIDER-CONTRACTS.md`。实现 PR/提交必须更新受影响行。任何空单元格、引用不存在、需求无测试/任务或任务无需求都阻断开发。一个测试可覆盖多项需求，但测试失败时必须能回溯到具体需求断言。

## 2. Functional requirements

### 2.1 Onboarding

| Requirement | UX | Architecture | Contract | Test | Task | Milestone |
|---|---|---|---|---|---|---|
| FR-ONB-001 | UX-ONB-001 | ARCH-002, ARCH-006 | API-002, API-003, API-033, API-048, API-049 | TEST-ONB-001 | TASK-010, TASK-032 | M2/M7 |
| FR-ONB-002 | UX-ONB-002 | ARCH-002, ARCH-003, ARCH-004 | API-003, API-010, API-011, API-016 | TEST-ONB-001 | TASK-010 | M2 |
| FR-ONB-003 | UX-ONB-003 | ARCH-003, ARCH-007 | API-004, API-006 | TEST-ONB-002 | TASK-003, TASK-008 | M1/M2 |
| FR-ONB-004 | UX-ONB-003, UX-STA-003 | ARCH-008, ARCH-015 | API-006, [`provider-error.schema.json`](../contracts/schemas/provider-error.schema.json) | TEST-ONB-002 | TASK-008 | M2 |
| FR-ONB-005 | UX-ONB-004 | ARCH-008, ARCH-012 | API-009, API-043, EVT-008, EVT-009 | TEST-ONB-003 | TASK-008, TASK-017 | M2/M3 |
| FR-ONB-006 | UX-ONB-004 | ARCH-006, ARCH-011 | API-003 | TEST-ONB-001 | TASK-010 | M2 |
| FR-ONB-007 | UX-ONB-005 | ARCH-014, ARCH-016 | API-002, API-003, API-033, API-048, API-049 | TEST-ONB-004 | TASK-010 | M2 |

### 2.2 Radio and program

| Requirement | UX | Architecture | Contract | Test | Task | Milestone |
|---|---|---|---|---|---|---|
| FR-RAD-001 | UX-RAD-001, UX-WIN-002 | ARCH-014 | API-024, EVT-002 | TEST-ONB-004, TEST-RAD-001 | TASK-018, TASK-019, TASK-032 | M3/M7 |
| FR-RAD-002 | UX-STA-005 | ARCH-009, ARCH-010 | API-024, [`program-plan.schema.json`](../contracts/schemas/program-plan.schema.json) | TEST-LIB-004 | TASK-015, TASK-016 | M3 |
| FR-RAD-003 | UX-RAD-002, UX-RAD-004 | ARCH-013 | API-024, EVT-002, EVT-003, [`program-plan.schema.json`](../contracts/schemas/program-plan.schema.json) | TEST-RAD-001, TEST-AI-001 | TASK-018, TASK-019 | M3 |
| FR-RAD-004 | UX-RAD-002, UX-RAD-003 | ARCH-004, ARCH-013, ARCH-016 | API-016–API-023, EVT-001, [`playback-state.schema.json`](../contracts/schemas/playback-state.schema.json) | TEST-RAD-002 | TASK-006, TASK-009, TASK-014, TASK-019 | M2/M3 |
| FR-RAD-005 | UX-RAD-003, UX-STA-007 | ARCH-006, ARCH-010 | API-027 | TEST-RAD-002 | TASK-020 | M4 |
| FR-RAD-006 | UX-RAD-006 | ARCH-004, ARCH-012, ARCH-013 | API-025, [`playback-state.schema.json`](../contracts/schemas/playback-state.schema.json) | TEST-RAD-003 | TASK-014, TASK-018, TASK-026 | M3/M6 |
| FR-RAD-007 | UX-STA-003, UX-STA-005 | ARCH-008, ARCH-015 | API-024, EVT-009, [`provider-error.schema.json`](../contracts/schemas/provider-error.schema.json) | TEST-RAD-004 | TASK-017, TASK-018, TASK-019, TASK-029 | M3/M7 |

### 2.3 Local library

| Requirement | UX | Architecture | Contract | Test | Task | Milestone |
|---|---|---|---|---|---|---|
| FR-LIB-001 | UX-LIB-001, UX-LIB-002 | ARCH-002, ARCH-005 | API-011, API-013, EVT-005 | TEST-LIB-001 | TASK-002, TASK-011, TASK-032 | M1/M3/M7 |
| FR-LIB-002 | UX-LIB-002 | ARCH-006, ARCH-012 | API-013, API-014, EVT-005 | TEST-LIB-002 | TASK-011 | M3 |
| FR-LIB-003 | UX-LIB-003 | ARCH-005, ARCH-006 | API-013, API-015 | TEST-LIB-001 | TASK-011 | M3 |
| FR-LIB-004 | UX-LIB-004 | ARCH-008, ARCH-015 | API-015, [`PROVIDER-CONTRACTS.md` §5](../contracts/PROVIDER-CONTRACTS.md#5-metadataprovider) | TEST-LIB-003 | TASK-013 | M3 |
| FR-LIB-005 | UX-LIB-003, UX-LIB-005 | ARCH-006, ARCH-016 | API-015 | TEST-LIB-003 | TASK-009, TASK-012 | M2/M3 |
| FR-LIB-006 | UX-LIB-003 | ARCH-009, ARCH-010 | [`program-plan.schema.json`](../contracts/schemas/program-plan.schema.json), [`PROVIDER-CONTRACTS.md` §3](../contracts/PROVIDER-CONTRACTS.md#3-llmprovider) | TEST-LIB-004 | TASK-015 | M3 |

### 2.4 Apple Music companion mode

| Requirement | UX | Architecture | Contract | Test | Task | Milestone |
|---|---|---|---|---|---|---|
| FR-APL-001 | UX-SET-003, UX-RAD-006 | ARCH-004, ARCH-019 | API-001, API-016, [`PROVIDER-CONTRACTS.md` §2](../contracts/PROVIDER-CONTRACTS.md#2-musicsourceadapter) | TEST-APL-001 | TASK-001, TASK-025, TASK-032 | M1/M6/M7 |
| FR-APL-002 | UX-RAD-002 | ARCH-004, ARCH-016 | API-018, EVT-001, [`playback-state.schema.json`](../contracts/schemas/playback-state.schema.json) | TEST-APL-002 | TASK-001, TASK-025 | M1/M6 |
| FR-APL-003 | UX-RAD-003, UX-STA-006 | ARCH-004, ARCH-013 | API-019–API-023, EVT-001 | TEST-APL-002 | TASK-001, TASK-025 | M1/M6 |
| FR-APL-004 | UX-RAD-006 | ARCH-009, ARCH-010 | API-024, EVT-001, [`program-plan.schema.json`](../contracts/schemas/program-plan.schema.json), [`PROVIDER-CONTRACTS.md` §2.1](../contracts/PROVIDER-CONTRACTS.md#21-invariants), [`PROVIDER-CONTRACTS.md` §3.1](../contracts/PROVIDER-CONTRACTS.md#31-inputs) | TEST-APL-003 | TASK-026 | M6 |
| FR-APL-005 | UX-RAD-004, UX-RAD-006 | ARCH-013 | EVT-001, [`playback-event.schema.json`](../contracts/schemas/playback-event.schema.json), [`playback-state.schema.json`](../contracts/schemas/playback-state.schema.json), [`PROVIDER-CONTRACTS.md` §2.2](../contracts/PROVIDER-CONTRACTS.md#22-tts-interruption-protocol), [`PROVIDER-CONTRACTS.md` §4](../contracts/PROVIDER-CONTRACTS.md#4-ttsprovider) | TEST-APL-004 | TASK-026 | M6 |

### 2.5 Chat and memory

| Requirement | UX | Architecture | Contract | Test | Task | Milestone |
|---|---|---|---|---|---|---|
| FR-CHAT-001 | UX-RAD-005 | ARCH-009, ARCH-011 | API-026, EVT-004 | TEST-CHAT-001 | TASK-016, TASK-020, TASK-032 | M3/M4/M7 |
| FR-CHAT-002 | UX-RAD-005, UX-A11Y-003 | ARCH-003, ARCH-019 | API-026, EVT-004 | TEST-CHAT-002, TEST-AI-001 | TASK-020 | M4 |
| FR-CHAT-003 | UX-RAD-005 | ARCH-012 | API-038, EVT-011 | TEST-CHAT-003 | TASK-020 | M4 |
| FR-MEM-001 | UX-YOU-001 | ARCH-011, ARCH-016 | API-028, API-044, API-046, [`memory-record.schema.json`](../contracts/schemas/memory-record.schema.json) | TEST-MEM-001 | TASK-009, TASK-021, TASK-022, TASK-032 | M2/M4/M7 |
| FR-MEM-002 | UX-YOU-002 | ARCH-009, ARCH-011 | API-028, EVT-006, [`memory-record.schema.json`](../contracts/schemas/memory-record.schema.json) | TEST-MEM-001, TEST-AI-001 | TASK-021 | M4 |
| FR-MEM-003 | UX-YOU-002 | ARCH-006, ARCH-011 | API-029, API-030, API-039 | TEST-MEM-002 | TASK-021 | M4 |
| FR-MEM-004 | UX-YOU-003 | ARCH-006, ARCH-011 | API-030, API-031, API-045, [`memory-record.schema.json`](../contracts/schemas/memory-record.schema.json) | TEST-MEM-003 | TASK-007, TASK-021 | M2/M4 |
| FR-MEM-005 | UX-YOU-001, UX-YOU-004 | ARCH-006, ARCH-011 | API-046, API-047 | TEST-MEM-004 | TASK-022 | M4 |

### 2.6 Schedule and weather

| Requirement | UX | Architecture | Contract | Test | Task | Milestone |
|---|---|---|---|---|---|---|
| FR-SCH-001 | UX-SET-004 | ARCH-006, ARCH-014 | API-032, API-033, API-034, [`schedule-rule.schema.json`](../contracts/schemas/schedule-rule.schema.json) | TEST-SCH-001 | TASK-024, TASK-032 | M5/M7 |
| FR-SCH-002 | UX-WIN-001, UX-WIN-002 | ARCH-014 | API-035, EVT-007 | TEST-SCH-002 | TASK-003, TASK-024 | M1/M5 |
| FR-SCH-003 | UX-WIN-001 | ARCH-013, ARCH-014 | API-035, [`schedule-rule.schema.json`](../contracts/schemas/schedule-rule.schema.json) | TEST-SCH-003 | TASK-024 | M5 |
| FR-SCH-004 | UX-SET-004 | ARCH-013 | API-033, [`schedule-rule.schema.json`](../contracts/schemas/schedule-rule.schema.json) | TEST-SCH-001 | TASK-024 | M5 |
| FR-WEA-001 | UX-ONB-004, UX-SET-001, UX-SET-006 | ARCH-008 | API-048, API-049, [`PROVIDER-CONTRACTS.md` §6](../contracts/PROVIDER-CONTRACTS.md#6-weatherprovider) | TEST-WEA-001 | TASK-023, TASK-032 | M5/M7 |
| FR-WEA-002 | UX-RAD-001, UX-SET-006, UX-STA-005 | ARCH-008, ARCH-015 | API-007, [`PROVIDER-CONTRACTS.md` §6](../contracts/PROVIDER-CONTRACTS.md#6-weatherprovider) | TEST-WEA-002 | TASK-023, TASK-029 | M5/M7 |

### 2.7 Settings and data control

| Requirement | UX | Architecture | Contract | Test | Task | Milestone |
|---|---|---|---|---|---|---|
| FR-SET-001 | UX-SET-001, UX-SET-002 | ARCH-007, ARCH-008 | API-004–API-009 | TEST-SET-001 | TASK-008, TASK-028, TASK-032 | M2/M6/M7 |
| FR-SET-002 | UX-SET-001 | ARCH-004, ARCH-015 | API-007, API-008 | TEST-SET-002 | TASK-017, TASK-028 | M3/M6 |
| FR-SET-003 | UX-SET-004, UX-WIN-003 | ARCH-014, ARCH-018 | API-007, API-008 | TEST-SET-003 | TASK-003, TASK-028 | M1/M6 |
| FR-SET-004 | UX-SET-002, UX-SET-003 | ARCH-008, ARCH-015 | API-006, API-007 | TEST-SET-001 | TASK-006, TASK-008, TASK-009, TASK-023, TASK-028 | M2/M5/M6 |
| FR-DAT-001 | UX-SET-005 | ARCH-006, ARCH-011 | API-040, [`API-CONTRACT.md` §3.4](../contracts/API-CONTRACT.md#34-memory-schedule-and-data-control), [`DATA-MODEL.md` §5](../architecture/DATA-MODEL.md#5-数据保留与清理) | TEST-MEM-004 | TASK-007, TASK-022, TASK-032 | M2/M4/M7 |
| FR-DAT-002 | UX-SET-005 | ARCH-006, ARCH-007 | API-040 | TEST-DAT-001 | TASK-007, TASK-027 | M2/M6 |
| FR-DAT-003 | UX-SET-005, UX-STA-007 | ARCH-007, ARCH-017 | API-036, EVT-008 | TEST-DAT-002 | TASK-027 | M6 |
| FR-DAT-004 | UX-LIB-005, UX-SET-005, UX-STA-008 | ARCH-003, ARCH-006 | API-041, API-042 | TEST-DAT-003 | TASK-027 | M6 |
| FR-DAT-005 | UX-SET-005, UX-STA-008 | ARCH-007, ARCH-014, ARCH-018 | API-037 | TEST-DAT-004 | TASK-003, TASK-007, TASK-027 | M1/M2/M6 |

## 3. Non-functional requirements

### 3.1 Performance and reliability

| Requirement | UX | Architecture | Contract | Test | Task | Milestone |
|---|---|---|---|---|---|---|
| NFR-PERF-001 | UX-STA-001 | ARCH-012, ARCH-016 | API-001, [`API-CONTRACT.md` §3.1](../contracts/API-CONTRACT.md#31-app-onboarding-settings-and-secrets) | TEST-ONB-004 | TASK-010 | M2 |
| NFR-PERF-002 | UX-LIB-002 | ARCH-005, ARCH-012 | API-013, API-014, EVT-005 | TEST-LIB-002 | TASK-011 | M3 |
| NFR-PERF-003 | UX-RAD-003, UX-STA-007 | ARCH-013, ARCH-016 | API-019–API-023, API-027, EVT-001 | TEST-RAD-002 | TASK-002, TASK-012, TASK-014, TASK-019 | M1/M3 |
| NFR-PERF-004 | UX-RAD-002, UX-STA-006 | ARCH-004, ARCH-013 | EVT-001, [`playback-event.schema.json`](../contracts/schemas/playback-event.schema.json) | TEST-APL-002 | TASK-001, TASK-025 | M1/M6 |
| NFR-PERF-005 | UX-WIN-003 | ARCH-012, ARCH-018 | [`API-CONTRACT.md` §4](../contracts/API-CONTRACT.md#4-events) | TEST-RESOURCE-001 | TASK-032 | M7 |
| NFR-REL-001 | UX-RAD-002, UX-STA-003 | ARCH-005, ARCH-013 | EVT-001–EVT-003, [`playback-event.schema.json`](../contracts/schemas/playback-event.schema.json) | TEST-RAD-005 | TASK-014, TASK-018 | M3 |
| NFR-REL-002 | UX-STA-003 | ARCH-006, ARCH-013, ARCH-016 | API-018, ERR-1402, [`API-CONTRACT.md` §5](../contracts/API-CONTRACT.md#5-error-registry) | TEST-REL-001 | TASK-007, TASK-029 | M2/M7 |
| NFR-REL-003 | UX-STA-003, UX-STA-005 | ARCH-008, ARCH-015 | ERR-1302–ERR-1305, [`provider-error.schema.json`](../contracts/schemas/provider-error.schema.json) | TEST-RAD-004 | TASK-015, TASK-016, TASK-018 | M3 |
| NFR-REL-004 | UX-STA-005, UX-STA-006 | ARCH-013 | EVT-001, EVT-010, [`playback-state.schema.json`](../contracts/schemas/playback-state.schema.json), [`playback-event.schema.json`](../contracts/schemas/playback-event.schema.json) | TEST-APL-004 | TASK-002, TASK-014, TASK-024, TASK-025, TASK-026, TASK-029 | M1/M3/M5/M6/M7 |

### 3.2 Security and privacy

| Requirement | UX | Architecture | Contract | Test | Task | Milestone |
|---|---|---|---|---|---|---|
| NFR-SEC-001 | UX-ONB-003, UX-SET-002 | ARCH-007 | API-004, API-005, ERR-1101, ERR-1102, [`API-CONTRACT.md` §1.1](../contracts/API-CONTRACT.md#11-success-and-failure) | TEST-ONB-002 | TASK-003, TASK-007, TASK-028 | M1/M2/M6 |
| NFR-SEC-002 | UX-STA-003 | ARCH-001, ARCH-002, ARCH-008 | API-001–API-049, [`API-CONTRACT.md` §1](../contracts/API-CONTRACT.md#1-boundary-and-transport), [`API-CONTRACT.md` §6](../contracts/API-CONTRACT.md#6-capability-and-compatibility-rules) | TEST-SEC-001 | TASK-006, TASK-031 | M2/M7 |
| NFR-SEC-003 | UX-LIB-001, UX-STA-008 | ARCH-002, ARCH-003 | API-011, API-036, API-041, API-042, ERR-1501 | TEST-SEC-002 | TASK-007, TASK-027 | M2/M6 |
| NFR-SEC-004 | UX-STA-003 | ARCH-017 | [`API-CONTRACT.md` §1.1](../contracts/API-CONTRACT.md#11-success-and-failure), [`provider-error.schema.json`](../contracts/schemas/provider-error.schema.json), [`PROVIDER-CONTRACTS.md` §1](../contracts/PROVIDER-CONTRACTS.md#1-common-rules) | TEST-SEC-001 | TASK-006, TASK-032 | M2/M7 |
| NFR-SEC-005 | UX-SET-002 | ARCH-019 | [`DEPENDENCY-POLICY.md`](../architecture/DEPENDENCY-POLICY.md), [`API-CONTRACT.md` §6](../contracts/API-CONTRACT.md#6-capability-and-compatibility-rules) | TEST-SEC-003 | TASK-031 | M7 |
| NFR-PRIV-001 | UX-A11Y-003, UX-ONB-005 | ARCH-001, ARCH-019 | API-001, [`API-CONTRACT.md` §6](../contracts/API-CONTRACT.md#6-capability-and-compatibility-rules) | TEST-PRIV-001 | TASK-023 | M5 |
| NFR-PRIV-002 | UX-SET-005 | ARCH-006, ARCH-011 | API-040, [`API-CONTRACT.md` §3.4](../contracts/API-CONTRACT.md#34-memory-schedule-and-data-control), [`DATA-MODEL.md` §5](../architecture/DATA-MODEL.md#5-数据保留与清理) | TEST-MEM-004 | TASK-007, TASK-022 | M2/M4 |
| NFR-PRIV-003 | UX-SET-005, UX-STA-008 | ARCH-006, ARCH-007 | API-036, API-037, API-040, API-041, API-042 | TEST-DAT-002, TEST-DAT-003, TEST-DAT-004 | TASK-027 | M6 |
| NFR-PRIV-004 | UX-ONB-005, UX-SET-002, UX-SET-006 | ARCH-008, ARCH-009 | API-048, API-049, [`PROVIDER-CONTRACTS.md` §2.1](../contracts/PROVIDER-CONTRACTS.md#21-invariants), [`PROVIDER-CONTRACTS.md` §3](../contracts/PROVIDER-CONTRACTS.md#3-llmprovider), [`PROVIDER-CONTRACTS.md` §5](../contracts/PROVIDER-CONTRACTS.md#5-metadataprovider), [`PROVIDER-CONTRACTS.md` §6](../contracts/PROVIDER-CONTRACTS.md#6-weatherprovider) | TEST-LIB-003, TEST-APL-003, TEST-WEA-001, TEST-WEA-002, TEST-PRIV-001 | TASK-013, TASK-016, TASK-023 | M3/M5 |
| NFR-PRIV-005 | UX-YOU-002, UX-YOU-003 | ARCH-009, ARCH-011 | API-028–API-031, API-039, [`memory-record.schema.json`](../contracts/schemas/memory-record.schema.json) | TEST-MEM-001, TEST-MEM-003 | TASK-021 | M4 |

### 3.3 Accessibility, compatibility and cost

| Requirement | UX | Architecture | Contract | Test | Task | Milestone |
|---|---|---|---|---|---|---|
| NFR-A11Y-001 | UX-NAV-003, UX-A11Y-001 | ARCH-001 | [`API-CONTRACT.md` §2](../contracts/API-CONTRACT.md#2-shared-dtos), [`API-CONTRACT.md` §4](../contracts/API-CONTRACT.md#4-events) | TEST-A11Y-001 | TASK-010, TASK-012, TASK-019, TASK-022, TASK-030 | M2/M3/M4/M7 |
| NFR-A11Y-002 | UX-SYS-002, UX-SYS-004 | ARCH-001 | [`UX-SPEC.md` §3](../product/UX-SPEC.md#3-design-tokens), [`API-CONTRACT.md` §7](../contracts/API-CONTRACT.md#7-contract-enforcement) | TEST-A11Y-002 | TASK-009, TASK-030 | M2/M7 |
| NFR-A11Y-003 | UX-A11Y-002, UX-STA-003 | ARCH-016 | EVT-001–EVT-011, [`API-CONTRACT.md` §1.1](../contracts/API-CONTRACT.md#11-success-and-failure) | TEST-A11Y-001 | TASK-030 | M7 |
| NFR-A11Y-004 | UX-SYS-005, UX-A11Y-001 | ARCH-001 | [`UX-SPEC.md` §3.4](../product/UX-SPEC.md#34-组件图标与动效), [`API-CONTRACT.md` §7](../contracts/API-CONTRACT.md#7-contract-enforcement) | TEST-A11Y-002 | TASK-009, TASK-030 | M2/M7 |
| NFR-COMPAT-001 | UX-NAV-001, UX-WIN-003 | ARCH-018 | API-001, [`API-CONTRACT.md` §6](../contracts/API-CONTRACT.md#6-capability-and-compatibility-rules) | TEST-COMPAT-001 | TASK-003, TASK-004, TASK-028, TASK-031 | M1/M2/M6/M7 |
| NFR-COMPAT-002 | UX-LIB-002 | ARCH-005 | API-013, EVT-005 | TEST-LIB-001 | TASK-002, TASK-011 | M1/M3 |
| NFR-COMPAT-003 | UX-A11Y-001, UX-SET-003 | ARCH-004, ARCH-018 | API-001, EVT-001, [`playback-state.schema.json`](../contracts/schemas/playback-state.schema.json), [`playback-event.schema.json`](../contracts/schemas/playback-event.schema.json) | TEST-COMPAT-002 | TASK-001, TASK-009, TASK-025, TASK-030 | M1/M2/M6/M7 |
| NFR-COST-001 | UX-WIN-001, UX-ONB-004 | ARCH-008, ARCH-014 | API-006, API-009, API-024, API-026 | TEST-ONB-003, TEST-SCH-002 | TASK-008, TASK-017, TASK-020, TASK-024 | M2/M3/M4/M5 |
| NFR-COST-002 | UX-RAD-007, UX-SET-002 | ARCH-009, ARCH-010 | [`PROVIDER-CONTRACTS.md` §3](../contracts/PROVIDER-CONTRACTS.md#3-llmprovider), [`PROVIDER-CONTRACTS.md` §4](../contracts/PROVIDER-CONTRACTS.md#4-ttsprovider), [`program-plan.schema.json`](../contracts/schemas/program-plan.schema.json) | TEST-COST-001 | TASK-015, TASK-016, TASK-017 | M3 |

### 3.4 Maintainability and offline degradation

| Requirement | UX | Architecture | Contract | Test | Task | Milestone |
|---|---|---|---|---|---|---|
| NFR-MAINT-001 | UX-STA-003 | ARCH-003, ARCH-016 | [`API-CONTRACT.md` §7](../contracts/API-CONTRACT.md#7-contract-enforcement), [`schemas/`](../contracts/schemas/), [`examples/`](../contracts/examples/) | TEST-MAINT-001 | TASK-004, TASK-005, TASK-032 | M2/M7 |
| NFR-MAINT-002 | UX-STA-003 | ARCH-010, ARCH-011, ARCH-013 | [`API-CONTRACT.md` §7](../contracts/API-CONTRACT.md#7-contract-enforcement), [`program-plan.schema.json`](../contracts/schemas/program-plan.schema.json), [`memory-record.schema.json`](../contracts/schemas/memory-record.schema.json), [`playback-state.schema.json`](../contracts/schemas/playback-state.schema.json) | TEST-MAINT-002 | TASK-004, TASK-032 | M2/M7 |
| NFR-MAINT-003 | UX-STA-003 | ARCH-003, ARCH-012, ARCH-019 | [`API-CONTRACT.md` §3](../contracts/API-CONTRACT.md#3-commands), [`API-CONTRACT.md` §4](../contracts/API-CONTRACT.md#4-events), [`API-CONTRACT.md` §5](../contracts/API-CONTRACT.md#5-error-registry), [`schemas/`](../contracts/schemas/) | TEST-MAINT-001 | TASK-004, TASK-005, TASK-006, TASK-032 | M2/M7 |
| NFR-OFF-001 | UX-STA-004 | ARCH-006, ARCH-015 | local APIs API-007, API-015, API-018–API-023, API-028–API-047 | TEST-OFF-001 | TASK-029 | M7 |
| NFR-OFF-002 | UX-STA-004, UX-STA-005 | ARCH-008, ARCH-015 | [`provider-error.schema.json`](../contracts/schemas/provider-error.schema.json), ERR-1302–ERR-1305 | TEST-LIB-003, TEST-RAD-004, TEST-WEA-002 | TASK-013, TASK-023, TASK-029 | M3/M5/M7 |
| NFR-OFF-003 | UX-STA-004, UX-STA-007 | ARCH-008, ARCH-012, ARCH-015 | API-006, [`PROVIDER-CONTRACTS.md` §1](../contracts/PROVIDER-CONTRACTS.md#1-common-rules), [`API-CONTRACT.md` §1.1](../contracts/API-CONTRACT.md#11-success-and-failure) | TEST-OFF-002 | TASK-029 | M7 |

## 4. Coverage summary and audit method

本基线追踪 48 个 `FR-*` 与 34 个 `NFR-*`，每行均具有 UX、架构、契约、验收测试、原子任务和里程碑。`scripts/verify-docs.ps1` 检查 ID 唯一性、引用和需求/任务覆盖；语义审查还必须确认表中引用确实实现该断言。若新增需求，先增加权威需求 ID，再在同一变更中补齐本矩阵、验收场景和 Backlog；禁止先创建无需求任务。
