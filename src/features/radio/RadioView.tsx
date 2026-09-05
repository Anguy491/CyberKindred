import { type RefObject, useCallback, useEffect, useRef, useState } from "react";

import { ControlButton } from "../../components/ControlButton";
import { IpcInvocationError, type SourceSummary } from "../../ipc";
import type { PlaybackState, ProgramPlan, RadioEvent, RadioIpc } from "./types";
import "./radio.css";

type LoadState = "loading" | "ready" | "error";
type ProgramState = "idle" | "planning" | "running" | "paused" | "stopping" | "completed" | "failed";
interface ChatLine {
  readonly key: string;
  readonly operationId: string;
  readonly role: "user" | "assistant" | "status";
  readonly text: string;
}

export interface RadioViewProps {
  readonly ipc: RadioIpc;
  readonly disabledReason?: string;
  readonly startButtonRef?: RefObject<HTMLButtonElement | null>;
  readonly autoFocusStart?: boolean;
  readonly onStartFocused?: () => void;
}

export function RadioView({
  ipc, disabledReason, startButtonRef, autoFocusStart = false, onStartFocused,
}: RadioViewProps) {
  const [loadState, setLoadState] = useState<LoadState>("loading");
  const [sources, setSources] = useState<ReadonlyArray<SourceSummary>>([]);
  const [playback, setPlayback] = useState<PlaybackState | null>(null);
  const [programId, setProgramId] = useState<string | null>(null);
  const [plan, setPlan] = useState<ProgramPlan | null>(null);
  const [programState, setProgramState] = useState<ProgramState>("idle");
  const [segmentStates, setSegmentStates] = useState<Readonly<Record<string, string>>>({});
  const [messages, setMessages] = useState<ReadonlyArray<string>>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  const [chatLines, setChatLines] = useState<ReadonlyArray<ChatLine>>([]);
  const [chatOperation, setChatOperation] = useState<string | null>(null);
  const [feedbackStatus, setFeedbackStatus] = useState<string | null>(null);
  const [subscriptionAttempt, setSubscriptionAttempt] = useState(0);
  const programIdRef = useRef<string | null>(null);
  const terminalChatOperationsRef = useRef(new Set<string>());

  const refreshSnapshot = useCallback(async () => {
    try {
      const [catalog, state] = await Promise.all([
        ipc.listMusicSources(), ipc.getPlaybackState(),
      ]);
      setSources(catalog.sources);
      setPlayback(state);
      setLoadState("ready");
    } catch (cause) {
      setError(safeError(cause));
      setLoadState("error");
      throw cause;
    }
  }, [ipc]);

  const receive = useCallback((event: RadioEvent) => {
    if (event.type === "playback") {
      if (event.payload.state !== null) {
        const state = event.payload.state;
        setSources((current) => current.map((source) => source.sourceId === state.sourceId ? {
          ...source,
          connected: state.status !== "disconnected",
          capabilities: state.capabilities,
        } : source));
        setPlayback((current) => current === null || current.sourceId === state.sourceId
          ? state
          : current);
      }
      return;
    }
    if (event.type === "program-state") {
      if (programIdRef.current !== null && programIdRef.current !== event.payload.programId) return;
      programIdRef.current = event.payload.programId;
      setProgramId(event.payload.programId);
      setProgramState(event.payload.state);
      const message = event.payload.safeMessage;
      if (message !== null) {
        setMessages((current) => current.includes(message)
          ? current
          : [...current, message]);
      }
      return;
    }
    if (event.type === "chat-message") {
      if (programIdRef.current !== event.payload.programId) return;
      setChatLines((current) => appendChat(current, {
        key: `${event.payload.operationId}:${event.payload.role}`,
        operationId: event.payload.operationId,
        role: event.payload.role,
        text: event.payload.text,
      }));
      if (event.payload.role === "user") setChatOperation(event.payload.operationId);
      if (event.payload.final) {
        rememberTerminal(terminalChatOperationsRef.current, event.payload.operationId);
        setChatOperation((current) => current === event.payload.operationId ? null : current);
      }
      return;
    }
    if (event.type === "operation-cancelled") {
      if (event.payload.kind !== "chat") return;
      rememberTerminal(terminalChatOperationsRef.current, event.payload.operationId);
      setChatOperation((current) => current === event.payload.operationId ? null : current);
      setChatLines((current) => current.some((line) => line.operationId === event.payload.operationId)
        ? appendChat(current, {
          key: `${event.payload.operationId}:cancelled`,
          operationId: event.payload.operationId,
          role: "status",
          text: "这次文字请求已取消；不会产生 AI 回复或记忆提案。",
        })
        : current);
      return;
    }
    if (programIdRef.current !== null && programIdRef.current !== event.payload.programId) return;
    setSegmentStates((current) => ({
      ...current, [event.payload.segmentId]: event.payload.state,
    }));
  }, []);

  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;
    setLoadState("loading");
    void ipc.subscribeRadio(
      (event) => { if (active) receive(event); },
      async () => { if (active) await refreshSnapshot(); },
    ).then(
      (next) => { if (active) unlisten = next; else next(); },
      (cause) => {
        if (active) {
          setError(safeError(cause));
          setLoadState("error");
        }
      },
    );
    return () => { active = false; unlisten?.(); };
  }, [ipc, receive, refreshSnapshot, subscriptionAttempt]);

  useEffect(() => {
    if (loadState === "ready" && autoFocusStart && startButtonRef?.current !== null
      && startButtonRef?.current !== undefined) {
      startButtonRef.current.focus();
      onStartFocused?.();
    }
  }, [autoFocusStart, loadState, onStartFocused, startButtonRef]);

  const localSource = sources.find((source) => source.sourceId === "local");
  const unavailable = disabledReason
    ?? (localSource === undefined ? "[UNAVAILABLE: LOCAL SOURCE]"
      : !localSource.connected ? "[UNAVAILABLE: LOCAL SOURCE DISCONNECTED]"
        : !localSource.capabilities.setQueue ? "[UNAVAILABLE: QUEUE CAPABILITY]" : undefined);
  const active = programState === "planning" || programState === "running"
    || programState === "paused" || programState === "stopping";
  const currentVoice = currentVoiceText(plan, segmentStates);

  const run = async (label: string, action: () => Promise<PlaybackState>) => {
    if (busy !== null) return;
    setBusy(label); setError(null);
    try { setPlayback(await action()); } catch (cause) { setError(safeError(cause)); }
    finally { setBusy(null); }
  };

  const start = async () => {
    if (busy !== null || unavailable !== undefined || active) return;
    programIdRef.current = null;
    setProgramId(null); setBusy("start"); setError(null); setMessages([]); setSegmentStates({});
    setChatLines([]); setChatOperation(null); setFeedbackStatus(null);
    terminalChatOperationsRef.current.clear();
    setProgramState("planning");
    try {
      const response = await ipc.startProgram("local");
      programIdRef.current = response.programId;
      setProgramId(response.programId);
      setPlan(response.plan);
    } catch (cause) { setError(safeError(cause)); setProgramState("idle"); }
    finally { setBusy(null); }
  };

  const stop = async () => {
    if (busy !== null || programId === null || !active) return;
    setBusy("stop"); setError(null); setProgramState("stopping");
    try { await ipc.stopProgram(programId); } catch (cause) { setError(safeError(cause)); }
    finally { setBusy(null); }
  };

  const sendChat = async () => {
    const text = draft.trim();
    if (programId === null || !active || chatOperation !== null || text.length === 0) return;
    setError(null);
    try {
      const accepted = await ipc.submitChat(programId, text);
      setDraft("");
      if (terminalChatOperationsRef.current.delete(accepted.operationId)) {
        setChatOperation(null);
      } else {
        setChatOperation(accepted.operationId);
      }
    } catch (cause) { setError(safeError(cause)); }
  };

  const cancelChat = async () => {
    if (chatOperation === null) return;
    try { await ipc.cancelChat(chatOperation); } catch (cause) { setError(safeError(cause)); }
  };

  const feedback = async (kind: "like" | "skip" | "less_talk") => {
    if (programId === null || !active) return;
    const trackId = kind === "less_talk" ? null : playback?.currentTrack?.trackId ?? null;
    if (kind !== "less_talk" && trackId === null) return;
    setFeedbackStatus(null); setError(null);
    try {
      await ipc.submitFeedback(programId, trackId, kind);
      setFeedbackStatus(kind === "less_talk" ? "已应用：本节目余下时间每 4–6 首至多一次串场。" : kind === "like" ? "已保存喜欢反馈；后续节目选择会参考它。" : "已保存跳过反馈；后续节目选择会降低相似候选。" );
    } catch (cause) { setError(safeError(cause)); }
  };

  if (loadState === "loading") {
    return <section className="radio-state" aria-labelledby="radio-title">
      <p className="instrument-label">RADIO / LOADING</p>
      <h1 id="radio-title" className="hero-title">正在连接本地播放核心</h1>
      <p role="status" className="inline-status">[LOADING…] 尚未播放或调用服务。</p>
    </section>;
  }
  if (loadState === "error") {
    return <section className="radio-state" aria-labelledby="radio-title">
      <p className="instrument-label">RADIO / ERROR</p>
      <h1 id="radio-title" className="hero-title">电台状态不可用</h1>
      <p role="alert" className="secondary-copy">{error ?? "无法读取本地播放状态；保持静音。"}</p>
      <ControlButton onClick={() => setSubscriptionAttempt((attempt) => attempt + 1)}>重试</ControlButton>
    </section>;
  }

  const revision = playback?.revision ?? 0;
  const controlsDisabled = playback === null || busy !== null;
  const startDisabled = busy !== null || active ? "[PROGRAM ACTIVE]" : unavailable;
  return <section className="page radio-page" aria-labelledby="radio-title">
    <div className="page-primary radio-now" aria-live="polite">
      <p className="instrument-label">RADIO / {programState.toUpperCase()}</p>
      <h1 id="radio-title" className="hero-title">
        {playback?.currentTrack?.title ?? "今天想听什么状态？"}
      </h1>
      <p className="secondary-copy">
        {playback?.currentTrack === null || playback === null
          ? "点击开始后才会规划、调用可用服务并播放。"
          : `${playback.currentTrack.artist ?? "未知艺术家"} · ${playback.currentTrack.album ?? "未知专辑"}`}
      </p>
      <dl className="radio-readout">
        <div><dt>来源</dt><dd>{playback?.sourceId ?? "local"}</dd></div>
        <div><dt>状态</dt><dd>{playback?.status ?? "disconnected"}</dd></div>
        <div><dt>进度</dt><dd>{formatTime(playback?.positionMs ?? 0)} / {formatTime(playback?.durationMs ?? 0)}</dd></div>
        <div><dt>REV</dt><dd>{revision}</dd></div>
      </dl>
      {currentVoice !== null ? <blockquote className="radio-voice">
        <span className="instrument-label">ON AIR / TEXT</span>{currentVoice}
      </blockquote> : null}
    </div>

    <div className="page-secondary radio-workbench">
      <fieldset className="source-selector">
        <legend className="instrument-label">音乐来源</legend>
        {sources.length === 0 ? <p className="secondary-copy">没有可用来源。</p> : sources.map((source) =>
          <ControlButton key={source.sourceId} aria-pressed={playback?.sourceId === source.sourceId}
            {...disabledWhen(!source.connected, "[DISCONNECTED]")}
            onClick={() => void run("source", async () => (await ipc.selectMusicSource(source.sourceId)).state)}>
            {source.displayName}
          </ControlButton>)}
      </fieldset>

      <div className="primary-action-row">
        <ControlButton {...(startButtonRef === undefined ? {} : { ref: startButtonRef })} tone="primary"
          {...(startDisabled === undefined ? {} : { disabledReason: startDisabled })}
          data-testid="radio-start" onClick={() => void start()}>开始节目</ControlButton>
        <ControlButton {...disabledWhen(!active || busy !== null, "[NO ACTIVE PROGRAM]")}
          onClick={() => void stop()}>停止节目</ControlButton>
      </div>
      <p className="inline-status">保持静音，等待用户明确开始。</p>

      <div className="playback-controls" aria-label="播放控制">
        <ControlButton {...disabledWhen(controlsDisabled || !playback?.capabilities.previous, "[UNAVAILABLE]")}
          onClick={() => void run("previous", () => ipc.previous(revision))}>上一首</ControlButton>
        {playback?.status === "playing" ?
          <ControlButton {...disabledWhen(controlsDisabled || !playback.capabilities.pause, "[UNAVAILABLE]")}
            onClick={() => void run("pause", () => ipc.pause(revision))}>暂停</ControlButton>
          : <ControlButton {...disabledWhen(controlsDisabled || !playback?.capabilities.play, "[UNAVAILABLE]")}
            onClick={() => void run("play", () => ipc.play(revision))}>播放</ControlButton>}
        <ControlButton {...disabledWhen(controlsDisabled || !playback?.capabilities.next, "[UNAVAILABLE]")}
          onClick={() => void run("next", () => ipc.next(revision))}>下一首</ControlButton>
        <label className="progress-control">
          <span className="instrument-label">进度 / {formatTime(playback?.positionMs ?? 0)}</span>
          <input type="range" min="0" max={Math.max(1, playback?.durationMs ?? 1)}
            value={playback?.positionMs ?? 0} disabled={controlsDisabled || !playback?.capabilities.seek}
            aria-label="播放进度" onChange={(event) => void run("seek", () =>
              ipc.seek(revision, Number(event.currentTarget.value)))} />
        </label>
      </div>

      <div className="feedback-controls" aria-label="节目反馈">
        <ControlButton {...disabledWhen(!active || playback === null || playback.currentTrack === null, "[NO CURRENT TRACK]")}
          onClick={() => void feedback("like")}>喜欢</ControlButton>
        <ControlButton {...disabledWhen(!active || playback === null || playback.currentTrack === null, "[NO CURRENT TRACK]")}
          onClick={() => void feedback("skip")}>跳过反馈</ControlButton>
        <ControlButton {...disabledWhen(!active, "[NO ACTIVE PROGRAM]")}
          onClick={() => void feedback("less_talk")}>少说一点</ControlButton>
      </div>
      {feedbackStatus !== null ? <p role="status" className="inline-status">{feedbackStatus}</p> : null}

      {error !== null ? <p role="alert" className="inline-status radio-error">{error}</p> : null}
      {messages.map((message) => <p key={message} role="status" className="radio-degradation">
        {message.includes("语音") ? "[TTS UNAVAILABLE — TEXT CONTINUES] " : "[DETERMINISTIC LOCAL QUEUE] "}{message}
      </p>)}

      <PlanView plan={plan} segmentStates={segmentStates} />

      <label className="text-entry" htmlFor="radio-message">
        <span className="instrument-label">告诉 CyberKindred 你现在想听什么</span>
        <textarea id="radio-message" rows={2} value={draft}
          onChange={(event) => setDraft(event.currentTarget.value)}
          maxLength={4_000}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey) {
              event.preventDefault(); void sendChat();
            }
          }}
          placeholder="输入文字请求。Enter 发送，Shift+Enter 换行。" />
      </label>
      <div className="primary-action-row">
        <ControlButton tone="primary"
          {...disabledWhen(!active || programId === null || draft.trim().length === 0 || chatOperation !== null,
            chatOperation !== null ? "[REQUEST IN PROGRESS]" : "[START A PROGRAM FIRST]")}
          onClick={() => void sendChat()}>发送</ControlButton>
        <ControlButton {...disabledWhen(chatOperation === null, "[NO ACTIVE REQUEST]")}
          onClick={() => void cancelChat()}>取消请求</ControlButton>
      </div>
      <ol className="chat-transcript" aria-label="文字对话" aria-live="polite">
        {chatLines.map((line) => <li key={line.key} data-role={line.role}>
          <strong>{line.role === "user" ? "你" : line.role === "assistant" ? "CyberKindred" : "状态"}</strong>
          <span>{line.text}</span>
        </li>)}
      </ol>
    </div>

    <aside className="page-tertiary" aria-label="电台状态">
      <dl className="instrument-list">
        <div><dt>PROGRAM</dt><dd>{programId === null ? "尚未开始" : programState}</dd></div>
        <div><dt>PLAN</dt><dd>{plan === null ? "等待明确开始" : `${trackSegments(plan).length} 首`}</dd></div>
        <div><dt>SPEECH</dt><dd>{currentVoice ?? "文字串场待命"}</dd></div>
        <div><dt>NETWORK</dt><dd>仅在明确开始后按设置尝试</dd></div>
      </dl>
    </aside>
  </section>;
}

