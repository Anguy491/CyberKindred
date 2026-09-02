import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";

import { App } from "../../src/App";
import { FOUNDATION_CAPABILITIES } from "../../src/design/foundation";
import { OnboardingFlow } from "../../src/onboarding/OnboardingFlow";
import type { OnboardingClient } from "../../src/onboarding/state";
import type { OnboardingState, OnboardingStepSubmission, SettingsView } from "../../src/ipc";

const REQUEST_ID = "00000000-0000-4000-8000-000000000001";
const ROOT_ID = "00000000-0000-4000-8000-000000000002";

describe("TASK-010 onboarding", () => {
  // FR-ONB-001..007; NFR-PRIV-001; NFR-COST-001.
  it("completes all seven local-only steps without provider or audio calls", async () => {
    const user = userEvent.setup();
    const backend = createBackend(emptyState());
    const completed = vi.fn();
    render(<Harness backend={backend} onCompleted={completed} />);

    await user.click(screen.getByRole("button", { name: "开始设置" }));
    await user.click(screen.getByRole("checkbox", { name: /本地音乐/u }));
    await user.click(screen.getByRole("button", { name: "选择本地目录" }));
    await waitFor(() => expect(screen.getByText("已授权 1 个目录")).not.toBeNull());
    expect(screen.getByRole("list", { name: "已授权目录" }).textContent).toContain("Music [待扫描]");
    await user.click(screen.getByRole("button", { name: "继续" }));
    await user.click(screen.getByRole("button", { name: "稍后，仅本地" }));
    expect(await screen.findByText(/本地模式使用文字旁白/u)).not.toBeNull();
    await user.click(screen.getByRole("button", { name: "继续使用文字" }));
    await user.type(screen.getByRole("textbox", { name: /称呼/u }), "小岚");
    await user.click(screen.getByRole("button", { name: "继续" }));
    await user.click(screen.getByRole("button", { name: "暂不设置并继续" }));

    const finish = screen.getByRole("button", { name: "完成设置" });
    expect(finish).toHaveProperty("disabled", true);
    await user.click(screen.getByRole("checkbox", { name: /声音只会/u }));
    expect(finish).toHaveProperty("disabled", true);
    await user.click(screen.getByRole("checkbox", { name: /原始对话/u }));
    expect(finish).toHaveProperty("disabled", false);
    await user.click(finish);

    await waitFor(() => expect(completed).toHaveBeenCalledTimes(1));
    expect(backend.calls.map((call) => call.kind)).toEqual([
      "save", "get", "roots", "pick", "save", "get", "settings", "settings", "save", "get",
      "settings", "save", "get", "save", "get", "save", "get", "save", "get",
    ]);
    expect(backend.calls.some((call) => ["secret", "voices", "preview", "update"]
      .includes(call.kind))).toBe(false);
  });

  // FR-ONB-007; NFR-REL-001; NFR-A11Y-001.
  it("restores the first incomplete step and permits keyboard back editing", async () => {
    const user = userEvent.setup();
    const backend = createBackend({
      ...emptyState(), completedSteps: ["welcome", "music_source", "openai_key"],
      sourceSelection: ["apple_music"], aiMode: "local_only", revision: 3,
    });
    render(<Harness backend={backend} onCompleted={vi.fn()} />);

    expect(screen.getByRole("heading", { name: "声音" })).not.toBeNull();
    await user.keyboard("{Escape}");
    expect(screen.getByRole("heading", { name: "AI" })).not.toBeNull();
    await user.click(screen.getByRole("button", { name: "返回" }));
    expect(screen.getByRole("heading", { name: "音乐来源" })).not.toBeNull();
    expect(screen.getByRole("checkbox", { name: /Apple Music/u })).toHaveProperty("checked", true);
  });

  // FR-ONB-007; NFR-REL-001; NFR-PRIV-001.
  it("App treats API-002 as authoritative and does not load the shell while incomplete", async () => {
    const capabilities = vi.fn();
    const backend = createBackend({
      ...emptyState(), completedSteps: ["welcome", "music_source"],
      sourceSelection: ["apple_music"], revision: 2,
    });
    render(<App onboardingLoader={backend.client.getOnboardingState}
      onboardingClient={backend.client} capabilityLoader={capabilities} />);

    expect(await screen.findByRole("heading", { name: "AI" })).not.toBeNull();
    expect(screen.queryByRole("heading", { name: "今天想听什么状态？" })).toBeNull();
    expect(capabilities).not.toHaveBeenCalled();
  });

  // FR-ONB-003/007; NFR-SEC-001; NFR-COST-001.
  it("recovers configured credential status from API-007 without restoring or probing the secret", async () => {
    const backend = createBackend({
      ...emptyState(), completedSteps: ["welcome", "music_source"],
      sourceSelection: ["apple_music"], revision: 2,
    }, true);
    render(<Harness backend={backend} onCompleted={vi.fn()} />);

    expect(await screen.findByText("[VALID]")).not.toBeNull();
    expect(screen.getByTestId("onboarding-openai-key")).toHaveProperty("value", "");
    expect(screen.getByTestId("onboarding-next-openai")).toHaveProperty("disabled", false);
    expect(backend.calls.map((call) => call.kind)).toEqual(["settings"]);
  });

  // FR-ONB-003/004; FR-ONB-005; NFR-COST-001.
  it("keeps Key validation and voice preview behind their explicit click controls", async () => {
    const user = userEvent.setup();
    const keyBackend = createBackend({
      ...emptyState(), completedSteps: ["welcome", "music_source"],
      sourceSelection: ["apple_music"], revision: 2,
    });
    const keyView = render(<Harness backend={keyBackend} onCompleted={vi.fn()} />);
    const keyInput = await screen.findByTestId("onboarding-openai-key");
    await user.type(keyInput, "test-only-key");
    expect(keyBackend.calls.some((call) => call.kind === "secret")).toBe(false);
    await user.click(screen.getByTestId("onboarding-validate-key"));
    expect(keyBackend.calls.some((call) => call.kind === "secret")).toBe(true);
    keyView.unmount();

    const voiceBackend = createBackend({
      ...emptyState(), completedSteps: ["welcome", "music_source", "openai_key"],
      sourceSelection: ["apple_music"], aiMode: "verified", revision: 3,
    }, false, true);
    render(<Harness backend={voiceBackend} onCompleted={vi.fn()} />);
    await user.click(await screen.findByRole("radio", { name: /Alloy/u }));
    expect(voiceBackend.calls.some((call) => call.kind === "preview")).toBe(false);
    await user.click(screen.getByTestId("onboarding-preview-voice"));
    expect(voiceBackend.calls.some((call) => call.kind === "preview")).toBe(true);
  });

  // NFR-SEC-001; NFR-PRIV-001.
  it("fails closed for every hold-to-reveal release path", async () => {
    const backend = createBackend({
      ...emptyState(), completedSteps: ["welcome", "music_source"],
      sourceSelection: ["apple_music"], revision: 2,
    });
    render(<Harness backend={backend} onCompleted={vi.fn()} />);
    const input = await screen.findByTestId("onboarding-openai-key");
    const reveal = screen.getByRole("button", { name: "按住显示" });
    fireEvent.change(input, { target: { value: "test-only-key" } });

    fireEvent.pointerDown(reveal);
    expect(input).toHaveProperty("type", "text");
    fireEvent.pointerLeave(reveal);
    expect(input).toHaveProperty("type", "password");

    fireEvent.pointerDown(reveal);
    fireEvent.pointerUp(window);
    expect(input).toHaveProperty("type", "password");
    fireEvent.pointerDown(reveal);
    fireEvent.pointerCancel(window);
    expect(input).toHaveProperty("type", "password");
    fireEvent.pointerDown(reveal);
    fireEvent.blur(window);
    expect(input).toHaveProperty("type", "password");
    fireEvent.pointerDown(reveal);
    fireEvent(document, new Event("visibilitychange"));
    expect(input).toHaveProperty("type", "password");
    fireEvent.keyDown(reveal, { key: "Enter" });
    expect(input).toHaveProperty("type", "text");
    fireEvent.keyUp(window, { key: "Enter" });
    expect(input).toHaveProperty("type", "password");
  });

  // NFR-SEC-001; FR-ONB-003/007.
  it("clears an unsubmitted Key on Back, Escape, local-only, forward, and remount", async () => {
    const user = userEvent.setup();
    const backend = createBackend({
      ...emptyState(), completedSteps: ["welcome", "music_source"],
      sourceSelection: ["apple_music"], revision: 2,
    });
    const view = render(<Harness backend={backend} onCompleted={vi.fn()} />);
    let input = await screen.findByTestId("onboarding-openai-key");
    await user.type(input, "back-secret");
    await user.click(screen.getByTestId("onboarding-back"));
    await user.click(screen.getByTestId("onboarding-next-source"));
    input = await screen.findByTestId("onboarding-openai-key");
    expect(input).toHaveProperty("value", "");

    await user.type(input, "escape-secret");
    await user.keyboard("{Escape}");
    await user.click(screen.getByTestId("onboarding-next-source"));
    input = await screen.findByTestId("onboarding-openai-key");
    expect(input).toHaveProperty("value", "");

    await user.type(input, "local-secret");
    await user.click(screen.getByTestId("onboarding-local-only"));
    await user.click(screen.getByTestId("onboarding-back"));
    input = await screen.findByTestId("onboarding-openai-key");
    expect(input).toHaveProperty("value", "");

    await user.type(input, "forward-secret");
    await user.click(screen.getByTestId("onboarding-validate-key"));
    await user.click(screen.getByTestId("onboarding-next-openai"));
    await user.click(screen.getByTestId("onboarding-back"));
    expect(await screen.findByTestId("onboarding-openai-key")).toHaveProperty("value", "");

    await user.type(screen.getByTestId("onboarding-openai-key"), "unmount-secret");
    view.unmount();
    const remountBackend = createBackend({
      ...emptyState(), completedSteps: ["welcome", "music_source"],
      sourceSelection: ["apple_music"], revision: 2,
    });
    render(<Harness backend={remountBackend} onCompleted={vi.fn()} />);
    expect(await screen.findByTestId("onboarding-openai-key")).toHaveProperty("value", "");
  });

  // NFR-SEC-001; NFR-REL-001.
  it("does not let delayed configured recovery override a newly typed Key", async () => {
    let resolveSettings: ((settings: SettingsView) => void) | undefined;
    const pendingSettings = new Promise<SettingsView>((resolve) => { resolveSettings = resolve; });
    const backend = createBackend({
      ...emptyState(), completedSteps: ["welcome", "music_source"],
      sourceSelection: ["apple_music"], revision: 2,
    });
    backend.client.getSettings = vi.fn(() => pendingSettings);
    const user = userEvent.setup();
    render(<Harness backend={backend} onCompleted={vi.fn()} />);
    const input = screen.getByTestId("onboarding-openai-key");
    await user.type(input, "new-key");
    resolveSettings?.(settingsView(true));
    await waitFor(() => expect(backend.client.getSettings).toHaveBeenCalledTimes(1));
    expect(input).toHaveProperty("value", "new-key");
    expect(screen.queryByText("[VALID]")).toBeNull();
    expect(screen.getByTestId("onboarding-next-openai")).toHaveProperty("disabled", true);
  });

  // FR-ONB-003/004; NFR-COST-001; NFR-PRIV-001.
  it("persists TTS disabled before local-only and text-only transitions without provider or audio calls", async () => {
    const user = userEvent.setup();
    const localBackend = createBackend({
      ...emptyState(),
      completedSteps: ["welcome", "music_source", "openai_key", "voice"],
      sourceSelection: ["apple_music"], aiMode: "verified", voiceMode: "selected", revision: 4,
    }, true, true, true);
    const localView = render(<Harness backend={localBackend} onCompleted={vi.fn()} />);
    await user.keyboard("{Escape}");
    await screen.findByRole("heading", { name: "声音" });
    await user.keyboard("{Escape}");
    await screen.findByRole("heading", { name: "AI" });
    await user.click(screen.getByTestId("onboarding-local-only"));
    const localKinds = localBackend.calls.map((call) => call.kind);
    expect(localKinds.lastIndexOf("settings")).toBeLessThan(localKinds.indexOf("update"));
    expect(localKinds.indexOf("update")).toBeLessThan(localKinds.lastIndexOf("save"));
    expect(localBackend.calls.find((call) => call.kind === "update")?.patch)
      .toEqual({ ttsEnabled: false });
    expect(localKinds.some((kind) => ["secret", "preview"].includes(kind))).toBe(false);
    localView.unmount();

    const textBackend = createBackend({
      ...emptyState(), completedSteps: ["welcome", "music_source", "openai_key"],
      sourceSelection: ["apple_music"], aiMode: "verified", revision: 3,
    }, true, true, true);
    render(<Harness backend={textBackend} onCompleted={vi.fn()} />);
    await user.click(await screen.findByTestId("onboarding-voice-text-only"));
    const textKinds = textBackend.calls.map((call) => call.kind);
    expect(textKinds.lastIndexOf("settings")).toBeLessThan(textKinds.indexOf("update"));
    expect(textKinds.indexOf("update")).toBeLessThan(textKinds.lastIndexOf("save"));
    expect(textBackend.calls.find((call) => call.kind === "update")?.patch)
      .toEqual({ ttsEnabled: false });
    expect(textKinds.some((kind) => ["secret", "preview"].includes(kind))).toBe(false);
  });

  // NFR-REL-001; NFR-PRIV-001.
  it("does not advance onboarding when the required TTS disable write fails", async () => {
    const user = userEvent.setup();
    const backend = createBackend({
      ...emptyState(), completedSteps: ["welcome", "music_source", "openai_key"],
      sourceSelection: ["apple_music"], aiMode: "verified", revision: 3,
    }, true, true, true);
    backend.client.updateSettings = vi.fn(async (request) => {
      backend.calls.push({ kind: "update", patch: request.patch });
      throw new Error("fixture failure");
    });
    render(<Harness backend={backend} onCompleted={vi.fn()} />);

    await user.click(await screen.findByTestId("onboarding-voice-text-only"));
    expect(await screen.findByRole("alert")).not.toBeNull();
    expect(backend.calls.some((call) => call.kind === "save")).toBe(false);
    expect(backend.calls.some((call) => ["secret", "preview"].includes(call.kind))).toBe(false);
  });

  // FR-ONB-004; NFR-COST-001.
  it("disables unavailable voice previews and exposes a safe status", async () => {
    const user = userEvent.setup();
    const backend = createBackend({
      ...emptyState(), completedSteps: ["welcome", "music_source", "openai_key"],
      sourceSelection: ["apple_music"], aiMode: "verified", revision: 3,
    }, false, true, false, false);
    render(<Harness backend={backend} onCompleted={vi.fn()} />);
    await user.click(await screen.findByRole("radio", { name: /Alloy/u }));
    const preview = screen.getByTestId("onboarding-preview-voice");
    expect(preview).toHaveProperty("disabled", true);
    expect(screen.getByText("[PREVIEW UNAVAILABLE]")).not.toBeNull();
    await user.click(preview);
    expect(backend.calls.some((call) => call.kind === "preview")).toBe(false);
  });

  // FR-ONB-007; FR-RAD-001; NFR-PRIV-001; NFR-A11Y-001.
  it("enters a silent RADIO and focuses Start only after authoritative completion", async () => {
    const user = userEvent.setup();
    const backend = createBackend({
      ...emptyState(),
      completedSteps: ["welcome", "music_source", "openai_key", "voice", "profile", "city_schedule"],
      sourceSelection: ["apple_music"], aiMode: "local_only", voiceMode: "text_only",
      cityScheduleMode: "not_now", revision: 6,
    });
    const capabilities = vi.fn(async () => FOUNDATION_CAPABILITIES);
    render(<App onboardingLoader={backend.client.getOnboardingState}
      onboardingClient={backend.client} capabilityLoader={capabilities}
      fontStatusLoader={async () => "loaded"} />);

    await user.click(await screen.findByRole("checkbox", { name: /声音只会/u }));
    await user.click(screen.getByRole("checkbox", { name: /原始对话/u }));
    await user.click(screen.getByRole("button", { name: "完成设置" }));
    const start = await screen.findByRole("button", { name: "开始节目" });
    await waitFor(() => expect(document.activeElement).toBe(start));
    expect(screen.getByText("保持静音，等待用户明确开始。")).not.toBeNull();
    expect(backend.calls.some((call) => ["secret", "voices", "preview", "settings", "update"]
      .includes(call.kind))).toBe(false);
  });
});

