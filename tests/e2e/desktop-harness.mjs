import { spawn, spawnSync } from "node:child_process";
import { lstatSync } from "node:fs";
import { access, lstat, mkdtemp, readFile, readdir, realpath, rm, rmdir, stat } from "node:fs/promises";
import net from "node:net";
import { homedir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { WebDriverClient } from "./protocol.mjs";

const PINNED_TAURI_DRIVER = "2.0.6";
const E2E_IDENTIFIER = "com.cyberkindred.desktop.e2e";
const REPOSITORY_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const E2E_CONFIG = path.join(REPOSITORY_ROOT, "src-tauri", "tauri.e2e.conf.json");
const E2E_TARGET = path.join(REPOSITORY_ROOT, "target", "e2e");
const DEFAULT_APPLICATION = path.join(E2E_TARGET, "debug", "cyberkindred.exe");
const SOURCE_INPUTS = [
  "Cargo.lock",
  "Cargo.toml",
  "crates",
  "index.html",
  "package.json",
  "pnpm-lock.yaml",
  "pnpm-workspace.yaml",
  "rust-toolchain.toml",
  "src",
  "src-tauri",
  "tsconfig.json",
  "tsconfig.app.json",
  "tsconfig.node.json",
  "vite.config.ts",
];

export class DesktopPrerequisiteError extends Error {
  constructor(code, message) {
    super(message);
    this.name = "DesktopPrerequisiteError";
    this.code = code;
  }
}

export async function startDesktopSession() {
  if (process.platform !== "win32") {
    throw new DesktopPrerequisiteError(
      "windows_required",
      "Desktop E2E requires Windows because tauri-driver uses the system WebView2 driver.",
    );
  }

  await assertE2eConfiguration();
  const tauriDriver = resolvePinnedTauriDriver();
  const nativeDriver = resolveNativeDriver();
  await cleanupE2eDirectories();
  const application = await ensureApplication();
  const intermediaryPort = await reserveLoopbackPort();
  const nativePort = await reserveLoopbackPort(new Set([intermediaryPort]));
  const driver = spawn(
    tauriDriver,
    [
      "--port",
      String(intermediaryPort),
      "--native-port",
      String(nativePort),
      "--native-driver",
      nativeDriver,
    ],
    {
      cwd: REPOSITORY_ROOT,
      env: { ...process.env, NO_COLOR: "1" },
      stdio: ["ignore", "pipe", "pipe"],
      windowsHide: true,
    },
  );
  const output = captureBoundedOutput(driver);
  const client = new WebDriverClient(`http://127.0.0.1:${intermediaryPort}`, {
    timeoutMs: 5_000,
  });
  let closed = false;
  const close = async () => {
    if (closed) return;
    closed = true;
    process.removeListener("SIGINT", handleInterrupt);
    process.removeListener("SIGTERM", handleTerminate);
    try {
      await client.deleteSession();
    } catch {
      // Process-tree cleanup below is authoritative if the session endpoint
      // is already unavailable.
    } finally {
      await stopProcessTree(driver);
      await cleanupE2eDirectories();
    }
  };
  const handleInterrupt = () => {
    void close().finally(() => process.exit(130));
  };
  const handleTerminate = () => {
    void close().finally(() => process.exit(143));
  };
  process.once("SIGINT", handleInterrupt);
  process.once("SIGTERM", handleTerminate);

  try {
    await waitForDriverReady(client, driver, output);
    await client.createSession(application);
  } catch (error) {
    await close();
    if (error instanceof DesktopPrerequisiteError) throw error;
    throw classifyStartupFailure(error, output.text());
  }

  return {
    session: client,
    close,
  };
}

export async function ensureApplication() {
  const application = DEFAULT_APPLICATION;
  if (!(await isFreshApplication(application))) buildUnpackagedApplication();
  try {
    await access(application);
  } catch {
    throw new DesktopPrerequisiteError(
      "application_missing",
      "The unpackaged desktop executable is missing; run `pnpm tauri build --debug --no-bundle`.",
    );
  }
  return path.resolve(application);
}

export async function assertE2eConfiguration() {
  let parsed;
  try {
    parsed = JSON.parse(await readFile(E2E_CONFIG, "utf8"));
  } catch {
    throw new DesktopPrerequisiteError(
      "e2e_config_missing",
      "Create src-tauri/tauri.e2e.conf.json with the dedicated com.cyberkindred.desktop.e2e identifier before running desktop E2E.",
    );
  }
  if (parsed?.identifier !== E2E_IDENTIFIER) {
    throw new DesktopPrerequisiteError(
      "e2e_identifier_invalid",
      "The desktop E2E config must use the exact dedicated identifier com.cyberkindred.desktop.e2e; production identifiers are refused.",
    );
  }
}

export async function cleanupE2eDirectories() {
  for (const variable of ["APPDATA", "LOCALAPPDATA"]) {
    const root = process.env[variable];
    const target = validateCleanupTarget(root, variable);
    const lexicalRoot = path.dirname(target);
    try {
      await access(target);
    } catch (error) {
      if (!(error instanceof Error) || !Object.hasOwn(error, "code") || error.code !== "ENOENT") {
        throw new DesktopPrerequisiteError(
          "e2e_cleanup_access_failed",
          `The dedicated ${variable} test directory could not be inspected safely for cleanup.`,
        );
      }
      continue;
    }
    const targetMetadata = await lstat(target);
    if (!targetMetadata.isDirectory() || targetMetadata.isSymbolicLink()) {
      throw new DesktopPrerequisiteError(
        "e2e_cleanup_reparse_refused",
        `Refusing cleanup because the dedicated ${variable} target is not a regular directory.`,
      );
    }
    let effectiveRoot;
    let realTarget;
    try {
      [effectiveRoot, realTarget] = await Promise.all([
        resolveEffectiveDataRoot(lexicalRoot),
        realpath(target),
      ]);
    } catch {
      throw new DesktopPrerequisiteError(
        "e2e_cleanup_resolution_failed",
        `The dedicated ${variable} test directory could not be resolved safely for cleanup.`,
      );
    }
    if (!isExactNamedChild(realTarget, effectiveRoot, E2E_IDENTIFIER)) {
      throw new DesktopPrerequisiteError(
        "e2e_cleanup_reparse_refused",
        `Refusing cleanup because the dedicated ${variable} target resolves outside its expected root.`,
      );
    }
    try {
      await rm(realTarget, { recursive: true, force: true, maxRetries: 2 });
    } catch {
      throw new DesktopPrerequisiteError(
        "e2e_cleanup_failed",
        `The dedicated ${variable} test directory could not be removed; close stale CyberKindred E2E processes and retry.`,
      );
    }
  }
}

async function resolveEffectiveDataRoot(lexicalRoot) {
  const probePrefix = path.join(lexicalRoot, ".cyberkindred-e2e-root-probe-");
  let probe;
  try {
    probe = await mkdtemp(probePrefix);
    if (!isDirectChild(probe, lexicalRoot)) {
      throw new Error("probe escaped lexical root");
    }
    return path.dirname(await realpath(probe));
  } finally {
    if (probe !== undefined) await rmdir(probe);
  }
}

function isExactNamedChild(candidate, parent, name) {
  return isDirectChild(candidate, parent) && windowsPathEqual(path.basename(candidate), name);
}

function isDirectChild(candidate, parent) {
  return windowsPathEqual(path.dirname(candidate), parent);
}

function windowsPathEqual(left, right) {
  return path.normalize(left).toLocaleLowerCase("en-US") === path.normalize(right).toLocaleLowerCase("en-US");
}

export function validateCleanupTarget(root, variable = "application data") {
  if (root === undefined || !path.isAbsolute(root)) {
      throw new DesktopPrerequisiteError(
        "e2e_data_root_invalid",
        `${variable} must resolve to an absolute Windows application-data directory.`,
      );
  }
  const resolvedRoot = path.resolve(root);
  const target = path.resolve(resolvedRoot, E2E_IDENTIFIER);
  if (path.dirname(target) !== resolvedRoot || path.basename(target) !== E2E_IDENTIFIER) {
    throw new DesktopPrerequisiteError(
      "e2e_cleanup_scope_invalid",
      `Refusing cleanup because the dedicated ${variable} target failed scope validation.`,
    );
  }
  return target;
}

export function resolvePinnedTauriDriver() {
  const installed = spawnSync("cargo", ["install", "--list"], {
    cwd: REPOSITORY_ROOT,
    encoding: "utf8",
    windowsHide: true,
  });
  if (
    installed.status !== 0 ||
    !new RegExp(`^tauri-driver v${escapeRegex(PINNED_TAURI_DRIVER)}:$`, "m").test(
      installed.stdout ?? "",
    )
  ) {
    throw new DesktopPrerequisiteError(
      "tauri_driver_unpinned",
      `Install the pinned desktop driver with \`cargo install tauri-driver --version ${PINNED_TAURI_DRIVER} --locked\`.`,
    );
  }
  const cargoHome = process.env.CARGO_HOME ?? path.join(homedir(), ".cargo");
  const executable = path.join(cargoHome, "bin", "tauri-driver.exe");
  if (!path.isAbsolute(executable) || !isRegularFile(executable)) {
    throw new DesktopPrerequisiteError(
      "tauri_driver_missing",
      "tauri-driver 2.0.6 is registered by Cargo but its Cargo-home executable is unavailable or unsafe.",
    );
  }
  return executable;
}

export function resolveNativeDriver() {
  const configured = process.env.TAURI_NATIVE_DRIVER_PATH;
  if (configured !== undefined) {
    if (
      !path.isAbsolute(configured) ||
      path.basename(configured).toLowerCase() !== "msedgedriver.exe" ||
      !isRegularFile(configured)
    ) {
      throw new DesktopPrerequisiteError(
        "native_driver_path_invalid",
        "TAURI_NATIVE_DRIVER_PATH must be an absolute path to msedgedriver.exe.",
      );
    }
    assertNativeDriverExecutable(configured);
    return configured;
  }
  const executable = findOnPath("msedgedriver.exe");
  if (executable === undefined) {
    throw new DesktopPrerequisiteError(
      "native_driver_missing",
      "Install the Microsoft Edge WebDriver matching the installed WebView2/Edge runtime, then put msedgedriver.exe on PATH or set TAURI_NATIVE_DRIVER_PATH to its absolute path.",
    );
  }
  assertNativeDriverExecutable(executable);
  return executable;
}

function assertNativeDriverExecutable(executable) {
  const version = spawnSync(executable, ["--version"], {
    encoding: "utf8",
    shell: false,
    windowsHide: true,
  });
  const validVersion = /^(?:Microsoft Edge WebDriver|MSEdgeDriver) \d+\.\d+\.\d+\.\d+/u.test(
    version.stdout ?? "",
  );
  if (version.status !== 0 || !validVersion) {
    throw new DesktopPrerequisiteError(
      "native_driver_invalid",
      "The configured msedgedriver.exe is not a valid Microsoft Edge WebDriver executable.",
    );
  }
}

function isRegularFile(candidate) {
  try {
    const metadata = lstatSync(candidate);
    return metadata.isFile() && !metadata.isSymbolicLink();
  } catch {
    return false;
  }
}

async function isFreshApplication(application) {
  let applicationStat;
  try {
    applicationStat = await stat(application);
  } catch {
    return false;
  }
  for (const input of SOURCE_INPUTS) {
    const newest = await newestModifiedTime(path.join(REPOSITORY_ROOT, input));
    if (newest > applicationStat.mtimeMs) return false;
  }
  return true;
}

async function newestModifiedTime(input) {
  let inputStat;
  try {
    inputStat = await stat(input);
  } catch {
    return 0;
  }
  if (!inputStat.isDirectory()) return inputStat.mtimeMs;
  let newest = inputStat.mtimeMs;
  for (const entry of await readdir(input, { withFileTypes: true })) {
    if (entry.isSymbolicLink()) continue;
    newest = Math.max(newest, await newestModifiedTime(path.join(input, entry.name)));
  }
  return newest;
}

function buildUnpackagedApplication() {
  const npmExecutable = process.env.npm_execpath;
  const command =
    npmExecutable === undefined
      ? {
          executable: "pnpm",
          arguments: [
            "tauri",
            "build",
            "--debug",
            "--no-bundle",
            "--config",
            "src-tauri/tauri.e2e.conf.json",
          ],
          shell: true,
        }
      : {
          executable: process.execPath,
          arguments: [
            npmExecutable,
            "tauri",
            "build",
            "--debug",
            "--no-bundle",
            "--config",
            "src-tauri/tauri.e2e.conf.json",
          ],
          shell: false,
        };
  const result = spawnSync(command.executable, command.arguments, {
    cwd: REPOSITORY_ROOT,
    encoding: "utf8",
    env: { ...process.env, CARGO_TARGET_DIR: E2E_TARGET },
    maxBuffer: 1_048_576,
    stdio: "pipe",
    shell: command.shell,
    windowsHide: true,
  });
  if (result.status !== 0) {
    throw new DesktopPrerequisiteError(
      "application_build_failed",
      "The unpackaged desktop build failed; rerun `pnpm tauri build --debug --no-bundle` for build diagnostics.",
    );
  }
}

function findOnPath(executable) {
  const located = spawnSync("where.exe", [executable], {
    encoding: "utf8",
    windowsHide: true,
  });
  if (located.status !== 0) return undefined;
  const first = (located.stdout ?? "")
    .split(/\r?\n/u)
    .map((value) => value.trim())
    .find((value) => value.length > 0 && path.isAbsolute(value));
  return first;
}

async function reserveLoopbackPort(excluded = new Set()) {
  for (let attempt = 0; attempt < 5; attempt += 1) {
    const port = await new Promise((resolve, reject) => {
      const server = net.createServer();
      server.unref();
      server.once("error", reject);
      server.listen(0, "127.0.0.1", () => {
        const address = server.address();
        const selected = typeof address === "object" && address !== null ? address.port : 0;
        server.close((error) => (error === undefined ? resolve(selected) : reject(error)));
      });
    });
    if (port > 0 && !excluded.has(port)) return port;
  }
  throw new DesktopPrerequisiteError(
    "loopback_port_unavailable",
    "Unable to reserve distinct loopback ports for the desktop driver.",
  );
}

async function waitForDriverReady(client, driver, output) {
  const deadline = Date.now() + 10_000;
  while (Date.now() < deadline) {
    if (hasProcessExited(driver) || output.spawnFailed()) {
      throw classifyStartupFailure(undefined, output.text());
    }
    try {
      await client.status();
      return;
    } catch {
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
  }
  throw new DesktopPrerequisiteError(
    "tauri_driver_timeout",
    "tauri-driver did not become ready on loopback within 10 seconds; verify that its matching native WebDriver can start.",
  );
}

function captureBoundedOutput(child) {
  let captured = "";
  let failed = false;
  const append = (chunk) => {
    captured = `${captured}${chunk.toString("utf8")}`.slice(-8_192);
  };
  child.stdout?.on("data", append);
  child.stderr?.on("data", append);
  child.once("error", () => {
    failed = true;
  });
  return { text: () => captured, spawnFailed: () => failed };
}

function classifyStartupFailure(error, output) {
  if (/version of msedgedriver only supports|session not created/iu.test(output)) {
    return new DesktopPrerequisiteError(
      "native_driver_version_mismatch",
      "msedgedriver.exe does not match the installed WebView2/Edge runtime; install the matching Microsoft Edge WebDriver and retry.",
    );
  }
  if (/cannot find|not found|no such file/iu.test(output)) {
    return new DesktopPrerequisiteError(
      "desktop_dependency_missing",
      "A desktop-driver prerequisite could not be started; verify the pinned tauri-driver, matching msedgedriver.exe, and unpackaged application.",
    );
  }
  if (error instanceof Error && error.message.includes("timed out")) {
    return new DesktopPrerequisiteError(
      "desktop_session_timeout",
      "The desktop WebDriver session timed out; verify that msedgedriver matches WebView2/Edge and no stale driver process is holding the selected port.",
    );
  }
  return new DesktopPrerequisiteError(
    "desktop_session_failed",
    "The desktop WebDriver session could not start; verify the pinned tauri-driver, matching msedgedriver.exe, and unpackaged application.",
  );
}

async function stopProcessTree(child) {
  if (child.pid === undefined) return;
  if (hasProcessExited(child)) return;
  child.kill("SIGTERM");
  if (await waitForProcessExit(child, 2_000)) return;
  if (hasProcessExited(child)) return;
  if (process.platform === "win32") {
    spawnSync("taskkill.exe", ["/PID", String(child.pid), "/T", "/F"], {
      stdio: "ignore",
      windowsHide: true,
    });
    return;
  }
}

function waitForProcessExit(child, timeoutMs) {
  if (hasProcessExited(child)) return Promise.resolve(true);
  return new Promise((resolve) => {
    const onExit = () => finish(true);
    const timer = setTimeout(() => finish(hasProcessExited(child)), timeoutMs);
    const finish = (exited) => {
      clearTimeout(timer);
      child.removeListener("exit", onExit);
      resolve(exited);
    };
    child.once("exit", onExit);
  });
}

function hasProcessExited(child) {
  return child.exitCode !== null || child.signalCode !== null;
}

export function safeDiagnosticMessage(error) {
  if (!(error instanceof Error)) return "unknown failure";
  return error.message
    .replace(/[\r\n]/gu, " ")
    .replace(/\b(?:sk|key|secret|token)-[A-Za-z0-9._-]+/giu, "<redacted-secret>")
    .replace(/[A-Za-z]:\\.*$/gu, "<redacted-path>")
    .replace(/\/(?:Users|home)\/.*$/gu, "<redacted-path>")
    .slice(0, 500);
}

function escapeRegex(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&");
}
