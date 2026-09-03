import { useCallback, useEffect, useState } from "react";

import { ControlButton } from "../../components/ControlButton";
import { IpcInvocationError, type MemoryRecord, type ProfileViewResponse, type SessionSummaryView } from "../../ipc";
import type { YouIpc } from "./types";
import "./you.css";

export function YouView({ ipc }: { readonly ipc: YouIpc }) {
  const [profile, setProfile] = useState<ProfileViewResponse | null>(null);
  const [memories, setMemories] = useState<ReadonlyArray<MemoryRecord>>([]);
  const [summaries, setSummaries] = useState<ReadonlyArray<SessionSummaryView>>([]);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true); setError(null);
    try {
      const [nextProfile, memoryPage, summaryPage] = await Promise.all([
        ipc.getProfile(), ipc.listMemories(), ipc.listSummaries(),
      ]);
      setProfile(nextProfile); setMemories(memoryPage.items); setSummaries(summaryPage.items);
    } catch (cause) { setError(safeError(cause)); }
    finally { setLoading(false); }
  }, [ipc]);

  useEffect(() => { void refresh(); }, [refresh]);

  const mutate = async (operation: () => Promise<unknown>) => {
    if (busy) return;
    setBusy(true); setError(null);
    try { await operation(); await refresh(); } catch (cause) { setError(safeError(cause)); }
    finally { setBusy(false); }
  };

  if (loading && profile === null) return <section className="page" aria-labelledby="you-title">
    <div className="page-primary"><p className="instrument-label">YOU / LOADING</p>
      <h1 id="you-title" className="hero-title">正在读取本机画像</h1></div>
    <p className="inline-status" role="status">[LOADING…]</p>
  </section>;

  return <section className="page you-page" aria-labelledby="you-title">
    <div className="page-primary">
      <p className="instrument-label">YOU / LOCAL PROFILE</p>
      <h1 id="you-title" className="hero-title">CyberKindred 目前如何了解你</h1>
      <p className="secondary-copy">只有已批准且启用的记忆进入下一轮 Context。提案、停用项和已删除正文严格隔离。</p>
    </div>

    <div className="page-secondary knowledge-sections">
      {error !== null ? <p role="alert" className="inline-error">{error}</p> : null}
      {profile === null ? <ControlButton onClick={() => void refresh()}>重试</ControlButton>
        : <>
          <ProfileEditor profile={profile} disabled={busy} onSave={(patch) => mutate(() => ipc.updateProfile(profile.revision, patch))} />
          <section aria-labelledby="trend-heading">
            <h2 id="trend-heading">偏好趋势</h2>
            {profile.preferenceTrends.length === 0 ? <p className="empty-detail">还没有足够的显式反馈。</p>
              : <ul className="knowledge-list">{profile.preferenceTrends.map((trend) => <li key={trend.kind}>
                <p>{trend.label}：{trendDirection(trend.direction)}</p>
                <small>来源：本机反馈聚合 · 状态：有效 · {trend.windowDays} 天 / {trend.sampleCount} 个样本</small>
              </li>)}</ul>}
          </section>
        </>}

      <MemorySection title="待确认记忆" status="proposed" memories={memories} disabled={busy} mutate={mutate} ipc={ipc} />
      <MemorySection title="已批准记忆" status="approved" memories={memories} disabled={busy} mutate={mutate} ipc={ipc} />
      <MemorySection title="已停用记忆" status="disabled" memories={memories} disabled={busy} mutate={mutate} ipc={ipc} />

      <section aria-labelledby="summary-heading">
        <h2 id="summary-heading">会话摘要</h2>
        {summaries.length === 0 ? <p className="empty-detail">还没有长期摘要。</p>
          : <ul className="knowledge-list">{summaries.map((summary) => <li key={summary.summaryId}>
            <p>{summary.summary}</p>
            <small>来源：{summary.generationKind === "llm" ? "AI 摘要" : "本机确定性统计"} · 状态：有效 · 覆盖 {formatDate(summary.coveredFrom)}—{formatDate(summary.coveredTo)}</small>
            <ControlButton {...(busy ? { disabledReason: "[SAVING]" } : {})}
              onClick={() => void mutate(() => ipc.deleteSummary(summary.summaryId, summary.revision))}>删除摘要</ControlButton>
          </li>)}</ul>}
      </section>
    </div>

    <aside className="page-tertiary" aria-label="了解数据状态">
      <dl className="instrument-list">
        <div><dt>UPDATED</dt><dd>{profile === null ? "读取失败" : `REV ${profile.revision}`}</dd></div>
        <div><dt>SOURCE</dt><dd>仅本机 + 用户审批</dd></div>
        <div><dt>PROPOSALS</dt><dd>{memories.filter((memory) => memory.status === "proposed").length}</dd></div>
        <div><dt>SUMMARIES</dt><dd>{summaries.length}</dd></div>
      </dl>
    </aside>
  </section>;
}

