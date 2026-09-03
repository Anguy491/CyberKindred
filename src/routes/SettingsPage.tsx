import { useEffect, useState } from "react";

import { ControlButton } from "../components/ControlButton";
import type { FontStatus, FoundationState } from "../design/foundation";
import type { AppCapabilities } from "../ipc";
import type { SettingsView, WeatherLocationCandidate } from "../ipc";
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
    return (
      <div className="setting-stack">
        <SettingRow label="RULES" value="0" detail="到点只通知，确认后才播放。" />
        <ControlButton disabledReason={unavailable}>创建日程</ControlButton>
      </div>
    );
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