function Harness({ backend, onCompleted }: {
  readonly backend: ReturnType<typeof createBackend>;
  readonly onCompleted: (state: OnboardingState) => void;
}) {
  const [state, setState] = useState(backend.state());
  return <OnboardingFlow state={state} client={backend.client}
    onStateChange={setState} onCompleted={onCompleted} />;
}

function createBackend(
  initial: OnboardingState,
  configured = false,
  voiceCatalog = false,
  ttsEnabled = false,
  previewAvailable = true,
) {
  let current = initial;
  let roots: Array<{ rootId: string; displayName: string; available: boolean }> = [];
  let currentTtsEnabled = ttsEnabled;
  let settingsRevision = 0;
  const calls: Array<{ kind: string; patch?: unknown }> = [];
  const client = {
    getOnboardingState: vi.fn(async () => { calls.push({ kind: "get" }); return current; }),
    saveOnboardingStep: vi.fn(async (request) => {
      calls.push({ kind: "save" });
      current = applySubmission(current, request.submission);
      return { requestId: request.clientRequestId, revision: current.revision };
    }),
    listLibraryRoots: vi.fn(async () => { calls.push({ kind: "roots" }); return { roots, revision: roots.length }; }),
    pickAndAddLibraryRoot: vi.fn(async () => {
      calls.push({ kind: "pick" });
      roots = [{ rootId: ROOT_ID, displayName: "Music", available: true }];
      return { requestId: REQUEST_ID, root: roots[0]!, revision: 1 };
    }),
    validateAndSetSecret: vi.fn(async () => {
      calls.push({ kind: "secret" });
      return { requestId: REQUEST_ID, configured: true, verifiedAt: "2026-09-03T00:00:00Z" };
    }),
    listVoices: vi.fn(async () => {
      calls.push({ kind: "voices" });
      return { voices: voiceCatalog
        ? [{ voiceId: "alloy", displayName: "Alloy", previewAvailable }]
        : [] };
    }),
    previewVoice: vi.fn(async () => { calls.push({ kind: "preview" }); throw new Error("not expected"); }),
    getSettings: vi.fn(async () => {
      calls.push({ kind: "settings" });
      return settingsView(configured, currentTtsEnabled, settingsRevision);
    }),
    updateSettings: vi.fn(async (request) => {
      calls.push({ kind: "update", patch: request.patch });
      if (request.patch.ttsEnabled !== undefined) currentTtsEnabled = request.patch.ttsEnabled;
      settingsRevision += 1;
      return { requestId: request.clientRequestId, revision: settingsRevision };
    }),
  } as unknown as OnboardingClient;
  return { client, calls, state: () => current };
}

