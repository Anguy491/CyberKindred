import { useEffect, useRef, useState, type ReactNode } from "react";

import { ControlButton } from "../components/ControlButton";
import type { FontStatus, FoundationState } from "../design/foundation";
import type { AppCapabilities } from "../ipc";
import type {
  ScheduleDueEvent,
  DataCategoryInventory,
  DataDeletionCategory,
  IntegrationStatus,
  NarrationDensity,
  SourceSummary,
  SettingsPatch,
  ScheduleRule,
  ScheduleView,
  SettingsView,
  WeatherLocationCandidate,
} from "../ipc";
import type { SettingsIpc } from "../features/settings";

const SETTINGS_GROUPS = [
  "AI & VOICE",
  "PLAYBACK",
  "APPLE MUSIC",
  "CONTEXT",
  "SCHEDULE",
  "APP",
  "PRIVACY & DATA",
] as const;

type SettingsGroup = (typeof SETTINGS_GROUPS)[number];

interface SettingsPageProps {
  readonly capabilities: AppCapabilities;
  readonly fontStatus: FontStatus;
  readonly shellState: FoundationState;
  readonly ipc: SettingsIpc;
}

export function SettingsPage({ capabilities, fontStatus, shellState, ipc }: SettingsPageProps) {
  const [activeGroup, setActiveGroup] = useState<SettingsGroup>("AI & VOICE");
  return (
    <section className="page settings-page" aria-labelledby="settings-title">
      <div className="page-primary">
        <p className="instrument-label">SETTINGS / {activeGroup}</p>
        <h1 id="settings-title" className="hero-title">{activeGroup}</h1>
      </div>

      <nav className="settings-groups" aria-label="设置分组">
        {SETTINGS_GROUPS.map((group) => (
          <button
            key={group}
            type="button"
            data-settings-group={group.toLocaleLowerCase("en-US").replaceAll(" ", "-").replaceAll("&", "and")}
            aria-current={activeGroup === group ? "page" : undefined}
            onClick={() => setActiveGroup(group)}
          >
            {group}
          </button>
        ))}
      </nav>

      <div className="page-secondary settings-content">
        <SettingsGroupContent
          group={activeGroup}
          capabilities={capabilities}
          shellState={shellState}
          ipc={ipc}
        />
      </div>

      <aside className="page-tertiary" aria-label="应用诊断">
        <dl className="instrument-list">
          <div><dt>PROTOCOL</dt><dd>{capabilities.protocolVersion}</dd></div>
          <div><dt>APP</dt><dd>{capabilities.appVersion}</dd></div>
          <div><dt>PLATFORM</dt><dd>{capabilities.platform.toUpperCase()}</dd></div>
          <div>
            <dt>FONTS</dt>
            <dd>
              <span className={fontStatus === "fallback" ? "status-warning" : undefined} role="status">
                {fontLabel(fontStatus)}
              </span>
            </dd>
          </div>
        </dl>
      </aside>
    </section>
  );
}

interface SettingsGroupContentProps {
  readonly group: SettingsGroup;
  readonly capabilities: AppCapabilities;
  readonly shellState: FoundationState;
  readonly ipc: SettingsIpc;
}

function SettingsGroupContent({ group, capabilities, shellState, ipc }: SettingsGroupContentProps) {
  if (group === "AI & VOICE") {
    return <AiVoiceSettings ipc={ipc} shellState={shellState} />;
  }
  if (group === "PLAYBACK") {
    return <PlaybackSettings ipc={ipc} />;
  }
  if (group === "APPLE MUSIC") {
    return <AppleMusicSettings ipc={ipc} supported={capabilities.features.systemMediaSession} />;
  }
  if (group === "CONTEXT") {
    return <ContextSettings ipc={ipc} />;
  }
  if (group === "SCHEDULE") {
    return <ScheduleSettings ipc={ipc} />;
  }
  if (group === "APP") {
    return <ApplicationSettings ipc={ipc} />;
  }
  return <PrivacyDataSettings ipc={ipc} />;
}

