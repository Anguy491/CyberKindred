import assert from "node:assert/strict";
import test from "node:test";

import { desktopCases, selectDesktopCases } from "./cases.mjs";

test("desktop cases carry requirement identifiers [NFR-MAINT-003]", () => {
  assert.ok(desktopCases.length > 0);
  for (const testCase of desktopCases) {
    assert.match(testCase.name, /\[(?:FR|NFR)-[A-Z]+-\d{3}\]/u);
  }
});

test("case filtering supports pnpm test:e2e -- filter [NFR-MAINT-001]", () => {
  const selected = selectDesktopCases(desktopCases, "navigation");
  assert.equal(selected.length, 1);
  assert.match(selected[0].name, /navigation/u);
  assert.throws(() => selectDesktopCases(desktopCases, "does-not-exist"), /No desktop E2E case/u);
});
