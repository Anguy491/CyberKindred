import assert from "node:assert/strict";
import path from "node:path";
import test from "node:test";

import {
  assertE2eConfiguration,
  safeDiagnosticMessage,
  validateCleanupTarget,
} from "./desktop-harness.mjs";

test("harness requires the dedicated Tauri identifier [NFR-PRIV-002]", async () => {
  await assert.doesNotReject(assertE2eConfiguration());
});

test("cleanup target is one exact child of its application-data root [NFR-SEC-003]", () => {
  const root = path.resolve("C:\\e2e-fixture", "AppData", "Roaming");
  const target = validateCleanupTarget(root, "fixture");
  assert.equal(path.dirname(target), root);
  assert.equal(path.basename(target), "com.cyberkindred.desktop.e2e");
  assert.throws(() => validateCleanupTarget("relative", "fixture"), /absolute Windows/u);
});

test("diagnostics redact paths with spaces and secret-shaped values [NFR-SEC-001]", () => {
  const message = safeDiagnosticMessage(
    new Error("failed for sk-test-canary at C:\\Users\\Jane Doe\\private music\\track.mp3"),
  );
  assert.equal(message, "failed for <redacted-secret> at <redacted-path>");
});
