import {
  CyberKindredIpcClient,
  type OnboardingState,
} from "../ipc";

export type OnboardingClient = Pick<CyberKindredIpcClient,
  | "getOnboardingState"
  | "saveOnboardingStep"
  | "validateAndSetSecret"
  | "getSettings"
  | "updateSettings"
  | "previewVoice"
  | "listLibraryRoots"
  | "pickAndAddLibraryRoot"
  | "listVoices"
>;

export type OnboardingLoader = () => Promise<OnboardingState>;

/** Browser tests use a complete, silent fixture; only the desktop runtime invokes API-002. */
export const BROWSER_ONBOARDING_COMPLETE: OnboardingState = {
  completed: true,
  completedSteps: [
    "welcome", "music_source", "openai_key", "voice", "profile", "city_schedule", "privacy",
  ],
  sourceSelection: ["apple_music"],
  aiMode: "local_only",
  voiceMode: "text_only",
  cityScheduleMode: "not_now",
  profile: {
    displayName: "",
    companionStyle: "quiet_warm",
    initialPreferences: [],
    narrationDensity: "balanced",
  },
  privacyConfirmations: {
    explicitSound: true,
    rawConversationRetention: true,
  },
  revision: 7,
};

export function hasTauriRuntime(): boolean {
  return typeof window !== "undefined" && Reflect.has(window, "__TAURI_INTERNALS__");
}

export async function loadOnboardingState(): Promise<OnboardingState> {
  if (!hasTauriRuntime()) return BROWSER_ONBOARDING_COMPLETE;
  return new CyberKindredIpcClient().getOnboardingState();
}

export function createOnboardingClient(): OnboardingClient {
  return new CyberKindredIpcClient();
}

export function createClientRequestId(): string {
  return globalThis.crypto.randomUUID();
}
