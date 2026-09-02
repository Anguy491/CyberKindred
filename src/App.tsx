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
import { LibraryPage } from "./routes/LibraryPage";
import { RadioPage } from "./routes/RadioPage";
import { SettingsPage } from "./routes/SettingsPage";
import { YouPage } from "./routes/YouPage";

export interface AppProps {
  readonly capabilityLoader?: CapabilityLoader;
  readonly fontStatusLoader?: FontStatusLoader;
  readonly initialRoute?: RouteId;
  /** Deterministic visual scenario injection for hermetic UI tests. */
  readonly scenario?: FoundationState;
}

export function App({
  capabilityLoader = loadFoundationCapabilities,
  fontStatusLoader = detectLocalFontStatus,
  initialRoute = "radio",
  scenario,
}: AppProps) {
  const [activeRoute, setActiveRoute] = useState<RouteId>(initialRoute);
  const [shellState, setShellState] = useState<FoundationState>(scenario ?? "initializing");
  const [capabilities, setCapabilities] = useState<AppCapabilities>(FOUNDATION_CAPABILITIES);
  const [fontStatus, setFontStatus] = useState<FontStatus>("checking");
  const [loadAttempt, setLoadAttempt] = useState(0);
  const [radioDraft, setRadioDraft] = useState("");
  const [libraryQuery, setLibraryQuery] = useState("");
  const radioInputRef = useRef<HTMLTextAreaElement>(null);
  const librarySearchRef = useRef<HTMLInputElement>(null);

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
  }, [capabilityLoader, loadAttempt, scenario]);

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

function routeForShortcut(key: string): RouteId | undefined {
  if (key === "1") return "radio";
  if (key === "2") return "library";
  if (key === "3") return "you";
  if (key === "4") return "settings";
  return undefined;
}