function AiVoiceSettings({ ipc, shellState }: {
  readonly ipc: SettingsIpc;
  readonly shellState: FoundationState;
}) {
  const [settings, setSettings] = useState<SettingsView | null>(null);
  const [origin, setOrigin] = useState("");
  const [llmModel, setLlmModel] = useState("");
  const [ttsModel, setTtsModel] = useState("");
  const [voiceId, setVoiceId] = useState("");
  const [voices, setVoices] = useState<ReadonlyArray<{ voiceId: string; displayName: string }>>([]);
  const [secret, setSecret] = useState("");
  const [review, setReview] = useState<ReadonlyArray<string>>([]);
  const [status, setStatus] = useState("[LOADING…]");
  const [busy, setBusy] = useState(false);

  async function refresh() {
    const [next, voiceCatalog] = await Promise.all([ipc.getSettings(), ipc.listVoices()]);
    setSettings(next);
    setOrigin(next.providerOrigin);
    setLlmModel(next.llmModelId);
    setTtsModel(next.ttsModelId);
    setVoiceId(next.ttsVoiceId);
    setVoices(voiceCatalog.voices);
    setStatus("[READY]");
  }

  useEffect(() => {
    let active = true;
    void refresh().catch(() => { if (active) setStatus("[SETTINGS UNAVAILABLE]"); });
    return () => { active = false; };
  }, [ipc]);

  const configured = settings?.secretStatus.origins.some((entry) =>
    entry.origin === settings.providerOrigin && entry.openaiApiKeyConfigured) ?? false;

  function prepareReview() {
    if (settings === null) return;
    const changes = [
      origin !== settings.providerOrigin ? `BASE URL: ${settings.providerOrigin} → ${origin}` : null,
      llmModel !== settings.llmModelId ? `LLM: ${settings.llmModelId} → ${llmModel}` : null,
      ttsModel !== settings.ttsModelId ? `TTS MODEL: ${settings.ttsModelId} → ${ttsModel}` : null,
      voiceId !== settings.ttsVoiceId ? `VOICE: ${settings.ttsVoiceId} → ${voiceId}` : null,
    ].filter((value): value is string => value !== null);
    setReview(changes);
    setStatus(changes.length === 0 ? "[NO CHANGES]" : "[REVIEW CHANGES]");
  }

  async function saveReviewed() {
    if (settings === null || review.length === 0 || busy) return;
    const patch: SettingsPatch = {
      ...(origin !== settings.providerOrigin ? { providerOrigin: origin } : {}),
      ...(llmModel !== settings.llmModelId ? { llmModelId: llmModel } : {}),
      ...(ttsModel !== settings.ttsModelId ? { ttsModelId: ttsModel } : {}),
      ...(voiceId !== settings.ttsVoiceId ? { ttsVoiceId: voiceId } : {}),
    };
    setBusy(true);
    setStatus("[SAVING…]");
    try {
      await ipc.updateSettings({
        clientRequestId: crypto.randomUUID(), expectedRevision: settings.revision, patch,
      }, settings.llmModelId);
      setReview([]);
      await refresh();
      setStatus("[SAVED]");
    } catch {
      setStatus("[SAVE FAILED · SETTINGS UNCHANGED]");
    } finally {
      setBusy(false);
    }
  }

  async function replaceSecret() {
    if (secret.length === 0 || origin.length === 0 || busy) return;
    setBusy(true);
    setStatus("[VERIFYING CREDENTIAL…]");
    try {
      await ipc.validateAndSetSecret({
        clientRequestId: crypto.randomUUID(), kind: "openai_api_key", origin, value: secret,
      });
      setSecret("");
      await refresh();
      setStatus("[CONNECTED · KEY REPLACED]");
    } catch {
      setStatus("[KEY REJECTED · EXISTING KEY KEPT]");
    } finally {
      setBusy(false);
    }
  }

  async function deleteSecret() {
    if (settings === null || !configured || busy) return;
    setBusy(true);
    try {
      await ipc.deleteSecret({
        clientRequestId: crypto.randomUUID(), kind: "openai_api_key",
        origin: settings.providerOrigin,
      });
      await refresh();
      setStatus("[KEY DELETED]");
    } catch {
      setStatus("[KEY DELETE FAILED]");
    } finally {
      setBusy(false);
    }
  }

  async function test(kind: "llm" | "tts" | "metadata" | "weather") {
    if (busy) return;
    setBusy(true);
    setStatus(`[TESTING ${kind.toUpperCase()}…]`);
    try {
      const result = await ipc.testProvider({ clientRequestId: crypto.randomUUID(), kind });
      await refresh();
      setStatus(result.ok ? `[${kind.toUpperCase()} CONNECTED · ${String(result.latencyMs)} MS]` : "[DEGRADED]");
    } catch {
      setStatus(`[${kind.toUpperCase()} TEST FAILED]`);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="setting-stack">
      <SettingRow label="OPENAI KEY" value={configured ? "•••••••• [CONFIGURED]" : "[NOT CONFIGURED]"}
        detail="密钥只进入 Rust 与 Windows Credential Manager；无法明文读取。" />
      <IntegrationRows statuses={settings?.integrationStatuses ?? []} shellState={shellState} />
      <label className="text-entry" htmlFor="openai-key">替换密钥
        <input id="openai-key" type="password" value={secret} autoComplete="new-password"
          onChange={(event) => setSecret(event.target.value)} />
      </label>
      <div className="inline-action-row">
        <ControlButton disabled={busy || secret.length === 0} onClick={() => void replaceSecret()}>验证并替换</ControlButton>
        <ControlButton tone="danger" disabled={busy || !configured} onClick={() => void deleteSecret()}>删除当前来源密钥</ControlButton>
      </div>
      <details className="advanced-settings">
        <summary>高级 Provider 设置</summary>
        <label className="text-entry" htmlFor="provider-origin">Base URL
          <input id="provider-origin" value={origin} onChange={(event) => setOrigin(event.target.value)} />
        </label>
        <label className="text-entry" htmlFor="llm-model">LLM model ID
          <input id="llm-model" value={llmModel} onChange={(event) => setLlmModel(event.target.value)} />
        </label>
        <label className="text-entry" htmlFor="tts-model">TTS model ID
          <input id="tts-model" value={ttsModel} onChange={(event) => setTtsModel(event.target.value)} />
        </label>
        <label className="text-entry" htmlFor="tts-voice">声音
          <select id="tts-voice" value={voiceId} onChange={(event) => setVoiceId(event.target.value)}>
            {voices.map((voice) => <option key={voice.voiceId} value={voice.voiceId}>{voice.displayName}</option>)}
          </select>
        </label>
        <ControlButton disabled={busy || settings === null} onClick={prepareReview}>检查变更</ControlButton>
        {review.length === 0 ? null : (
          <div className="settings-change-review" aria-label="设置变更确认">
            <ul>{review.map((item) => <li key={item}>{item}</li>)}</ul>
            <ControlButton disabled={busy} onClick={() => void saveReviewed()}>保存已列出的变更</ControlButton>
          </div>
        )}
      </details>
      <div className="inline-action-row" aria-label="连接测试">
        {(["llm", "tts", "metadata", "weather"] as const).map((kind) => (
          <ControlButton key={kind} tone="ghost" disabled={busy}
            onClick={() => void test(kind)}>测试 {kind.toUpperCase()}</ControlButton>
        ))}
      </div>
      <StatusText status={status} />
    </div>
  );
}

function IntegrationRows({ statuses, shellState }: {
  readonly statuses: ReadonlyArray<IntegrationStatus>;
  readonly shellState: FoundationState;
}) {
  if (statuses.length === 0) {
    return <SettingRow label="INTEGRATIONS" value={shellState === "offline" ? "[OFFLINE]" : "[NOT CONFIGURED]"}
      detail="每项集成会独立显示状态。" />;
  }
  return <>{statuses.map((item) => (
    <SettingRow key={item.integration} label={item.integration.toUpperCase()}
      value={`[${item.state.toUpperCase()}]`}
      detail={`${item.safeMessage} 最近成功：${formatLastSuccess(item.lastSuccessAt)}`} />
  ))}</>;
}

function PlaybackSettings({ ipc }: { readonly ipc: SettingsIpc }) {
  const [settings, setSettings] = useState<SettingsView | null>(null);
  const [sources, setSources] = useState<ReadonlyArray<SourceSummary>>([]);
  const [sourceId, setSourceId] = useState("");
  const [density, setDensity] = useState<NarrationDensity>("balanced");
  const [tts, setTts] = useState(false);
  const [status, setStatus] = useState("[LOADING…]");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let active = true;
    void Promise.all([ipc.getSettings(), ipc.listMusicSources()]).then(([next, catalog]) => {
      if (!active) return;
      setSettings(next);
      setSources(catalog.sources);
      setSourceId(next.defaultSourceId ?? "");
      setDensity(next.narrationDensity);
      setTts(next.ttsEnabled);
      setStatus("[READY · NO AUTOMATIC SOUND]");
    }, () => { if (active) setStatus("[SETTINGS UNAVAILABLE]"); });
    return () => { active = false; };
  }, [ipc]);

  async function save() {
    if (settings === null || busy) return;
    setBusy(true);
    setStatus("[SAVING…]");
    try {
      const patch: SettingsPatch = {
        defaultSourceId: sourceId === "" ? null : sourceId,
        narrationDensity: density,
        ttsEnabled: tts,
      };
      const ack = await ipc.updateSettings({
        clientRequestId: crypto.randomUUID(), expectedRevision: settings.revision, patch,
      }, settings.llmModelId);
      setSettings({ ...settings, ...patch, revision: ack.revision });
      setStatus(tts ? "[SAVED]" : "[SAVED · TTS OFF, TEXT CONTINUES]");
    } catch {
      setStatus("[SAVE FAILED]");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="setting-stack">
      <label className="text-entry" htmlFor="default-source">默认音乐源
        <select id="default-source" value={sourceId} onChange={(event) => setSourceId(event.target.value)}>
          <option value="">每次询问</option>
          {sources.map((source) => <option key={source.sourceId} value={source.sourceId}>{source.displayName}</option>)}
        </select>
      </label>
      <label className="text-entry" htmlFor="narration-density">串场密度
        <select id="narration-density" value={density}
          onChange={(event) => setDensity(event.target.value as NarrationDensity)}>
          <option value="quiet">安静</option><option value="balanced">平衡</option><option value="frequent">频繁</option>
        </select>
      </label>
      <label className="toggle-setting"><input type="checkbox" checked={tts}
        onChange={(event) => setTts(event.target.checked)} />启用 TTS</label>
      <SettingRow label="AUDIO OUTPUT"
        value={settings?.audioOutputBehavior === "fixed_device" ? "[FIXED DEVICE]" : "[FOLLOW SYSTEM DEFAULT]"}
        detail="关闭 TTS 后音乐仍可播放，所有主播内容只显示文字；保存设置不会播放测试音。" />
      <ControlButton disabled={busy || settings === null} onClick={() => void save()}>保存播放设置</ControlButton>
      <StatusText status={status} />
    </div>
  );
}

