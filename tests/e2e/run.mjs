import { desktopCases, selectDesktopCases } from "./cases.mjs";
import {
  DesktopPrerequisiteError,
  safeDiagnosticMessage,
  startDesktopSession,
} from "./desktop-harness.mjs";

const filter = process.argv.slice(2).join(" ");
let desktop;

try {
  const selected = selectDesktopCases(desktopCases, filter);
  desktop = await startDesktopSession();
  for (const testCase of selected) {
    process.stdout.write(`RUN ${testCase.name}\n`);
    await testCase.run(desktop.session);
    process.stdout.write(`PASS ${testCase.name}\n`);
  }
} catch (error) {
  if (error instanceof DesktopPrerequisiteError) {
    process.stderr.write(`DESKTOP E2E PREREQUISITE [${error.code}]: ${error.message}\n`);
  } else {
    process.stderr.write(`DESKTOP E2E FAILED: ${safeDiagnosticMessage(error)}\n`);
  }
  process.exitCode = 1;
} finally {
  try {
    await desktop?.close();
  } catch (error) {
    process.stderr.write(`DESKTOP E2E CLEANUP FAILED: ${safeDiagnosticMessage(error)}\n`);
    process.exitCode = 1;
  }
}
