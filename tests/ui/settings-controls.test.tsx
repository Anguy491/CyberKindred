import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { FOUNDATION_CAPABILITIES } from "../../src/design/foundation";
import { BROWSER_SETTINGS_IPC, type SettingsIpc } from "../../src/features/settings";
import type { PlaybackState, SettingsView } from "../../src/ipc";
import { SettingsPage } from "../../src/routes/SettingsPage";

const SETTINGS: SettingsView = {
  providerOrigin: "https://api.openai.com",
  llmModelId: "gpt-5.6-luna",
  ttsModelId: "gpt-4o-mini-tts",
  ttsVoiceId: "alloy",
  metadataEnabled: true,
  weatherEnabled: true,
  defaultSourceId: "local",
  narrationDensity: "balanced",
  ttsEnabled: false,
  audioOutputDeviceId: null,
  audioOutputBehavior: "follow_system_default",
  minimizeToTray: false,
  launchAtStartup: false,
  notificationsEnabled: false,
  weatherLocation: null,
  secretStatus: {
    origins: [{
      origin: "https://api.openai.com",
      openaiApiKeyConfigured: true,
      lastVerifiedAt: "2026-09-05T00:00:00.000Z",
    }],
  },
  integrationStatuses: [
    { integration: "openai", state: "connected", lastSuccessAt: "2026-09-05T00:00:00.000Z", safeMessage: "OpenAI 连接可用。" },
    { integration: "apple_music", state: "connected", lastSuccessAt: "2026-09-05T00:01:00.000Z", safeMessage: "Apple Music Windows App 会话已连接。" },
    { integration: "musicbrainz", state: "degraded", lastSuccessAt: null, safeMessage: "服务或网络当前不可用。" },
    { integration: "weather", state: "disabled", lastSuccessAt: null, safeMessage: "此集成已关闭。" },
  ],
  revision: 4,
};

function fixture(): SettingsIpc {
  const appleState: PlaybackState = {
    schemaVersion: "1.0.0",
    sourceId: "apple_music",
    sourceKind: "system_session",
    status: "paused",
    capabilities: {
      play: true, pause: true, seek: false, next: true, previous: false, setQueue: false,
    },
    currentTrack: null,
    positionMs: 0,
    durationMs: null,
    revision: 1,
    updatedAt: "2026-09-05T00:01:00.000Z",
    lastError: null,
  };
  return {
    ...BROWSER_SETTINGS_IPC,
    getSettings: vi.fn(async () => SETTINGS),
    listVoices: vi.fn(async () => ({
      voices: [
        { voiceId: "alloy", displayName: "Alloy", previewAvailable: true },
        { voiceId: "coral", displayName: "Coral", previewAvailable: true },
      ],
    })),
    listMusicSources: vi.fn(async () => ({
      sources: [
        {
          sourceId: "local", kind: "local", displayName: "本地曲库", connected: true,
          capabilities: { play: true, pause: true, seek: true, next: true, previous: true, setQueue: true },
        },
        {
          sourceId: "apple_music", kind: "system_session", displayName: "Apple Music Windows App",
          connected: true,
          capabilities: { play: true, pause: true, seek: false, next: true, previous: false, setQueue: false },
        },
      ],
    })),
    selectMusicSource: vi.fn(async (request) => ({
      requestId: request.clientRequestId, state: appleState,
    })),
    validateAndSetSecret: vi.fn(async (request) => ({
      requestId: request.clientRequestId, configured: true, verifiedAt: "2026-09-05T00:00:00.000Z",
    })),
    deleteSecret: vi.fn(async (request) => ({ requestId: request.clientRequestId, configured: false })),
    updateSettings: vi.fn(async (request) => ({
      requestId: request.clientRequestId, revision: request.expectedRevision + 1,
    })),
  };
}

function renderSettings(ipc: SettingsIpc) {
  render(<SettingsPage
    capabilities={{
      ...FOUNDATION_CAPABILITIES,
      features: { ...FOUNDATION_CAPABILITIES.features, systemMediaSession: true },
    }}
    fontStatus="loaded"
    shellState="ready"
    ipc={ipc}
  />);
}