function AppleMusicSettings({ ipc, supported }: { readonly ipc: SettingsIpc; readonly supported: boolean }) {
  const [source, setSource] = useState<SourceSummary | null>(null);
  const [integration, setIntegration] = useState<IntegrationStatus | null>(null);
  const [status, setStatus] = useState("[LOADING…]");
  const [busy, setBusy] = useState(false);

  async function refresh() {
    const [catalog, settings] = await Promise.all([ipc.listMusicSources(), ipc.getSettings()]);
    const nextSource = catalog.sources.find((item) => item.sourceId === "apple_music") ?? null;
    const nextIntegration = settings.integrationStatuses.find((item) => item.integration === "apple_music") ?? null;
    setSource(nextSource);
    setIntegration(nextIntegration);
    setStatus(nextIntegration === null ? "[STATUS UNAVAILABLE]" : `[${nextIntegration.state.toUpperCase()}]`);
  }

  useEffect(() => {
    let active = true;
    void refresh().catch(() => { if (active) setStatus("[SESSION CHECK FAILED]"); });
    return () => { active = false; };
  }, [ipc]);

  async function connect() {
    if (!supported || busy) return;
    setBusy(true);
    setStatus("[CONNECTING…]");
    try {
      await ipc.selectMusicSource({
        clientRequestId: crypto.randomUUID(), sourceId: "apple_music",
      });
      await refresh();
    } catch {
      setStatus("[NO APP SESSION]");
    } finally {
      setBusy(false);
    }
  }

  const controls = source === null ? [] : Object.entries(source.capabilities)
    .filter(([name, enabled]) => name !== "setQueue" && enabled)
    .map(([name]) => name.toUpperCase());
  return (
    <div className="setting-stack">
      <SettingRow label="WINDOWS APP SESSION"
        value={<StatusText status={supported ? status : "[UNAVAILABLE]"} />}
        detail={integration?.safeMessage ?? "当前 Windows 不支持系统媒体会话。"} />
      <SettingRow label="CAPABILITIES" value={controls.length === 0 ? "尚未获得" : controls.join(" · ")}
        detail="每项控制只按当前会话实时能力启用；CyberKindred 不建立精确 Apple 队列。" />
      <p className="secondary-copy">未安装 App：请先从 Microsoft Store 安装 Apple Music Windows App。</p>
      <p className="secondary-copy">已安装但无会话：打开 App 并开始播放一首曲目。</p>
      <p className="non-impact-copy">只连接 Windows App 的 GSMTC 会话；不登录 MusicKit、不读取账户，也不控制网页。</p>
      <ControlButton tone="ghost" disabled={!supported || busy} onClick={() => void connect()}>
        连接 / 重新检查会话
      </ControlButton>
    </div>
  );
}

