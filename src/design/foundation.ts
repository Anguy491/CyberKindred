import { CyberKindredIpcClient, type AppCapabilities } from "../ipc";

export type RouteId = "radio" | "library" | "you" | "settings";
export type FoundationState =
  | "initializing"
  | "ready"
  | "empty"
  | "offline"
  | "degraded"
  | "error"
  | "permission-denied";
export type FontStatus = "checking" | "loaded" | "fallback";
export type CapabilityLoader = () => Promise<AppCapabilities>;
export type FontStatusLoader = () => Promise<Exclude<FontStatus, "checking">>;

export const ROUTES: ReadonlyArray<{
  readonly id: RouteId;
  readonly label: string;
  readonly accessibleName: string;
  readonly shortcut: string;
}> = [
  { id: "radio", label: "RADIO", accessibleName: "电台", shortcut: "Ctrl+1" },
  { id: "library", label: "LIBRARY", accessibleName: "曲库", shortcut: "Ctrl+2" },
  { id: "you", label: "YOU", accessibleName: "了解", shortcut: "Ctrl+3" },
  { id: "settings", label: "SETTINGS", accessibleName: "设置", shortcut: "Ctrl+4" },
];

export const FOUNDATION_CAPABILITIES: AppCapabilities = {
  protocolVersion: "1.0.0",
  appVersion: "0.1.0",
  platform: "windows",
  osVersion: "Windows",
  features: {
    localLibrary: true,
    systemMediaSession: false,
    musicKit: false,
    appleMusicDomControl: false,
    externalHttpApi: false,
  },
  sources: [],
  providers: [],
};

/** Uses API-001 only inside Tauri; browser/jsdom stays hermetic and local. */
export async function loadFoundationCapabilities(): Promise<AppCapabilities> {
  if (typeof window === "undefined" || !Reflect.has(window, "__TAURI_INTERNALS__")) {
    return FOUNDATION_CAPABILITIES;
  }
  return new CyberKindredIpcClient().getCapabilities();
}

/** Reports fallback explicitly when the browser cannot prove local fonts loaded. */
export async function detectLocalFontStatus(): Promise<"loaded" | "fallback"> {
  if (typeof document === "undefined" || document.fonts === undefined) {
    return "fallback";
  }
  await document.fonts.ready;
  const loaded = [
    document.fonts.check('16px "Space Grotesk"'),
    document.fonts.check('12px "Space Mono"'),
    document.fonts.check('48px "Doto"'),
  ].every(Boolean);
  return loaded ? "loaded" : "fallback";
}

export function isTextEditingTarget(target: EventTarget | null): boolean {
  return target instanceof HTMLElement
    && (target.isContentEditable
      || target.tagName === "INPUT"
      || target.tagName === "TEXTAREA"
      || target.tagName === "SELECT");
}