function ProfileEditor({ profile, disabled, onSave }: {
  readonly profile: ProfileViewResponse;
  readonly disabled: boolean;
  readonly onSave: (patch: { readonly displayName: string; readonly initialPreferences: ReadonlyArray<string>; readonly narrationDensity: "quiet" | "balanced" | "frequent" }) => Promise<void>;
}) {
  const [displayName, setDisplayName] = useState(profile.profile.displayName);
  const [preferences, setPreferences] = useState(profile.profile.initialPreferences.join("，"));
  const [density, setDensity] = useState(profile.profile.narrationDensity);
  return <section aria-labelledby="profile-heading">
    <h2 id="profile-heading">画像</h2>
    <label className="text-entry">称呼<input maxLength={80} value={displayName} onChange={(event) => setDisplayName(event.currentTarget.value)} /></label>
    <label className="text-entry">初始偏好（以逗号分隔）<input value={preferences} onChange={(event) => setPreferences(event.currentTarget.value)} /></label>
    <label className="select-entry">串场密度<select value={density} onChange={(event) => setDensity(event.currentTarget.value as typeof density)}>
      <option value="quiet">安静</option><option value="balanced">平衡</option><option value="frequent">较多</option>
    </select></label>
    <p>位置来源：{profile.profile.weatherLocation === null ? "未设置" : `${profile.profile.weatherLocation.city}（用户选择）`}</p>
    <ControlButton tone="primary" {...(disabled ? { disabledReason: "[SAVING]" } : {})}
      onClick={() => void onSave({ displayName, initialPreferences: splitPreferences(preferences), narrationDensity: density })}>保存画像</ControlButton>
  </section>;
}

function MemorySection({ title, status, memories, disabled, mutate, ipc }: {
  readonly title: string;
  readonly status: MemoryRecord["status"];
  readonly memories: ReadonlyArray<MemoryRecord>;
  readonly disabled: boolean;
  readonly mutate: (operation: () => Promise<unknown>) => Promise<void>;
  readonly ipc: YouIpc;
}) {
  const items = memories.filter((memory) => memory.status === status);
  const heading = `memory-${status}`;
  return <section aria-labelledby={heading}>
    <h2 id={heading}>{title}</h2>
    {items.length === 0 ? <p className="empty-detail">暂无{title}。</p>
      : <ul className="knowledge-list">{items.map((memory) => <MemoryEditor key={`${memory.memoryId}:${memory.revision}`}
        memory={memory} disabled={disabled} mutate={mutate} ipc={ipc} />)}</ul>}
  </section>;
}

function MemoryEditor({ memory, disabled, mutate, ipc }: {
  readonly memory: MemoryRecord;
  readonly disabled: boolean;
  readonly mutate: (operation: () => Promise<unknown>) => Promise<void>;
  readonly ipc: YouIpc;
}) {
  const [content, setContent] = useState(memory.content);
  const disabledReason = disabled ? "[SAVING]" : undefined;
  return <li>
    <textarea aria-label="记忆内容" maxLength={500} value={content} onChange={(event) => setContent(event.currentTarget.value)} />
    <small>来源：{memory.sourceSessionId === null ? "本机聚合" : `会话 ${shortId(memory.sourceSessionId)}`} · 状态：{memory.status} · 最近使用：{memory.lastUsedAt === null ? "尚未进入 Context" : formatDate(memory.lastUsedAt)}</small>
    <div className="knowledge-actions">
      <ControlButton {...(disabledReason === undefined ? {} : { disabledReason })}
        onClick={() => void mutate(() => ipc.updateMemory(memory.memoryId, memory.revision, content, memory.enabled))}>保存编辑</ControlButton>
      {memory.status === "proposed" ? <>
        <ControlButton tone="primary" {...(disabledReason === undefined ? {} : { disabledReason })}
          onClick={() => void mutate(async () => { if (content !== memory.content) await ipc.updateMemory(memory.memoryId, memory.revision, content, false); await ipc.approveMemory(memory.memoryId, content === memory.content ? memory.revision : memory.revision + 1); })}>批准</ControlButton>
        <ControlButton {...(disabledReason === undefined ? {} : { disabledReason })}
          onClick={() => void mutate(() => ipc.rejectMemory(memory.memoryId, memory.revision))}>拒绝</ControlButton>
      </> : <>
        <ControlButton {...(disabledReason === undefined ? {} : { disabledReason })}
          onClick={() => void mutate(() => ipc.updateMemory(memory.memoryId, memory.revision, content, memory.status === "disabled"))}>{memory.status === "disabled" ? "重新启用" : "停用"}</ControlButton>
        <ControlButton {...(disabledReason === undefined ? {} : { disabledReason })}
          onClick={() => void mutate(() => ipc.deleteMemory(memory.memoryId, memory.revision))}>删除</ControlButton>
      </>}
    </div>
  </li>;
}

function splitPreferences(value: string): ReadonlyArray<string> {
  return value.split(/[，,]/u).map((item) => item.trim()).filter(Boolean).slice(0, 20);
}
function shortId(value: string): string { return `${value.slice(0, 8)}…`; }
function formatDate(value: string): string { return new Date(value).toLocaleString("zh-CN"); }
function trendDirection(value: "up" | "stable" | "down"): string {
  if (value === "up") return "上升";
  if (value === "down") return "下降";
  return "稳定";
}
function safeError(cause: unknown): string { return cause instanceof IpcInvocationError ? cause.message : "本地了解数据操作失败；未继续写入。"; }