function appendChat(current: ReadonlyArray<ChatLine>, next: ChatLine): ReadonlyArray<ChatLine> {
  if (current.some((line) => line.key === next.key)) return current;
  return [...current, next].slice(-20);
}

function rememberTerminal(operations: Set<string>, operationId: string): void {
  operations.add(operationId);
  if (operations.size > 64) {
    const oldest = operations.values().next().value;
    if (oldest !== undefined) operations.delete(oldest);
  }
}

function PlanView({ plan, segmentStates }: {
  readonly plan: ProgramPlan | null;
  readonly segmentStates: Readonly<Record<string, string>>;
}) {
  if (plan === null) return <p className="secondary-copy">节目计划会在点击开始后显示。</p>;
  return <section className="radio-plan" aria-labelledby="radio-plan-title">
    <h2 id="radio-plan-title" className="instrument-label">节目计划 / {trackSegments(plan).length} 首</h2>
    <ol>{plan.segments.map((segment, index) => <li key={segment.segmentId}>
      <span>{segment.type === "track" ? `曲目 ${String(trackOrdinal(plan, index)).padStart(2, "0")}` : "文字串场"}</span>
      <small>{segment.type === "track" ? shortId(segment.trackId) : segment.text}</small>
      <em>{segmentStates[segment.segmentId] ?? "queued"}</em>
    </li>)}</ol>
  </section>;
}

function trackSegments(plan: ProgramPlan) {
  return plan.segments.filter((segment): segment is Extract<ProgramPlan["segments"][number], { type: "track" }> =>
    segment.type === "track");
}

function trackOrdinal(plan: ProgramPlan, segmentIndex: number): number {
  return plan.segments.slice(0, segmentIndex + 1).filter((segment) => segment.type === "track").length;
}

function currentVoiceText(plan: ProgramPlan | null, states: Readonly<Record<string, string>>): string | null {
  if (plan === null) return null;
  const active = plan.segments.find((segment) => segment.type === "voice" && states[segment.segmentId] === "playing");
  return active?.type === "voice" ? active.text : null;
}

function shortId(value: string): string {
  return `ID ${value.slice(0, 8)}…${value.slice(-4)}`;
}

function formatTime(milliseconds: number): string {
  const seconds = Math.max(0, Math.floor(milliseconds / 1_000));
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

function safeError(cause: unknown): string {
  return cause instanceof IpcInvocationError && cause.message.length > 0
    ? cause.message
    : "本地电台操作失败；已保持安全状态。";
}

function disabledWhen(condition: boolean, message: string): { readonly disabledReason?: string } {
  return condition ? { disabledReason: message } : {};
}
