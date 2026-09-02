import { render } from "@testing-library/react";
import axe from "axe-core";

import { App } from "../../src/App";

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
});