function ApplicationSettings({ ipc }: { readonly ipc: SettingsIpc }) {
  const [settings, setSettings] = useState<SettingsView | null>(null);
  const [status, setStatus] = useState("[LOADING…]");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let active = true;
    void ipc.getSettings().then((next) => {
      if (active) { setSettings(next); setStatus("[READY · STARTUP STAYS SILENT]"); }
    }, () => { if (active) setStatus("[SETTINGS UNAVAILABLE]"); });
    return () => { active = false; };
  }, [ipc]);

  async function toggle(field: "minimizeToTray" | "launchAtStartup" | "notificationsEnabled") {
    if (settings === null || busy) return;
    const next = !settings[field];
    setBusy(true);
    setStatus("[APPLYING WINDOWS SETTING…]");
    try {
      const patch: SettingsPatch = { [field]: next };
      const ack = await ipc.updateSettings({
        clientRequestId: crypto.randomUUID(), expectedRevision: settings.revision, patch,
      }, settings.llmModelId);
      setSettings({ ...settings, [field]: next, revision: ack.revision });
      setStatus(next ? "[ENABLED · NO SOUND]" : "[DISABLED]");
    } catch {
      setStatus("[WINDOWS SETTING FAILED · VALUE UNCHANGED]");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="setting-stack">
      <SettingRow label="LAUNCH AT STARTUP" value={settings?.launchAtStartup ? "[ENABLED]" : "[DISABLED]"}
        detail="首次安装默认关闭；启用后随当前 Windows 用户登录启动，启动后保持静音。" />
      <ControlButton disabled={busy || settings === null}
        data-testid={settings?.launchAtStartup ? "app-autostart-disable" : "app-autostart-enable"}
        onClick={() => void toggle("launchAtStartup")}>{settings?.launchAtStartup ? "关闭登录启动" : "启用登录启动"}</ControlButton>
      <SettingRow label="MINIMIZE TO TRAY" value={settings?.minimizeToTray ? "[ENABLED]" : "[DISABLED]"}
        detail="启用后关闭主窗口会隐藏到托盘；托盘操作仍遵守实时播放能力。" />
      <ControlButton disabled={busy || settings === null}
        data-testid={settings?.minimizeToTray ? "app-tray-disable" : "app-tray-enable"}
        onClick={() => void toggle("minimizeToTray")}>{settings?.minimizeToTray ? "关闭托盘运行" : "启用托盘运行"}</ControlButton>
      <SettingRow label="NOTIFICATIONS" value={settings?.notificationsEnabled ? "[ENABLED]" : "[DISABLED]"}
        detail="日程到点只通知，不自动开播。" />
      <ControlButton disabled={busy || settings === null}
        onClick={() => void toggle("notificationsEnabled")}>{settings?.notificationsEnabled ? "关闭通知" : "启用通知"}</ControlButton>
      <StatusText status={status} />
    </div>
  );
}

const WEEKDAYS = [
  ["mon", "一"], ["tue", "二"], ["wed", "三"], ["thu", "四"],
  ["fri", "五"], ["sat", "六"], ["sun", "日"],
] as const;
type Weekday = ScheduleRule["daysOfWeek"][number];