function settingsView(configured: boolean, ttsEnabled = false, revision = 0): SettingsView {
  return {
    providerOrigin: "https://api.openai.com", llmModelId: "gpt-4.1-mini", ttsModelId: "gpt-4o-mini-tts",
    ttsVoiceId: "alloy", metadataEnabled: false, weatherEnabled: false, defaultSourceId: null,
    narrationDensity: "balanced" as const, ttsEnabled, audioOutputDeviceId: null,
    audioOutputBehavior: "follow_system_default" as const, minimizeToTray: false, launchAtStartup: false,
    notificationsEnabled: false, weatherLocation: null,
    secretStatus: { origins: configured ? [{ origin: "https://api.openai.com",
      openaiApiKeyConfigured: true, lastVerifiedAt: "2026-09-03T00:00:00Z" }] : [] },
    integrationStatuses: [], revision,
  };
}

function emptyState(): OnboardingState {
  return {
    completed: false, completedSteps: [], sourceSelection: [], aiMode: null, voiceMode: null,
    cityScheduleMode: null,
    profile: { displayName: "", companionStyle: "quiet_warm", initialPreferences: [], narrationDensity: "balanced" },
    privacyConfirmations: { explicitSound: false, rawConversationRetention: false }, revision: 0,
  };
}

function applySubmission(state: OnboardingState, submission: OnboardingStepSubmission): OnboardingState {
  const order = ["welcome", "music_source", "openai_key", "voice", "profile", "city_schedule", "privacy"] as const;
  const completedSteps = state.completedSteps.includes(submission.step)
    ? state.completedSteps : order.slice(0, state.completedSteps.length + 1);
  const base = { ...state, completedSteps, revision: state.revision + 1 };
  if (submission.step === "music_source") return { ...base, sourceSelection: submission.sources };
  if (submission.step === "openai_key") return { ...base, aiMode: submission.mode };
  if (submission.step === "voice") return { ...base, voiceMode: submission.mode };
  if (submission.step === "profile") return { ...base, profile: submission.profile };
  if (submission.step === "city_schedule") return { ...base, cityScheduleMode: submission.mode };
  if (submission.step === "privacy") return {
    ...base, completed: true, privacyConfirmations: submission.confirmations,
  };
  return base;
}
