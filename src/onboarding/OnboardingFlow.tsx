import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { ControlButton } from "../components/ControlButton";
import { IpcInvocationError, type LibraryRoot, type MusicSourceKind, type OnboardingState,
  type OnboardingStepSubmission, type SettingsView, type VoiceView } from "../ipc";
import { createClientRequestId, type OnboardingClient } from "./state";

const STEPS = [
  "welcome", "music_source", "openai_key", "voice", "profile", "city_schedule", "privacy",
] as const;
const STEP_LABELS = ["欢迎", "音乐来源", "AI", "声音", "偏好", "城市与日程", "隐私"] as const;

interface OnboardingFlowProps {
  readonly state: OnboardingState;
  readonly client: OnboardingClient;
  readonly onStateChange: (state: OnboardingState) => void;
  readonly onCompleted: (state: OnboardingState) => void;
}

export function OnboardingFlow({ state, client, onStateChange, onCompleted }: OnboardingFlowProps) {
  const [stepIndex, setStepIndex] = useState(() => Math.min(state.completedSteps.length, STEPS.length - 1));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [sources, setSources] = useState<MusicSourceKind[]>([...state.sourceSelection]);
  const [roots, setRoots] = useState<ReadonlyArray<LibraryRoot>>([]);
  const [keyValue, setKeyValue] = useState("");
  const [keyVisible, setKeyVisible] = useState(false);
  const [keyStatus, setKeyStatus] = useState<string | null>(null);
  const [voices, setVoices] = useState<ReadonlyArray<VoiceView>>([]);
  const [voiceId, setVoiceId] = useState("");
  const [voiceStatus, setVoiceStatus] = useState<string | null>(null);
  const [settings, setSettings] = useState<SettingsView | null>(null);
  const [displayName, setDisplayName] = useState(state.profile.displayName);
  const [preferences, setPreferences] = useState(state.profile.initialPreferences.join(", "));
  const [density, setDensity] = useState(state.profile.narrationDensity);
  const [explicitSound, setExplicitSound] = useState(state.privacyConfirmations.explicitSound);
  const [retention, setRetention] = useState(state.privacyConfirmations.rawConversationRetention);
  const titleRef = useRef<HTMLHeadingElement>(null);
  const keyValueRef = useRef("");
  const keyEditGenerationRef = useRef(0);
  const step = STEPS[stepIndex];

  const hideKey = useCallback(() => setKeyVisible(false), []);
  const clearKeyEntry = useCallback(() => {
    keyValueRef.current = "";
    keyEditGenerationRef.current += 1;
    setKeyValue("");
    setKeyVisible(false);
  }, []);

  useEffect(() => titleRef.current?.focus(), [stepIndex]);

  useEffect(() => {
    if (step !== "openai_key") return;
    let active = true;
    const recoveryGeneration = keyEditGenerationRef.current;
    void client.getSettings().then(
      (result) => {
        if (!active) return;
        setSettings(result);
        const configured = result.secretStatus.origins.some((secret) =>
          secret.origin === result.providerOrigin && secret.openaiApiKeyConfigured);
        if (configured && keyEditGenerationRef.current === recoveryGeneration
          && keyValueRef.current === "") setKeyStatus("[VALID]");
      },
      (reason: unknown) => { if (active) setError(safeError(reason)); },
    );
    return () => { active = false; };
  }, [client, step]);

  useEffect(() => {
    if (step !== "music_source" || !sources.includes("local")) return;
    let active = true;
    void client.listLibraryRoots().then(
      (result) => { if (active) setRoots(result.roots); },
      (reason: unknown) => { if (active) setError(safeError(reason)); },
    );
    return () => { active = false; };
  }, [client, sources, step]);

  useEffect(() => {
    if (step !== "voice" || state.aiMode !== "verified") return;
    let active = true;
    void Promise.all([client.listVoices(), client.getSettings()]).then(
      ([voiceResult, settingsResult]) => {
        if (!active) return;
        setVoices(voiceResult.voices);
        setSettings(settingsResult);
        setVoiceId(settingsResult.ttsVoiceId);
      },
      (reason: unknown) => { if (active) setError(safeError(reason)); },
    );
    return () => { active = false; };
  }, [client, state.aiMode, step]);

  useEffect(() => {
    const hideOnVisibilityChange = () => hideKey();
    window.addEventListener("pointerup", hideKey);
    window.addEventListener("pointercancel", hideKey);
    window.addEventListener("blur", hideKey);
    window.addEventListener("keyup", hideKey);
    document.addEventListener("visibilitychange", hideOnVisibilityChange);
    return () => {
      window.removeEventListener("pointerup", hideKey);
      window.removeEventListener("pointercancel", hideKey);
      window.removeEventListener("blur", hideKey);
      window.removeEventListener("keyup", hideKey);
      document.removeEventListener("visibilitychange", hideOnVisibilityChange);
    };
  }, [hideKey]);

  useEffect(() => () => clearKeyEntry(), [clearKeyEntry]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && stepIndex > 0 && !busy) {
        if (step === "openai_key") clearKeyEntry();
        setStepIndex((current) => current - 1);
      }
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [busy, clearKeyEntry, step, stepIndex]);

  async function persist(submission: OnboardingStepSubmission, disableTts = false): Promise<void> {
    setBusy(true);
    setError(null);
    if (submission.step === "openai_key") clearKeyEntry();
    try {
      if (disableTts) await disableTtsIfEnabled();
      await client.saveOnboardingStep({
        clientRequestId: createClientRequestId(), expectedRevision: state.revision, submission,
      });
      const authoritative = await client.getOnboardingState();
      onStateChange(authoritative);
      if (authoritative.completed) {
        onCompleted(authoritative);
      } else {
        setStepIndex((current) => Math.min(current + 1, STEPS.length - 1));
      }
    } catch (reason) {
      setError(safeError(reason));
    } finally {
      setBusy(false);
    }
  }

  async function disableTtsIfEnabled(): Promise<void> {
    const current = await client.getSettings();
    setSettings(current);
    if (!current.ttsEnabled) return;
    const result = await client.updateSettings({
      clientRequestId: createClientRequestId(), expectedRevision: current.revision,
      patch: { ttsEnabled: false },
    }, current.llmModelId);
    setSettings({ ...current, ttsEnabled: false, revision: result.revision });
  }

  async function chooseRoot(): Promise<void> {
    setBusy(true);
    setError(null);
    try {
      const result = await client.pickAndAddLibraryRoot(createClientRequestId());
      if (result.root !== null) setRoots((current) => uniqueRoots([...current, result.root!]));
    } catch (reason) {
      setError(safeError(reason));
    } finally {
      setBusy(false);
    }
  }

  async function validateKey(): Promise<void> {
    if (keyValue.trim() === "") return;
    setBusy(true);
    setError(null);
    setKeyStatus("[CHECKING…]");
    try {
      await client.validateAndSetSecret({
        clientRequestId: createClientRequestId(), kind: "openai_api_key",
        origin: "https://api.openai.com", value: keyValue,
      });
      clearKeyEntry();
      setKeyStatus("[VALID]");
    } catch (reason) {
      setKeyStatus(keyFailureLabel(reason));
    } finally {
      setBusy(false);
    }
  }

  async function previewSelectedVoice(): Promise<void> {
    const selectedVoice = voices.find((voice) => voice.voiceId === voiceId);
    if (selectedVoice?.previewAvailable !== true) return;
    setBusy(true);
    setError(null);
    try {
      await client.previewVoice(createClientRequestId(), voiceId);
      setVoiceStatus("[PREVIEW REQUESTED]");
    } catch (reason) {
      setError(safeError(reason));
    } finally {
      setBusy(false);
    }
  }

  async function saveSelectedVoice(): Promise<void> {
    if (settings === null || voiceId === "") return;
    setBusy(true);
    setError(null);
    try {
      await client.updateSettings({
        clientRequestId: createClientRequestId(), expectedRevision: settings.revision,
        patch: { ttsVoiceId: voiceId, ttsEnabled: true },
      }, settings.llmModelId);
      await persist({ step: "voice", mode: "selected" });
    } catch (reason) {
      setError(safeError(reason));
      setBusy(false);
    }
  }

  const localNeedsRoot = sources.includes("local") && roots.length === 0;
  const selectedVoice = voices.find((voice) => voice.voiceId === voiceId);
  const previewUnavailable = selectedVoice !== undefined && !selectedVoice.previewAvailable;
  const preferenceList = useMemo(() => preferences.split(/[,\n]/u).map((item) => item.trim())
    .filter(Boolean).slice(0, 20).map((item) => limitCodePoints(item, 100)), [preferences]);

  return (
    <main className="onboarding" id="main-content" data-testid="onboarding-flow">
      <aside className="onboarding-progress" aria-label="引导进度">
        <p className="brand-name">CYBERKINDRED</p>
        <ol>{STEP_LABELS.map((label, index) => (
          <li key={label} aria-current={index === stepIndex ? "step" : undefined}>
            <span>{String(index + 1).padStart(2, "0")}</span> {label}
          </li>
        ))}</ol>
      </aside>
      <section className="onboarding-stage" aria-labelledby="onboarding-title"
        data-testid={`onboarding-step-${step}`}>
        <p className="instrument-label">SETUP / {stepIndex + 1} OF 7</p>
        <h1 id="onboarding-title" className="hero-title" ref={titleRef} tabIndex={-1}>
          {STEP_LABELS[stepIndex]}
        </h1>

        <div className="onboarding-form">{renderStep()}</div>
        {error === null ? null : <p className="inline-error" role="alert"><strong>[ERROR]</strong>{error}</p>}
        {busy ? <p className="inline-status" role="status">[LOADING…]</p> : null}
        <div className="onboarding-actions">
        <ControlButton tone="ghost" disabled={stepIndex === 0 || busy} data-testid="onboarding-back"
            onClick={() => {
              if (step === "openai_key") clearKeyEntry();
              setStepIndex((current) => Math.max(0, current - 1));
            }}>返回</ControlButton>
          {renderPrimaryAction()}
        </div>
      </section>
      <aside className="onboarding-context" aria-label="当前步骤说明">
        <p className="instrument-label">BOUNDARY</p>
        <p>默认保持静音。只有你点击播放或试听，CyberKindred 才会发出声音。</p>
        <p>API Key 只交给本地 Rust 后端和系统凭据库，不保存在网页界面。</p>
      </aside>
    </main>
  );

  function renderStep() {
    if (step === "welcome") return <>
      <p className="secondary-copy">这是一台 Windows 优先的 AI 陪伴电台。你决定音乐来源、AI 是否联网，以及何时出声。</p>
      <p className="onboarding-note">设置可稍后修改；当前流程不会自动播放、扫描或调用付费服务。</p>
    </>;
    if (step === "music_source") return <fieldset className="choice-stack">
      <legend>选择一个或多个音乐来源</legend>
      <Choice id="onboarding-source-apple" checked={sources.includes("apple_music")} label="Apple Music / Windows App"
        detail="仅使用 Windows 系统媒体会话；不操作网页 DOM，也不保证精确点歌。"
        onChange={(checked) => setSources(toggleSource(sources, "apple_music", checked))} />
      <Choice id="onboarding-source-local" checked={sources.includes("local")} label="本地音乐"
        detail="只读取你通过系统选择器明确授权的目录。"
        onChange={(checked) => setSources(toggleSource(sources, "local", checked))} />
      {sources.includes("local") ? <div className="root-picker">
        <ControlButton onClick={() => void chooseRoot()} disabled={busy}
          data-testid="onboarding-pick-root">选择本地目录</ControlButton>
        <span className="inline-status">已授权 {roots.length} 个目录</span>
        <ul className="authorized-root-list" aria-label="已授权目录">
          {roots.map((root) => <li key={root.rootId}>{root.displayName} <span>[待扫描]</span></li>)}
        </ul>
      </div> : null}
    </fieldset>;
    if (step === "openai_key") return <>
      <p className="secondary-copy">可验证自己的 OpenAI API Key，或暂时只使用确定性的本地队列与文字。</p>
      <label className="text-entry">OpenAI API Key
        <input id="onboarding-openai-key" data-testid="onboarding-openai-key"
          type={keyVisible ? "text" : "password"} value={keyValue} autoComplete="off"
          onChange={(event) => {
            keyValueRef.current = event.target.value;
            keyEditGenerationRef.current += 1;
            setKeyValue(event.target.value);
            setKeyStatus(null);
          }} />
      </label>
      <div className="inline-action-row">
        <ControlButton disabled={keyValue.trim() === "" || busy} onClick={() => void validateKey()}
          data-testid="onboarding-validate-key">验证 Key</ControlButton>
        <button className="text-control" type="button" aria-pressed={keyVisible}
          onPointerDown={() => setKeyVisible(true)} onPointerUp={hideKey}
          onPointerCancel={hideKey} onPointerLeave={hideKey} onBlur={hideKey} onKeyDown={(event) => {
            if (event.key === " " || event.key === "Enter") setKeyVisible(true);
          }} onKeyUp={hideKey}>按住显示</button>
        {keyStatus === null ? null : <span className="inline-status" role="status">{keyStatus}</span>}
      </div>
    </>;
    if (step === "voice") return state.aiMode === "local_only" ? <>
      <p className="secondary-copy">本地模式使用文字旁白，不会加载声音目录，也不会发起试听或网络音频调用。</p>
      <p className="onboarding-note">TEXT ONLY / LOCAL FALLBACK</p>
    </> : <fieldset className="choice-stack"><legend>选择旁白声音</legend>
      {voices.map((voice) => <label className="choice-row" key={voice.voiceId}>
        <input type="radio" name="voice" checked={voiceId === voice.voiceId}
          onChange={() => {
            setVoiceId(voice.voiceId);
            setVoiceStatus(voice.previewAvailable ? null : "[PREVIEW UNAVAILABLE]");
          }} />
        <span><strong>{voice.displayName}</strong><small>{voice.voiceId}</small></span>
      </label>)}
      <div className="inline-action-row">
        <ControlButton disabled={voiceId === "" || busy || previewUnavailable}
          onClick={() => void previewSelectedVoice()}
          data-testid="onboarding-preview-voice">试听所选声音</ControlButton>
        {previewUnavailable || voiceStatus !== null
          ? <span className="inline-status" role="status">
              {previewUnavailable ? "[PREVIEW UNAVAILABLE]" : voiceStatus}
            </span>
          : null}
      </div>
      <ControlButton tone="ghost" disabled={busy} data-testid="onboarding-voice-text-only"
        onClick={() => void persist({ step: "voice", mode: "text_only" }, true)}>仅文字</ControlButton>
    </fieldset>;
    if (step === "profile") return <>
      <label className="text-entry">称呼（可选）<input value={displayName}
        onChange={(event) => setDisplayName(limitCodePoints(event.target.value, 80))} /></label>
      <label className="text-entry">初始偏好（逗号或换行分隔）<textarea value={preferences}
        onChange={(event) => setPreferences(event.target.value)} /></label>
      <label className="select-entry">旁白密度<select value={density}
        onChange={(event) => setDensity(event.target.value as typeof density)}>
        <option value="quiet">安静</option><option value="balanced">平衡</option><option value="frequent">经常</option>
      </select></label>
      <p className="onboarding-note">陪伴风格固定为安静、温暖；不会诊断、操控或替代真人关系。</p>
    </>;
    if (step === "city_schedule") return <>
      <p className="secondary-copy">城市天气和日程可以稍后配置。日程只会通知；仍需你明确确认才开始播放。</p>
      <label className="choice-row"><input type="radio" checked readOnly /><span><strong>暂不设置</strong><small>NOT NOW</small></span></label>
      <label className="choice-row choice-row--disabled"><input type="radio" disabled /><span><strong>配置城市与日程</strong><small>[UNAVAILABLE IN THIS PROTOTYPE]</small></span></label>
    </>;
    return <>
      <div className="privacy-summary">
        <p><strong>音乐来源</strong> {sourceSummary(sources)}</p>
        <p><strong>外部服务</strong> {state.aiMode === "verified" ? "OpenAI（仅在明确操作时）" : "无；本地队列与文字"}</p>
        <p><strong>原始对话</strong> 最多保留 30 天，可随时清除。</p>
        <p><strong>声音与通知</strong> 默认静音；通知不会自动播放。</p>
      </div>
      <Choice id="onboarding-confirm-sound" checked={explicitSound} label="我理解声音只会在我明确确认后播放"
        detail="试听按钮同样视为一次明确确认。" onChange={setExplicitSound} />
      <Choice id="onboarding-confirm-retention" checked={retention} label="我理解原始对话保留与清除规则"
        detail="长期记忆可在设置中查看、修改和删除。" onChange={setRetention} />
    </>;
  }

  function renderPrimaryAction() {
    if (step === "welcome") return <ControlButton tone="primary" disabled={busy} data-testid="onboarding-next-welcome"
      onClick={() => void persist({ step: "welcome" })}>开始设置</ControlButton>;
    if (step === "music_source") return <ControlButton tone="primary" data-testid="onboarding-next-source"
      disabled={busy || sources.length === 0 || localNeedsRoot}
      onClick={() => void persist({ step: "music_source", sources })}>继续</ControlButton>;
    if (step === "openai_key") return <div className="inline-action-row">
      <ControlButton tone="ghost" disabled={busy} data-testid="onboarding-local-only"
        onClick={() => void persist({ step: "openai_key", mode: "local_only" }, true)}>稍后，仅本地</ControlButton>
      <ControlButton tone="primary" disabled={busy || keyStatus !== "[VALID]"} data-testid="onboarding-next-openai"
        onClick={() => void persist({ step: "openai_key", mode: "verified" })}>继续</ControlButton>
    </div>;
    if (step === "voice") return state.aiMode === "local_only"
      ? <ControlButton tone="primary" disabled={busy} data-testid="onboarding-next-text-only"
          onClick={() => void persist({ step: "voice", mode: "text_only" }, true)}>继续使用文字</ControlButton>
      : <ControlButton tone="primary" disabled={busy || voiceId === "" || settings === null}
          data-testid="onboarding-next-voice"
          onClick={() => void saveSelectedVoice()}>保存声音并继续</ControlButton>;
    if (step === "profile") return <ControlButton tone="primary" disabled={busy} data-testid="onboarding-next-profile"
      onClick={() => void persist({ step: "profile", profile: {
        displayName: displayName.trim(), companionStyle: "quiet_warm",
        initialPreferences: preferenceList, narrationDensity: density,
      } })}>继续</ControlButton>;
    if (step === "city_schedule") return <ControlButton tone="primary" disabled={busy} data-testid="onboarding-next-city"
      onClick={() => void persist({ step: "city_schedule", mode: "not_now" })}>暂不设置并继续</ControlButton>;
    return <ControlButton tone="primary" disabled={busy || !explicitSound || !retention}
      data-testid="onboarding-complete"
      onClick={() => void persist({ step: "privacy", confirmations: {
        explicitSound: true, rawConversationRetention: true,
      } })}>完成设置</ControlButton>;
  }
}

