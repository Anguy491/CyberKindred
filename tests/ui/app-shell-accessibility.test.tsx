import { render, screen } from "@testing-library/react";
import axe from "axe-core";

import { App } from "../../src/App";
import { FOUNDATION_CAPABILITIES } from "../../src/design/foundation";
import { BROWSER_SETTINGS_IPC } from "../../src/features/settings";
import { OnboardingFlow } from "../../src/onboarding/OnboardingFlow";
import type { OnboardingClient } from "../../src/onboarding/state";
import type { OnboardingState } from "../../src/ipc";
import { SettingsPage } from "../../src/routes/SettingsPage";

// TEST-A11Y-001/002; NFR-A11Y-001..004.
describe("TASK-030 focused accessibility checks", () => {
  it.each(["radio", "library", "you", "settings"] as const)(
    "has no automatic axe violations on the %s shell",
    async (initialRoute) => {
      const { container, findByText } = render(
        <App
          scenario="ready"
          initialRoute={initialRoute}
          fontStatusLoader={async () => "loaded"}
        />,
      );
      await findByText("[LOCAL FONTS]");
      const results = await axe.run(container, {
        rules: { "color-contrast": { enabled: false } },
      });
      expect(results.violations).toEqual([]);
      if (initialRoute === "radio") {
        const region = container.querySelector(".radio-current");
        expect(region?.getAttribute("aria-live")).toBe("polite");
        expect(region?.getAttribute("aria-atomic")).toBe("true");
        expect(region?.textContent).not.toContain("进度");
        expect(container.querySelector(".radio-now")?.getAttribute("aria-live")).toBeNull();

        const unavailable = container.querySelector<HTMLButtonElement>("[data-testid='radio-start']");
        const reasonId = unavailable?.getAttribute("aria-describedby");
        expect(unavailable?.getAttribute("aria-disabled")).toBe("true");
        expect(reasonId).not.toBeNull();
        expect(container.ownerDocument.getElementById(reasonId ?? "")?.textContent)
          .toContain("[UNAVAILABLE");
      }
    },
  );

  // UX-SYS-001; NFR-A11Y-002.
  it("surfaces a font fallback diagnostic in Settings", async () => {
    const { findByText } = render(
      <App
        scenario="ready"
        initialRoute="settings"
        fontStatusLoader={async () => "fallback"}
      />,
    );
    expect(await findByText("[FONT FALLBACK]")).not.toBeNull();
    expect(document.body.textContent).toContain("[FONT FALLBACK]");
  });

  // FR-ONB-001; NFR-A11Y-001/002; TEST-A11Y-002.
  it("has no automatic axe violations on the onboarding welcome step", async () => {
    const state: OnboardingState = {
      completed: false, completedSteps: [], sourceSelection: [], aiMode: null, voiceMode: null,
      cityScheduleMode: null,
      profile: { displayName: "", companionStyle: "quiet_warm", initialPreferences: [], narrationDensity: "balanced" },
      privacyConfirmations: { explicitSound: false, rawConversationRetention: false }, revision: 0,
    };
    const client = {} as OnboardingClient;
    const { container } = render(<div className="app-shell"><OnboardingFlow state={state} client={client}
      onStateChange={() => undefined} onCompleted={() => undefined} /></div>);
    const results = await axe.run(container, { rules: { "color-contrast": { enabled: false } } });
    expect(results.violations).toEqual([]);
  });

  // UX-STA-003; NFR-A11Y-003.
  it("announces Settings failures assertively", async () => {
    render(<SettingsPage capabilities={FOUNDATION_CAPABILITIES} fontStatus="loaded" shellState="ready"
      ipc={{
        ...BROWSER_SETTINGS_IPC,
        getSettings: async () => { throw new Error("test-only settings failure"); },
      }} />);
    expect(await screen.findByRole("alert")).toHaveProperty("textContent", "[SETTINGS UNAVAILABLE]");
  });
});