describe("[TASK-028] settings controls", () => {
  it("never reveals a configured secret and requires an explicit advanced-change review", async () => {
    const ipc = fixture();
    const user = userEvent.setup();
    renderSettings(ipc);

    expect(await screen.findByText("•••••••• [CONFIGURED]")).not.toBeNull();
    expect(screen.queryByText(/sk-[a-z0-9]/iu)).toBeNull();
    expect(screen.getAllByText("[CONNECTED]")).toHaveLength(2);
    expect(screen.getAllByText(/最近成功：尚无记录/u)).toHaveLength(2);

    await user.type(screen.getByLabelText("替换密钥"), "sk-test-only");
    await user.click(screen.getByRole("button", { name: "验证并替换" }));
    await waitFor(() => expect(ipc.validateAndSetSecret).toHaveBeenCalledWith(expect.objectContaining({
      kind: "openai_api_key", origin: "https://api.openai.com", value: "sk-test-only",
    })));

    await user.click(screen.getByText("高级 Provider 设置"));
    await user.clear(screen.getByLabelText("LLM model ID"));
    await user.type(screen.getByLabelText("LLM model ID"), "gpt-6-astra");
    await user.selectOptions(screen.getByLabelText("声音"), "coral");
    await user.click(screen.getByRole("button", { name: "检查变更" }));
    expect(screen.getByText("LLM: gpt-5.6-luna → gpt-6-astra")).not.toBeNull();
    expect(screen.getByText("VOICE: alloy → coral")).not.toBeNull();
    expect(ipc.updateSettings).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "保存已列出的变更" }));
    await waitFor(() => expect(ipc.updateSettings).toHaveBeenCalledWith(expect.objectContaining({
      expectedRevision: 4,
      patch: { llmModelId: "gpt-6-astra", ttsVoiceId: "coral" },
    }), "gpt-5.6-luna"));
  });

  it("saves playback preferences without starting audio", async () => {
    const ipc = fixture();
    const user = userEvent.setup();
    renderSettings(ipc);
    await user.click(screen.getByRole("button", { name: "PLAYBACK" }));

    await user.selectOptions(await screen.findByLabelText("默认音乐源"), "apple_music");
    await user.selectOptions(screen.getByLabelText("串场密度"), "quiet");
    await user.click(screen.getByRole("checkbox", { name: "启用 TTS" }));
    await user.click(screen.getByRole("button", { name: "保存播放设置" }));
    await waitFor(() => expect(ipc.updateSettings).toHaveBeenCalledWith(expect.objectContaining({
      expectedRevision: 4,
      patch: { defaultSourceId: "apple_music", narrationDensity: "quiet", ttsEnabled: true },
    }), "gpt-5.6-luna"));
    expect(screen.getByText("[SAVED]")).not.toBeNull();
  });

  it("applies each Windows behavior setting as a separate silent change", async () => {
    const ipc = fixture();
    const user = userEvent.setup();
    renderSettings(ipc);
    await user.click(screen.getByRole("button", { name: "APP" }));

    await user.click(await screen.findByRole("button", { name: "启用登录启动" }));
    await waitFor(() => expect(ipc.updateSettings).toHaveBeenNthCalledWith(1, expect.objectContaining({
      expectedRevision: 4, patch: { launchAtStartup: true },
    }), "gpt-5.6-luna"));
    expect(screen.getByText("[ENABLED · NO SOUND]")).not.toBeNull();

    await user.click(screen.getByRole("button", { name: "启用托盘运行" }));
    await waitFor(() => expect(ipc.updateSettings).toHaveBeenNthCalledWith(2, expect.objectContaining({
      expectedRevision: 5, patch: { minimizeToTray: true },
    }), "gpt-5.6-luna"));
  });

  it("shows only live GSMTC capabilities and the no-web-control boundary", async () => {
    const ipc = fixture();
    const user = userEvent.setup();
    renderSettings(ipc);
    await user.click(screen.getByRole("button", { name: "APPLE MUSIC" }));

    expect(await screen.findByText("PLAY · PAUSE · NEXT")).not.toBeNull();
    expect(screen.getByText("[CONNECTED]")).not.toBeNull();
    expect(screen.getByText(/不登录 MusicKit、不读取账户，也不控制网页/u)).not.toBeNull();
    expect(screen.getByText(/不建立精确 Apple 队列/u)).not.toBeNull();
    expect(ipc.selectMusicSource).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "连接 / 重新检查会话" }));
    await waitFor(() => expect(ipc.selectMusicSource).toHaveBeenCalledWith(expect.objectContaining({
      sourceId: "apple_music",
    })));
  });
});
