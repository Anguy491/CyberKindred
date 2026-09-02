import { render } from "@testing-library/react";
import axe from "axe-core";

import { App } from "../../src/App";
import { OnboardingFlow } from "../../src/onboarding/OnboardingFlow";
import type { OnboardingClient } from "../../src/onboarding/state";
import type { OnboardingState } from "../../src/ipc";

// TEST-A11Y-002; NFR-A11Y-002; NFR-A11Y-004.
describe("TASK-009 focused axe checks", () => {
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
});
