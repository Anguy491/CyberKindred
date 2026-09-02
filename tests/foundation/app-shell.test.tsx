import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { App } from "../../src/App";
import { FOUNDATION_CAPABILITIES } from "../../src/design/foundation";

const loadedFonts = async () => "loaded" as const;

describe("[TASK-009] application shell", () => {
  // UX-STA-001; NFR-MAINT-002.
  it("renders a silent initializing state while API-001 is pending", () => {
    render(<App capabilityLoader={() => new Promise(() => undefined)} fontStatusLoader={loadedFonts} />);

    expect(screen.getByRole("heading", { name: "正在读取本机能力" })).not.toBeNull();
    expect(screen.getByRole("progressbar", { name: "正在初始化" })).not.toBeNull();
    expect(screen.queryByText("开始节目")).toBeNull();
  });

  // UX-NAV-002; FR-RAD-004; FR-LIB-005; FR-MEM-001; FR-SET-004.
  it("renders the four Chinese-labelled pages after a hermetic capability load", async () => {
    render(<App capabilityLoader={async () => FOUNDATION_CAPABILITIES} fontStatusLoader={loadedFonts} />);
    expect(await screen.findByRole("heading", { name: "今天想听什么状态？" })).not.toBeNull();

    const expected = [
      ["曲库", "还没有本地曲库"],
      ["了解", "CyberKindred 目前如何了解你"],
      ["设置", "AI & VOICE"],
      ["电台", "今天想听什么状态？"],
    ] as const;
    for (const [navigationName, heading] of expected) {
      fireEvent.click(screen.getByRole("button", { name: navigationName }));
      expect(screen.getByRole("heading", { name: heading })).not.toBeNull();
    }
    expect(screen.getByRole("button", { name: "开始节目" }).getAttribute("aria-disabled")).toBe("true");
  });

  // UX-NAV-002/003; NFR-A11Y-001.
  it("supports keyboard routes and preserves page-local text", async () => {
    const user = userEvent.setup();
    render(<App scenario="ready" fontStatusLoader={loadedFonts} />);
    const radioInput = screen.getByRole("textbox", { name: /告诉 CyberKindred/u });
    await user.type(radioInput, "安静一点");
    await user.click(screen.getByRole("button", { name: "曲库" }));
    const librarySearch = screen.getByRole("searchbox", { name: /搜索曲库/u });
    await user.type(librarySearch, "夜晚");

    fireEvent.keyDown(document, { key: "1", ctrlKey: true });
    expect(screen.getByRole("textbox", { name: /告诉 CyberKindred/u })).toHaveProperty("value", "安静一点");
    fireEvent.keyDown(document, { key: "k", ctrlKey: true });
    await waitFor(() => expect(document.activeElement).toBe(librarySearch));
    expect(librarySearch).toHaveProperty("value", "夜晚");
    fireEvent.keyDown(document, { key: ",", ctrlKey: true });
    expect(screen.getByRole("heading", { name: "AI & VOICE" })).not.toBeNull();
  });

  // UX-STA-002/003/004/005/006; NFR-A11Y-002.
  it.each([
    ["offline", "[OFFLINE]"],
    ["degraded", "[TTS UNAVAILABLE — TEXT CONTINUES]"],
    ["permission-denied", "[ERROR: PERMISSION DENIED]"],
  ] as const)("renders the %s foundation state with text", (scenario, expected) => {
    render(
      <App
        scenario={scenario}
        initialRoute={scenario === "permission-denied" ? "library" : "radio"}
        fontStatusLoader={loadedFonts}
      />,
    );
    expect(screen.getAllByText(expected).length).toBeGreaterThan(0);
  });

  // UX-STA-003; NFR-MAINT-002.
  it("renders a local actionable error without exposing rejection text", async () => {
    render(
      <App
        capabilityLoader={async () => {
          throw new Error("C:\\Users\\Alice\\Music sk-private-canary");
        }}
        fontStatusLoader={loadedFonts}
      />,
    );
    expect(await screen.findByRole("alert")).toHaveProperty("textContent", "[ERROR: IPC]");
    expect(document.body.textContent).not.toContain("sk-private-canary");
    expect(screen.getByRole("button", { name: "重新读取" })).not.toBeNull();
  });
});
