import { afterEach, describe, expect, it, vi } from "vitest";

import { CyberKindredIpcClient, IpcInvocationError, type IpcTransport } from "../../src/ipc";

const REQUEST_ID = "00000000-0000-4000-8000-000000000001";

class Transport implements IpcTransport {
  readonly calls: Array<{ command: string; args: Readonly<Record<string, unknown>> }> = [];
  response: unknown;
  pending = false;
  async invoke<Response>(command: string, args: Readonly<Record<string, unknown>>): Promise<Response> {
    this.calls.push({ command, args });
    if (this.pending) return new Promise<Response>(() => undefined);
    return this.response as Response;
  }
  async listen(): Promise<() => void> { return () => undefined; }
}

afterEach(() => vi.useRealTimers());

describe("TASK-010 onboarding IPC", () => {
  // API-002/API-003; FR-ONB-007; NFR-MAINT-003.
  it("uses nested request envelopes and strict onboarding DTO validation", async () => {
    const transport = new Transport();
    const client = new CyberKindredIpcClient(transport);
    transport.response = {
      completed: false, completedSteps: [], sourceSelection: [], aiMode: null, voiceMode: null,
      cityScheduleMode: null,
      profile: { displayName: "", companionStyle: "quiet_warm", initialPreferences: [], narrationDensity: "balanced" },
      privacyConfirmations: { explicitSound: false, rawConversationRetention: false }, revision: 0,
    };
    await client.getOnboardingState();
    expect(transport.calls[0]).toEqual({ command: "api_v1_get_onboarding_state", args: { request: {} } });

    transport.response = { requestId: REQUEST_ID, revision: 1 };
    await client.saveOnboardingStep({
      clientRequestId: REQUEST_ID, expectedRevision: 0, submission: { step: "welcome" },
    });
    expect(transport.calls[1]).toEqual({
      command: "api_v1_save_onboarding_step",
      args: { request: { clientRequestId: REQUEST_ID, expectedRevision: 0, submission: { step: "welcome" } } },
    });

    transport.response = { completed: true, completedSteps: [], sourceSelection: [], aiMode: null,
      voiceMode: null, cityScheduleMode: null,
      profile: { displayName: "", companionStyle: "quiet_warm", initialPreferences: [], narrationDensity: "balanced" },
      privacyConfirmations: { explicitSound: false, rawConversationRetention: false }, revision: 0 };
    await expect(client.getOnboardingState()).rejects.toBeInstanceOf(IpcInvocationError);
  });

  // API-002; FR-ONB-006; NFR-MAINT-003.
  it("counts onboarding profile bounds as Unicode code points and permits slash text", async () => {
    const transport = new Transport();
    const client = new CyberKindredIpcClient(transport);
    const emojiName = "🌙".repeat(80);
    transport.response = {
      completed: false, completedSteps: [], sourceSelection: [], aiMode: null, voiceMode: null,
      cityScheduleMode: null,
      profile: { displayName: emojiName, companionStyle: "quiet_warm",
        initialPreferences: ["AC/DC", "C:\\Music", "夜🌙"], narrationDensity: "balanced" },
      privacyConfirmations: { explicitSound: false, rawConversationRetention: false }, revision: 0,
    };
    await expect(client.getOnboardingState()).resolves.toMatchObject({ profile: { displayName: emojiName } });

    transport.response = { ...transport.response as object,
      profile: { displayName: "🌙".repeat(81), companionStyle: "quiet_warm",
        initialPreferences: [], narrationDensity: "balanced" } };
    await expect(client.getOnboardingState()).rejects.toBeInstanceOf(IpcInvocationError);
  });

  // API-008; NFR-PERF-002; NFR-COST-001.
  it("uses 60 seconds only for an actual model change and five seconds otherwise", async () => {
    vi.useFakeTimers();
    const transport = new Transport();
    transport.pending = true;
    const client = new CyberKindredIpcClient(transport);
    const base = { clientRequestId: REQUEST_ID, expectedRevision: 2 };

    const unchanged = client.updateSettings({ ...base, patch: { llmModelId: "gpt-current" } }, "gpt-current");
    const unchangedAssertion = expect(unchanged).rejects.toBeInstanceOf(IpcInvocationError);
    await vi.advanceTimersByTimeAsync(4_999);
    await vi.advanceTimersByTimeAsync(1);
    await unchangedAssertion;

    const changed = client.updateSettings({ ...base, patch: { llmModelId: "gpt-next" } }, "gpt-current");
    let settled = false;
    void changed.catch(() => { settled = true; });
    await vi.advanceTimersByTimeAsync(59_999);
    expect(settled).toBe(false);
    const changedAssertion = expect(changed).rejects.toBeInstanceOf(IpcInvocationError);
    await vi.advanceTimersByTimeAsync(1);
    await changedAssertion;
  });

  // API-007; NFR-PERF-002.
  it("uses the exact two-second caller deadline for settings reads", async () => {
    vi.useFakeTimers();
    const transport = new Transport();
    transport.pending = true;
    const client = new CyberKindredIpcClient(transport);
    const request = client.getSettings();
    let settled = false;
    void request.then(() => { settled = true; }, () => { settled = true; });
    const assertion = expect(request).rejects.toBeInstanceOf(IpcInvocationError);

    await vi.advanceTimersByTimeAsync(1_999);
    expect(settled).toBe(false);
    await vi.advanceTimersByTimeAsync(1);
    await assertion;
    expect(transport.calls).toEqual([
      { command: "api_v1_get_settings", args: { request: {} } },
    ]);
  });

  // API-004/API-009/API-011; FR-ONB-002/003/004; NFR-SEC-002.
  it("serializes sensitive and user-controlled actions only inside request", async () => {
    const transport = new Transport();
    const client = new CyberKindredIpcClient(transport);
    transport.response = { requestId: REQUEST_ID, configured: true, verifiedAt: "2026-09-03T00:00:00Z" };
    await client.validateAndSetSecret({ clientRequestId: REQUEST_ID, kind: "openai_api_key",
      origin: "https://api.openai.com", value: "test-only-secret" });
    expect(transport.calls.at(-1)?.args).toEqual({ request: { clientRequestId: REQUEST_ID,
      kind: "openai_api_key", origin: "https://api.openai.com", value: "test-only-secret" } });

    transport.response = { operationId: REQUEST_ID, acceptedAt: "2026-09-03T00:00:00Z" };
    await client.previewVoice(REQUEST_ID, "alloy");
    expect(transport.calls.at(-1)).toEqual({ command: "api_v1_preview_voice",
      args: { request: { clientRequestId: REQUEST_ID, voiceId: "alloy" } } });

    transport.response = { requestId: REQUEST_ID, root: null, revision: 0 };
    await client.pickAndAddLibraryRoot(REQUEST_ID);
    expect(transport.calls.at(-1)).toEqual({ command: "api_v1_pick_and_add_library_root",
      args: { request: { clientRequestId: REQUEST_ID } } });
  });
});
