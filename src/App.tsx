import { useCallback, useEffect, useRef, useState } from "react";

import "./app.css";
import { AppHeader } from "./components/AppHeader";
import { ShellNotice } from "./components/ShellNotice";
import { ShellStatePanel } from "./components/ShellStatePanel";
import {
  detectLocalFontStatus,
  FOUNDATION_CAPABILITIES,
  isTextEditingTarget,
  loadFoundationCapabilities,
  type CapabilityLoader,
  type FontStatus,
  type FontStatusLoader,
  type FoundationState,
  type RouteId,
} from "./design/foundation";
import type { AppCapabilities } from "./ipc";
import { OnboardingFlow } from "./onboarding/OnboardingFlow";
import {
  BROWSER_ONBOARDING_COMPLETE,
  createOnboardingClient,
  hasTauriRuntime,
  loadOnboardingState,
  type OnboardingClient,
  type OnboardingLoader,
} from "./onboarding/state";
import type { OnboardingState } from "./ipc";
import { LibraryPage } from "./routes/LibraryPage";
import { RadioPage } from "./routes/RadioPage";
import { SettingsPage } from "./routes/SettingsPage";
import { YouPage } from "./routes/YouPage";

export interface AppProps {
  readonly capabilityLoader?: CapabilityLoader;
  readonly fontStatusLoader?: FontStatusLoader;
  readonly initialRoute?: RouteId;
  readonly onboardingClient?: OnboardingClient;
  readonly onboardingLoader?: OnboardingLoader;
  /** Deterministic visual scenario injection for hermetic UI tests. */
  readonly scenario?: FoundationState;
}

