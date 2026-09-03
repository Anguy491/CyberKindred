import { useEffect, useState } from "react";

import { ControlButton } from "../components/ControlButton";
import type { FontStatus, FoundationState } from "../design/foundation";
import type { AppCapabilities } from "../ipc";
import type {
  ScheduleDueEvent,
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
  const unavailable = "[UNAVAILABLE: SETTINGS SERVICE NOT READY]";
  if (group === "AI & VOICE") {
    const providerStatus = shellState === "offline"
      ? "[OFFLINE]"
      : shellState === "degraded" ? "[DEGRADED]" : "[NOT CONFIGURED]";
    return (
      <div className="setting-stack">
        <SettingRow label="OPENAI" value={providerStatus} detail="密钥未保存到前端；仅可替换或删除。" />
        <SettingRow label="LLM" value="尚未配置" detail="模型与连接状态将分别验证。" />
        <SettingRow label="TTS" value="尚未配置" detail="关闭时主播内容仍显示文字。" />
        <div className="inline-action-row">
          <ControlButton disabledReason={unavailable}>替换密钥</ControlButton>
          <ControlButton disabledReason={unavailable}>测试连接</ControlButton>
        </div>
      </div>
    );
  }
  if (group === "PLAYBACK") {
    return (
      <div className="setting-stack">
        <SettingRow label="DEFAULT SOURCE" value="每次询问" detail="不会在启动或恢复时自动出声。" />
        <SettingRow label="TTS" value="关闭" detail="本地音乐不依赖语音能力。" />
        <ControlButton disabledReason={unavailable}>更改播放设置</ControlButton>
      </div>
    );
  }
  if (group === "APPLE MUSIC") {
    return (
      <div className="setting-stack">
        <SettingRow
          label="WINDOWS APP SESSION"
          value={capabilities.features.systemMediaSession ? "[AVAILABLE]" : "[UNAVAILABLE]"}
          detail="只读取并控制 Windows 系统媒体会话；不登录 MusicKit，不控制网页。"
        />
        <SettingRow label="CAPABILITIES" value="尚未获得" detail="每项控制只按会话实时能力启用。" />
      </div>
    );
  }
  if (group === "CONTEXT") {
    return <ContextSettings ipc={ipc} />;
  }
  if (group === "SCHEDULE") {
    return <ScheduleSettings ipc={ipc} />;
  }
  if (group === "APP") {
    return (
      <div className="setting-stack">
        <SettingRow label="LAUNCH AT STARTUP" value="关闭" detail="启动后保持静音。" />
        <SettingRow label="MINIMIZE TO TRAY" value="关闭" detail="更改后会在原位置显示结果。" />
        <ControlButton disabledReason={unavailable}>更改应用设置</ControlButton>
      </div>
    );
  }
  return (
    <div className="setting-stack">
      <SettingRow label="LOCAL DATA" value="空" detail="数据类别、数量、保留期和外发服务将在此列出。" />
      <ControlButton disabledReason={unavailable}>导出数据</ControlButton>
      <ControlButton tone="danger" disabledReason={unavailable}>删除所选数据</ControlButton>
      <p className="non-impact-copy">全部重置不会删除你的音乐文件或主动保存的导出。</p>
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
        <span className="inline-status" role="status">{status}</span>
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
        <span className="inline-status" role="status">{status}</span>
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

interface SettingRowProps {
  readonly label: string;
  readonly value: string;
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

function fontLabel(status: FontStatus): string {
  if (status === "fallback") {
    return "[FONT FALLBACK]";
  }
  return status === "loaded" ? "[LOCAL FONTS]" : "[FONT CHECK…]";
}