function ScheduleSettings({ ipc }: { readonly ipc: SettingsIpc }) {
  const [schedules, setSchedules] = useState<ReadonlyArray<ScheduleView>>([]);
  const [revision, setRevision] = useState<number | null>(null);
  const [settings, setSettings] = useState<SettingsView | null>(null);
  const [name, setName] = useState("Morning radio");
  const [localTime, setLocalTime] = useState("07:00");
  const [timezone, setTimezone] = useState(defaultTimezone);
  const [days, setDays] = useState<ReadonlyArray<Weekday>>(["mon", "tue", "wed", "thu", "fri"]);
  const [due, setDue] = useState<ScheduleDueEvent | null>(null);
  const [status, setStatus] = useState("[LOADING…]");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;
    const refresh = async () => {
      const response = await ipc.listSchedules();
      if (active) {
        setSchedules(response.schedules);
        setRevision(response.revision);
        setStatus(response.schedules.length === 0 ? "[NO RULES]" : "[READY]");
      }
    };
    void ipc.getSettings().then(
      (value) => { if (active) setSettings(value); },
      () => { if (active) setStatus("[SETTINGS UNAVAILABLE]"); },
    );
    void ipc.subscribeToEvents({
      onEvent(eventName, payload) {
        if (!active || eventName !== "cyberkindred://v1/schedule/due") return;
        const event = payload as unknown as ScheduleDueEvent;
        if (event.notificationShown) {
          setDue(event);
          setStatus("[DUE · WAITING FOR YOU]");
        } else {
          setStatus("[MISSED · NO NOTIFICATION SHOWN]");
        }
      },
      refreshSnapshot: async () => { await refresh(); },
    }).then(
      (stop) => { if (active) unlisten = stop; else stop(); },
      () => { if (active) setStatus("[EVENTS UNAVAILABLE]"); },
    );
    return () => { active = false; unlisten?.(); };
  }, [ipc]);

  const validForm = name.trim().length >= 1 && Array.from(name.trim()).length <= 80
    && days.length >= 1 && /^(?:[01][0-9]|2[0-3]):[0-5][0-9]$/u.test(localTime)
    && isIanaTimezone(timezone);

  function toggleDay(day: Weekday) {
    setDays((current) => current.includes(day)
      ? current.filter((value) => value !== day)
      : WEEKDAYS.map(([value]) => value).filter((value) => current.includes(value) || value === day));
  }

  async function createSchedule() {
    if (!validForm || revision === null || busy) return;
    setBusy(true);
    setStatus("[SAVING…]");
    const now = new Date().toISOString();
    try {
      const response = await ipc.upsertSchedule({
        clientRequestId: crypto.randomUUID(),
        expectedRevision: revision,
        schedule: {
          schemaVersion: "1.0.0",
          scheduleId: crypto.randomUUID(),
          name: name.trim(),
          timezone,
          daysOfWeek: days,
          localTime,
          enabled: true,
          notificationOnly: true,
          createdAt: now,
          updatedAt: now,
          revision: 0,
        },
      });
      setSchedules((current) => [...current, response.schedule]);
      setRevision(response.revision);
      setStatus("[SAVED · NOTIFICATION ONLY]");
    } catch {
      setStatus("[SAVE FAILED · REFRESH SCHEDULES]");
    } finally {
      setBusy(false);
    }
  }

  async function updateSchedule(view: ScheduleView, enabled: boolean) {
    if (revision === null || busy) return;
    setBusy(true);
    setStatus("[SAVING…]");
    try {
      const response = await ipc.upsertSchedule({
        clientRequestId: crypto.randomUUID(), expectedRevision: revision,
        schedule: { ...view.rule, enabled, updatedAt: new Date().toISOString() },
      });
      setSchedules((current) => current.map((item) => item.rule.scheduleId === view.rule.scheduleId
        ? response.schedule : item));
      setRevision(response.revision);
      setStatus(enabled ? "[RULE ENABLED]" : "[RULE PAUSED]");
    } catch {
      setStatus("[SAVE FAILED · REFRESH SCHEDULES]");
    } finally {
      setBusy(false);
    }
  }

  async function removeSchedule(view: ScheduleView) {
    if (revision === null || busy) return;
    setBusy(true);
    setStatus("[DELETING…]");
    try {
      const response = await ipc.deleteSchedule({
        clientRequestId: crypto.randomUUID(),
        scheduleId: view.rule.scheduleId,
        expectedRevision: revision,
      });
      setSchedules((current) => current.filter((item) => item.rule.scheduleId !== view.rule.scheduleId));
      setRevision(response.revision);
      setStatus("[RULE DELETED]");
    } catch {
      setStatus("[DELETE FAILED · REFRESH SCHEDULES]");
    } finally {
      setBusy(false);
    }
  }

  async function toggleNotifications() {
    if (settings === null || busy) return;
    setBusy(true);
    setStatus("[SAVING…]");
    try {
      const next = !settings.notificationsEnabled;
      const response = await ipc.updateSettings({
        clientRequestId: crypto.randomUUID(), expectedRevision: settings.revision,
        patch: { notificationsEnabled: next },
      }, settings.llmModelId);
      setSettings({ ...settings, notificationsEnabled: next, revision: response.revision });
      setStatus(next ? "[NOTIFICATIONS ON · STILL SILENT]" : "[NOTIFICATIONS OFF]");
    } catch {
      setStatus("[SETTINGS SAVE FAILED]");
    } finally {
      setBusy(false);
    }
  }

  async function actOnDue(
    action: "open" | "dismiss" | "start" | "snooze",
    snoozeMinutes: 10 | 30 | 60 | null,
  ) {
    if (due === null || busy) return;
    setBusy(true);
    setStatus("[RECORDING YOUR CHOICE…]");
    try {
      const response = await ipc.handleNotificationAction({
        clientRequestId: crypto.randomUUID(),
        scheduleId: due.scheduleId,
        occurrenceId: due.occurrenceId,
        action,
        snoozeMinutes,
      });
      if (response.status === "starting") {
        await ipc.startProgram({
          clientRequestId: crypto.randomUUID(), sourceId: "local", trigger: "notification",
        });
        setStatus("[PROGRAM START REQUESTED]");
        setDue(null);
      } else if (response.status === "snoozed") {
        setStatus(`[SNOOZED · ${String(snoozeMinutes)} MIN]`);
        setDue(null);
      } else if (response.status === "dismissed") {
        setStatus("[DISMISSED · NO SOUND]");
        setDue(null);
      } else {
        setStatus("[OPEN · WAITING FOR YOUR CHOICE]");
      }
    } catch {
      setStatus("[ACTION FAILED]");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="setting-stack">
      <SettingRow
        label="RULES"
        value={String(schedules.length)}
        detail="重复规则使用 IANA 时区；到点只显示静音通知，绝不自动开播或调用付费服务。"
      />
      <div className="inline-action-row">
        <ControlButton disabled={busy || settings === null} onClick={() => void toggleNotifications()}>
          {settings?.notificationsEnabled ? "关闭系统通知" : "启用系统通知"}
        </ControlButton>
        <StatusText status={status} />
      </div>
      {due === null ? null : (
        <section className="schedule-due" aria-label="到点节目选择">
          <strong>节目时间到了</strong>
          <p>只有“开始节目”会确认开播；打开或稍后提醒不会播放声音。</p>
          <div className="inline-action-row">
            <ControlButton disabled={busy} onClick={() => void actOnDue("start", null)}>开始节目</ControlButton>
            <ControlButton tone="ghost" disabled={busy} onClick={() => void actOnDue("open", null)}>保持打开</ControlButton>
            <ControlButton tone="ghost" disabled={busy} onClick={() => void actOnDue("dismiss", null)}>忽略</ControlButton>
          </div>
          <div className="inline-action-row" aria-label="稍后提醒">
            {([10, 30, 60] as const).map((minutes) => (
              <ControlButton key={minutes} tone="ghost" disabled={busy}
                onClick={() => void actOnDue("snooze", minutes)}>{minutes} 分钟后</ControlButton>
            ))}
          </div>
        </section>
      )}
      <div className="schedule-form" aria-label="创建重复日程">
        <label className="text-entry" htmlFor="schedule-name">名称
          <input id="schedule-name" type="text" maxLength={80} value={name}
            onChange={(event) => setName(event.target.value)} />
        </label>
        <label className="text-entry" htmlFor="schedule-time">本地时间
          <input id="schedule-time" type="time" value={localTime}
            onChange={(event) => setLocalTime(event.target.value)} />
        </label>
        <label className="text-entry" htmlFor="schedule-timezone">IANA 时区
          <input id="schedule-timezone" type="text" maxLength={64} value={timezone}
            onChange={(event) => setTimezone(event.target.value)} />
        </label>
        <fieldset className="schedule-weekdays">
          <legend>星期</legend>
          {WEEKDAYS.map(([day, label]) => (
            <label key={day}><input type="checkbox" checked={days.includes(day)}
              onChange={() => toggleDay(day)} />{label}</label>
          ))}
        </fieldset>
        <ControlButton disabled={busy || revision === null || !validForm}
          data-testid="schedule-create"
          onClick={() => void createSchedule()}>创建日程</ControlButton>
      </div>
      {schedules.length === 0 ? <p className="secondary-copy">尚无重复日程。</p> : (
        <ul className="schedule-list" aria-label="重复日程">
          {schedules.map((view) => (
            <li key={view.rule.scheduleId} data-testid="schedule-rule">
              <div><strong>{view.rule.name}</strong><span>
                {formatDays(view.rule.daysOfWeek)} · {view.rule.localTime} · {view.rule.timezone}
              </span><small>下次：{formatTimestamp(view.nextOccurrenceAt)}</small></div>
              <div className="inline-action-row">
                <ControlButton tone="ghost" disabled={busy}
                  onClick={() => void updateSchedule(view, !view.rule.enabled)}>
                  {view.rule.enabled ? "暂停" : "启用"}
                </ControlButton>
                <ControlButton tone="danger" disabled={busy}
                  onClick={() => void removeSchedule(view)}>删除</ControlButton>
              </div>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function defaultTimezone(): string {
  const timezone = Intl.DateTimeFormat().resolvedOptions().timeZone;
  return isIanaTimezone(timezone) ? timezone : "Etc/UTC";
}

function isIanaTimezone(value: string): boolean {
  if (!/^[A-Za-z_+-]+(?:\/[A-Za-z0-9_+-]+)+$/u.test(value)) return false;
  try {
    new Intl.DateTimeFormat("en", { timeZone: value }).format(0);
    return true;
  } catch {
    return false;
  }
}

function formatDays(days: ScheduleRule["daysOfWeek"]): string {
  return days.map((day) => WEEKDAYS.find(([value]) => value === day)?.[1] ?? day).join("、");
}

function formatTimestamp(value: string | null): string {
  return value === null ? "已暂停" : new Date(value).toLocaleString();
}

function formatLastSuccess(value: string | null): string {
  return value === null ? "尚无记录" : formatTimestamp(value);
}

function ContextSettings({ ipc }: { readonly ipc: SettingsIpc }) {
  const [settings, setSettings] = useState<SettingsView | null>(null);
  const [query, setQuery] = useState("");
  const [candidates, setCandidates] = useState<ReadonlyArray<WeatherLocationCandidate>>([]);
  const [status, setStatus] = useState("[LOADING…]");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let active = true;
    void ipc.getSettings().then(
      (value) => {
        if (active) {
          setSettings(value);
          setStatus(value.weatherLocation === null ? "[NOT CONFIGURED]" : "[READY]");
        }
      },
      () => { if (active) setStatus("[UNAVAILABLE]"); },
    );
    return () => { active = false; };
  }, [ipc]);

  const locationLabel = settings?.weatherLocation === null || settings === null
    ? "尚未配置"
    : [settings.weatherLocation.city, settings.weatherLocation.region, settings.weatherLocation.country]
      .filter(Boolean).join(" · ");
  const queryLength = Array.from(query.trim()).length;

  async function search() {
    if (queryLength < 2 || queryLength > 100 || busy) return;
    setBusy(true);
    setStatus("[SEARCHING…]");
    try {
      const response = await ipc.searchWeatherLocations({
        clientRequestId: crypto.randomUUID(), query: query.trim(), limit: 8,
      });
      setCandidates(response.candidates);
      setStatus(response.candidates.length === 0 ? "[NO MATCHES]" : "[SELECT A CITY]");
    } catch {
      setCandidates([]);
      setStatus("[WEATHER UNAVAILABLE]");
    } finally {
      setBusy(false);
    }
  }

  async function select(candidate: WeatherLocationCandidate) {
    if (settings === null || busy) return;
    setBusy(true);
    setStatus("[SAVING…]");
    try {
      const selected = await ipc.selectWeatherLocation({
        clientRequestId: crypto.randomUUID(),
        candidateId: candidate.candidateId,
        expectedRevision: settings.revision,
      });
      let revision = selected.revision;
      if (!settings.weatherEnabled) {
        const ack = await ipc.updateSettings({
          clientRequestId: crypto.randomUUID(), expectedRevision: revision,
          patch: { weatherEnabled: true },
        }, settings.llmModelId);
        revision = ack.revision;
      }
      setSettings({
        ...settings,
        weatherLocation: selected.location,
        weatherEnabled: true,
        revision,
      });
      setCandidates([]);
      setStatus("[SAVED · WEATHER CONTEXT ON]");
    } catch {
      setStatus("[SAVE FAILED · REFRESH SETTINGS]");
    } finally {
      setBusy(false);
    }
  }

  async function clearLocation() {
    if (settings?.weatherLocation === null || settings === null || busy) return;
    setBusy(true);
    setStatus("[CLEARING…]");
    try {
      const ack = await ipc.updateSettings({
        clientRequestId: crypto.randomUUID(), expectedRevision: settings.revision,
        patch: { weatherEnabled: false, weatherLocationAction: "clear" },
      }, settings.llmModelId);
      setSettings({ ...settings, weatherLocation: null, weatherEnabled: false, revision: ack.revision });
      setCandidates([]);
      setStatus("[NOT CONFIGURED]");
    } catch {
      setStatus("[CLEAR FAILED · REFRESH SETTINGS]");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="setting-stack" id="context-attribution">
      <SettingRow
        label="CITY"
        value={locationLabel}
        detail="只在你点击搜索时发送城市词；不会请求设备定位或自动使用 IP 定位。"
      />
      <label className="text-entry" htmlFor="weather-city-query">
        手动搜索城市
        <input
          id="weather-city-query"
          data-testid="weather-city-query"
          value={query}
          maxLength={100}
          autoComplete="off"
          onChange={(event) => setQuery(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              void search();
            }
          }}
        />
      </label>
      <div className="inline-action-row">
        <ControlButton
          disabled={busy || queryLength < 2 || queryLength > 100}
          onClick={() => void search()}
          data-testid="weather-search"
        >搜索城市</ControlButton>
        <ControlButton
          tone="ghost"
          disabled={busy || settings?.weatherLocation === null || settings === null}
          onClick={() => void clearLocation()}
        >清除城市</ControlButton>
        <StatusText status={status} />
      </div>
      {candidates.length > 0 ? (
        <ul className="weather-candidate-list" aria-label="城市搜索结果">
          {candidates.map((candidate) => (
            <li key={candidate.candidateId}>
              <button type="button" disabled={busy} onClick={() => void select(candidate)}>
                <strong>{candidate.city}</strong>
                <span>{[candidate.region, candidate.country].filter(Boolean).join(" · ")}</span>
                <small>{candidate.timezone}</small>
              </button>
            </li>
          ))}
        </ul>
      ) : null}
      <a
        className="disabled-link"
        href="https://open-meteo.com/"
        target="_blank"
        rel="noreferrer"
      >Geocoding data © GeoNames, weather data by Open-Meteo</a>
    </div>
  );
}

const DELETION_LABELS: Readonly<Record<DataDeletionCategory, string>> = {
  profile_and_memories: "画像与记忆",
  conversations_and_summaries: "对话与摘要",
  playback_history: "节目与播放历史",
  metadata_cache: "元数据与缓存",
  library_index: "曲库索引",
};

function PrivacyDataSettings({ ipc }: { readonly ipc: SettingsIpc }) {
  const [inventory, setInventory] = useState<ReadonlyArray<DataCategoryInventory>>([]);
  const [category, setCategory] = useState<DataDeletionCategory>("profile_and_memories");
  const [preview, setPreview] = useState<Awaited<ReturnType<SettingsIpc["previewDataDeletion"]>> | null>(null);
  const [confirmation, setConfirmation] = useState("");
  const [resetConfirmation, setResetConfirmation] = useState("");
  const [status, setStatus] = useState("[LOADING…]");
  const [busy, setBusy] = useState(false);
  const exportOperation = useRef<string | null>(null);

  async function refresh() {
    const response = await ipc.getDataInventory();
    setInventory(response.categories);
    setStatus(`[READY · ${String(response.categories.length)} DATA CLASSES]`);
  }

  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;
    void refresh().catch(() => { if (active) setStatus("[INVENTORY UNAVAILABLE]"); });
    void ipc.subscribeToEvents({
      onEvent(eventName, payload) {
        if (!active || exportOperation.current === null
          || payload.operationId !== exportOperation.current) return;
        if (eventName === "cyberkindred://v1/operation/completed") setStatus("[EXPORT SAVED]");
        if (eventName === "cyberkindred://v1/operation/cancelled") setStatus("[EXPORT CANCELLED]");
        if (eventName === "cyberkindred://v1/operation/failed") setStatus("[EXPORT FAILED]");
      },
      refreshSnapshot: async () => { if (active) await refresh(); },
    }).then((stop) => { if (active) unlisten = stop; else stop(); }, () => undefined);
    return () => { active = false; unlisten?.(); };
  }, [ipc]);

  async function createPreview() {
    if (busy) return;
    setBusy(true);
    setConfirmation("");
    try {
      const response = await ipc.previewDataDeletion(category);
      setPreview(response);
      setStatus(`[PREVIEW · ${String(response.itemCount)} ITEMS]`);
    } catch {
      setStatus("[PREVIEW FAILED]");
    } finally {
      setBusy(false);
    }
  }

  async function deleteCategory() {
    if (busy || preview === null || confirmation !== "DELETE SELECTED DATA") return;
    setBusy(true);
    try {
      const response = await ipc.deleteDataCategory({
        clientRequestId: crypto.randomUUID(), previewToken: preview.previewToken,
        category: preview.category, confirmation: "DELETE SELECTED DATA",
      });
      setPreview(null);
      setConfirmation("");
      await refresh();
      setStatus(`[DELETED · ${String(response.deletedCount)} ITEMS]`);
    } catch {
      setStatus("[DELETE FAILED · CREATE A NEW PREVIEW]");
      setPreview(null);
    } finally {
      setBusy(false);
    }
  }

  async function exportData() {
    if (busy) return;
    setBusy(true);
    try {
      const accepted = await ipc.exportUserData(crypto.randomUUID());
      exportOperation.current = accepted.operationId;
      setStatus("[EXPORT PICKER OPEN]");
    } catch {
      setStatus("[EXPORT NOT STARTED]");
    } finally {
      setBusy(false);
    }
  }

  async function resetAll() {
    if (busy || resetConfirmation !== "DELETE CYBERKINDRED DATA") return;
    setBusy(true);
    try {
      await ipc.deleteAllUserData(crypto.randomUUID(), "DELETE CYBERKINDRED DATA");
      setInventory([]);
      setStatus("[RESET COMPLETE · RESTART REQUIRED]");
    } catch {
      setStatus("[RESET FAILED · DATA NOT REPORTED AS DELETED]");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="setting-stack privacy-data-settings">
      <SettingRow
        label="LOCAL DATA INVENTORY"
        value={String(inventory.length)}
        detail="清单只显示类别、数量、介质、保留期和外发对象；不显示正文、密钥或路径。"
      />
      <StatusText status={status} />
      <ul className="data-inventory-list" aria-label="本地数据清单">
        {inventory.map((entry) => (
          <li key={entry.category} data-testid="data-inventory-row">
            <strong>{entry.category.replaceAll("_", " ").toUpperCase()}</strong>
            <span>{entry.itemCount} 项 · {entry.storageClasses.join(" / ")}</span>
            <small>{entry.retentionSummary}</small>
            <small>外发：{entry.externalRecipients.length === 0 ? "无" : entry.externalRecipients.join("、")}</small>
          </li>
        ))}
      </ul>
      <ControlButton disabled={busy} onClick={() => void exportData()}>导出数据</ControlButton>

      <section className="destructive-data-control" aria-label="分类删除">
        <label className="text-entry" htmlFor="data-delete-category">删除类别
          <select id="data-delete-category" value={category} disabled={busy}
            onChange={(event) => {
              setCategory(event.target.value as DataDeletionCategory);
              setPreview(null);
              setConfirmation("");
            }}>
            {Object.entries(DELETION_LABELS).map(([value, label]) => (
              <option key={value} value={value}>{label}</option>
            ))}
          </select>
        </label>
        <ControlButton tone="danger" disabled={busy} onClick={() => void createPreview()}>
          预览删除影响
        </ControlButton>
        {preview === null ? null : (
          <div className="deletion-preview" data-testid="data-deletion-preview">
            <strong>{DELETION_LABELS[preview.category]} · {preview.itemCount} 项</strong>
            <ul>{preview.consequences.map((item) => <li key={item}>{item}</li>)}</ul>
            <label className="text-entry" htmlFor="data-delete-confirmation">
              输入 DELETE SELECTED DATA
              <input id="data-delete-confirmation" value={confirmation} autoComplete="off"
                onChange={(event) => setConfirmation(event.target.value)} />
            </label>
            <ControlButton tone="danger" disabled={busy || confirmation !== "DELETE SELECTED DATA"}
              onClick={() => void deleteCategory()}>永久删除所选类别</ControlButton>
          </div>
        )}
      </section>

      <section className="destructive-data-control" aria-label="全部重置">
        <p className="non-impact-copy">全部重置不可恢复，但不会删除源音乐文件或已保存到应用目录之外的导出。</p>
        <label className="text-entry" htmlFor="data-reset-confirmation">
          输入 DELETE CYBERKINDRED DATA
          <input id="data-reset-confirmation" value={resetConfirmation} autoComplete="off"
            onChange={(event) => setResetConfirmation(event.target.value)} />
        </label>
        <ControlButton tone="danger" disabled={busy || resetConfirmation !== "DELETE CYBERKINDRED DATA"}
          onClick={() => void resetAll()}>全部重置并要求重启</ControlButton>
      </section>
    </div>
  );
}

interface SettingRowProps {
  readonly label: string;
  readonly value: ReactNode;
  readonly detail: string;
}

function SettingRow({ label, value, detail }: SettingRowProps) {
  return (
    <div className="setting-row">
      <span className="instrument-label">{label}</span>
      <strong>{value}</strong>
      <span>{detail}</span>
    </div>
  );
}

function StatusText({ status }: { readonly status: string }) {
  const assertive = /(?:ERROR|FAILED|UNAVAILABLE|REJECTED|NO APP SESSION)/u.test(status);
  return (
    <span
      className={`inline-status${assertive ? " inline-status--error" : ""}`}
      role={assertive ? "alert" : "status"}
      aria-atomic="true"
    >
      {status}
    </span>
  );
}

function fontLabel(status: FontStatus): string {
  if (status === "fallback") {
    return "[FONT FALLBACK]";
  }
  return status === "loaded" ? "[LOCAL FONTS]" : "[FONT CHECK…]";
}