export function App({
  capabilityLoader = loadFoundationCapabilities,
  fontStatusLoader = detectLocalFontStatus,
  initialRoute = "radio",
  onboardingClient,
  onboardingLoader = loadOnboardingState,
  scenario,
}: AppProps) {
  const browserBypass = scenario !== undefined
    || (onboardingLoader === loadOnboardingState && !hasTauriRuntime());
  const [onboarding, setOnboarding] = useState<OnboardingState | "loading" | "error">(
    browserBypass ? BROWSER_ONBOARDING_COMPLETE : "loading",
  );
  const [onboardingAttempt, setOnboardingAttempt] = useState(0);
  const [focusAfterOnboarding, setFocusAfterOnboarding] = useState(false);
  const [activeRoute, setActiveRoute] = useState<RouteId>(initialRoute);
  const [shellState, setShellState] = useState<FoundationState>(scenario ?? "initializing");
  const [capabilities, setCapabilities] = useState<AppCapabilities>(FOUNDATION_CAPABILITIES);
  const [fontStatus, setFontStatus] = useState<FontStatus>("checking");
  const [loadAttempt, setLoadAttempt] = useState(0);
  const [radioDraft, setRadioDraft] = useState("");
  const [libraryQuery, setLibraryQuery] = useState("");
  const radioInputRef = useRef<HTMLTextAreaElement>(null);
  const librarySearchRef = useRef<HTMLInputElement>(null);
  const radioStartRef = useRef<HTMLButtonElement>(null);
  const onboardingIpc = useMemoOnboardingClient(onboardingClient);

  useEffect(() => {
    if (scenario !== undefined || browserBypass) return;
    let active = true;
    setOnboarding("loading");
    void onboardingLoader().then(
      (state) => { if (active) setOnboarding(state); },
      () => { if (active) setOnboarding("error"); },
    );
    return () => { active = false; };
  }, [browserBypass, onboardingAttempt, onboardingLoader, scenario]);

  useEffect(() => {
    let active = true;
    void fontStatusLoader().then(
      (status) => {
        if (active) setFontStatus(status);
      },
      () => {
        if (active) setFontStatus("fallback");
      },
    );
    return () => {
      active = false;
    };
  }, [fontStatusLoader]);

  useEffect(() => {
    if (onboarding === "loading" || onboarding === "error" || !onboarding.completed) return;
    if (scenario !== undefined) {
      setShellState(scenario);
      return;
    }
    let active = true;
    setShellState("initializing");
    void capabilityLoader().then(
      (nextCapabilities) => {
        if (active) {
          setCapabilities(nextCapabilities);
          setShellState(nextCapabilities.sources.length === 0 ? "empty" : "ready");
        }
      },
      () => {
        if (active) setShellState("error");
      },
    );
    return () => {
      active = false;
    };
  }, [capabilityLoader, loadAttempt, onboarding, scenario]);

  const retryCapabilities = useCallback(() => {
    setLoadAttempt((attempt) => attempt + 1);
  }, []);

  useEffect(() => {
    const handleShortcut = (event: KeyboardEvent) => {
      if (isTextEditingTarget(event.target) || !event.ctrlKey || event.altKey || event.metaKey) {
        return;
      }
      const route = routeForShortcut(event.key);
      if (route !== undefined) {
        event.preventDefault();
        setActiveRoute(route);
        return;
      }
      if (event.key.toLowerCase() === "l") {
        event.preventDefault();
        setActiveRoute("radio");
        queueMicrotask(() => radioInputRef.current?.focus());
      } else if (event.key.toLowerCase() === "k") {
        event.preventDefault();
        setActiveRoute("library");
        queueMicrotask(() => librarySearchRef.current?.focus());
      } else if (event.key === ",") {
        event.preventDefault();
        setActiveRoute("settings");
      }
    };
    document.addEventListener("keydown", handleShortcut);
    return () => document.removeEventListener("keydown", handleShortcut);
  }, []);

  const operational = shellState !== "initializing" && shellState !== "error";
  useEffect(() => {
    if (focusAfterOnboarding && operational) {
      radioStartRef.current?.focus();
      setFocusAfterOnboarding(false);
    }
  }, [focusAfterOnboarding, operational]);

  if (onboarding === "loading") {
    return <div className="app-shell"><main className="state-page onboarding-gate" id="main-content">
      <p className="instrument-label">ONBOARDING / LOADING</p>
      <h1 className="hero-title">正在恢复设置</h1><p className="inline-status" role="status">[LOADING…]</p>
    </main></div>;
  }
  if (onboarding === "error") {
    return <div className="app-shell"><main className="state-page onboarding-gate" id="main-content">
      <p className="instrument-label">ONBOARDING / ERROR</p>
      <h1 className="hero-title">无法读取设置进度</h1>
      <p className="secondary-copy" role="alert">本地状态读取失败；尚未进入电台，也不会播放或调用服务。</p>
      <button className="control control--primary" type="button"
        onClick={() => setOnboardingAttempt((attempt) => attempt + 1)}>重试</button>
    </main></div>;
  }
  if (!onboarding.completed) {
    return <div className="app-shell"><OnboardingFlow state={onboarding} client={onboardingIpc}
      onStateChange={setOnboarding} onCompleted={(state) => {
        setActiveRoute("radio"); setOnboarding(state); setFocusAfterOnboarding(true);
      }} /></div>;
  }
  return (
    <div className="app-shell">
      <AppHeader activeRoute={activeRoute} state={shellState} onNavigate={setActiveRoute} />
      <main className="app-content" id="main-content">
        {shellState === "initializing" ? (
          <ShellStatePanel kind="initializing" onRetry={retryCapabilities} />
        ) : null}
        {shellState === "error" ? (
          <ShellStatePanel kind="error" onRetry={retryCapabilities} />
        ) : null}
        {operational ? (
          <>
            <ShellNotice state={shellState} />
            <div hidden={activeRoute !== "radio"}>
              <RadioPage
                capabilities={capabilities}
                draft={radioDraft}
                inputRef={radioInputRef}
                startButtonRef={radioStartRef}
                onDraftChange={setRadioDraft}
              />
            </div>
            <div hidden={activeRoute !== "library"}>
              <LibraryPage
                permissionDenied={shellState === "permission-denied"}
                query={libraryQuery}
                searchRef={librarySearchRef}
                onQueryChange={setLibraryQuery}
              />
            </div>
            <div hidden={activeRoute !== "you"}>
              <YouPage />
            </div>
            <div hidden={activeRoute !== "settings"}>
              <SettingsPage
                capabilities={capabilities}
                fontStatus={fontStatus}
                shellState={shellState}
              />
            </div>
          </>
        ) : null}
      </main>
    </div>
  );
}

function useMemoOnboardingClient(client: OnboardingClient | undefined): OnboardingClient {
  const ref = useRef<OnboardingClient | null>(null);
  if (ref.current === null) ref.current = client ?? createOnboardingClient();
  return ref.current;
}

function routeForShortcut(key: string): RouteId | undefined {
  if (key === "1") return "radio";
  if (key === "2") return "library";
  if (key === "3") return "you";
  if (key === "4") return "settings";
  return undefined;
}