function Choice({ id, checked, label, detail, onChange }: {
  readonly id: string;
  readonly checked: boolean; readonly label: string; readonly detail: string;
  readonly onChange: (checked: boolean) => void;
}) {
  return <label className="choice-row"><input id={id} type="checkbox" checked={checked}
    onChange={(event) => onChange(event.target.checked)} /><span><strong>{label}</strong><small>{detail}</small></span></label>;
}

function toggleSource(current: MusicSourceKind[], source: MusicSourceKind, selected: boolean) {
  return selected ? [...new Set([...current, source])] : current.filter((item) => item !== source);
}

function uniqueRoots(roots: ReadonlyArray<LibraryRoot>) {
  return [...new Map(roots.map((root) => [root.rootId, root])).values()];
}

function limitCodePoints(value: string, limit: number): string {
  return Array.from(value).slice(0, limit).join("");
}

function sourceSummary(sources: ReadonlyArray<MusicSourceKind>): string {
  return sources.map((source) => source === "local" ? "本地目录" : "Apple Music 系统会话").join("、");
}

function safeError(reason: unknown): string {
  return reason instanceof IpcInvocationError ? reason.apiError.safeMessage : "本地操作失败，请重试。";
}

function keyFailureLabel(reason: unknown): string {
  if (!(reason instanceof IpcInvocationError)) return "[INVALID KEY]";
  if (reason.apiError.errorId === "ERR-1302") return "[RATE LIMITED]";
  if (["ERR-1303", "ERR-1304", "ERR-1601"].includes(reason.apiError.errorId)) return "[OFFLINE]";
  return "[INVALID KEY]";
}
