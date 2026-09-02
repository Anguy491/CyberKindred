import { render, screen } from "@testing-library/react";

import { App } from "../../src/App";

describe("[NFR-MAINT-002] application foundation", () => {
  it("renders a silent, static foundation state", () => {
    render(<App />);

    expect(screen.getByRole("heading", { name: "CYBERKINDRED" })).not.toBeNull();
    expect(screen.getByRole("status").textContent).toBe(
      "M2 FOUNDATION / SILENT",
    );
    expect(screen.queryByRole("button")).toBeNull();
  });
});
