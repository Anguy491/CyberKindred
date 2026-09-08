import { readdirSync, statSync } from "node:fs";
import { basename, relative, resolve } from "node:path";

import { hashFile, readJson, run, workspaceRoot, writeJson } from "./lib.mjs";

function argument(name, fallback) {
  const index = process.argv.indexOf(name);
  return index >= 0 ? process.argv[index + 1] : fallback;
}

const packageJson = readJson(resolve(workspaceRoot, "package.json"));
const input = resolve(argument("--input", resolve(workspaceRoot, "target/release-artifacts", packageJson.version)));
const output = resolve(argument("--output", resolve(input, "manifest.json")));
const overrideConfigArgument = argument("--config");
const baseConfigPath = resolve(workspaceRoot, "src-tauri/tauri.conf.json");
const overrideConfigPath = overrideConfigArgument ? resolve(workspaceRoot, overrideConfigArgument) : undefined;
for (const configPath of [baseConfigPath, overrideConfigPath].filter(Boolean)) {
  if (!configPath.startsWith(`${workspaceRoot}\\`) && !configPath.startsWith(`${workspaceRoot}/`)) {
    throw new Error("release config must be inside the workspace");
  }
}
const baseConfig = readJson(baseConfigPath);
const overrideConfig = overrideConfigPath ? readJson(overrideConfigPath) : {};
const effectiveConfig = { ...baseConfig, ...overrideConfig };
const expectedReleaseTestPath = resolve(workspaceRoot, "scripts/release/tauri.release-test.conf.json");
if (overrideConfigPath && overrideConfigPath !== expectedReleaseTestPath) {
  throw new Error("only the repository release-test config may override an installer build");
}
if (
  overrideConfigPath
  && (
    overrideConfig.identifier !== "com.cyberkindred.release-test"
    || overrideConfig.productName !== "CyberKindred Release Test"
    || Object.keys(overrideConfig).sort().join(",") !== "identifier,productName"
  )
) {
  throw new Error("release-test config identity or scope is invalid");
}
if (
  effectiveConfig.version !== packageJson.version
  || effectiveConfig.bundle?.windows?.webviewInstallMode?.type !== "embedBootstrapper"
  || effectiveConfig.bundle?.windows?.nsis?.installMode !== "currentUser"
  || effectiveConfig.bundle?.windows?.allowDowngrades !== false
) {
  throw new Error("effective Tauri config does not satisfy the release installer policy");
}

function filesBelow(root) {
  const result = [];
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    const path = resolve(root, entry.name);
    if (entry.isDirectory()) {
      result.push(...filesBelow(path));
    } else if (entry.isFile() && path !== output) {
      result.push(path);
    }
  }
  return result;
}

const artifactFiles = filesBelow(input).sort();
if (artifactFiles.length === 0) {
  throw new Error(`release artifact directory is empty: ${input}`);
}
const localTools = resolve(workspaceRoot, "target/.tauri");
const buildInputs = statSync(localTools, { throwIfNoEntry: false })?.isDirectory()
  ? filesBelow(localTools).sort().map((path) => ({
    path: relative(localTools, path).replaceAll("\\", "/"),
    bytes: statSync(path).size,
    sha256: hashFile(path),
  }))
  : [];
const commit = run("git", ["rev-parse", "HEAD"]).trim();
const commitTimestamp = Number(run("git", ["show", "-s", "--format=%ct", "HEAD"]).trim());
const migrationVersions = readdirSync(resolve(workspaceRoot, "src-tauri/migrations"))
  .map((name) => Number(name.match(/^V(\d+)__/u)?.[1]))
  .filter(Number.isFinite);
const installerFiles = artifactFiles.filter((path) => /-setup\.exe$/iu.test(path));
if (installerFiles.length !== 1) {
  throw new Error(`expected one NSIS setup executable, found ${installerFiles.length}`);
}
const installerFile = installerFiles[0];
const configPaths = [baseConfigPath, overrideConfigPath].filter(Boolean);
const configSnapshots = new Map([
  [baseConfigPath, resolve(input, "build-inputs/tauri.conf.json")],
  ...(overrideConfigPath
    ? [[overrideConfigPath, resolve(input, "build-inputs/tauri.release-test.conf.json")]]
    : []),
]);
for (const [configPath, snapshotPath] of configSnapshots) {
  if (statSync(snapshotPath, { throwIfNoEntry: false })?.isFile() !== true) {
    throw new Error(`release config snapshot is missing: ${relative(input, snapshotPath)}`);
  }
  if (hashFile(configPath) !== hashFile(snapshotPath)) {
    throw new Error(`release config snapshot differs: ${relative(workspaceRoot, configPath)}`);
  }
}

const manifest = {
  schemaVersion: 1,
  version: packageJson.version,
  commit,
  sourceDateEpoch: commitTimestamp,
  target: "x86_64-pc-windows-msvc",
  schemaDatabaseVersion: Math.max(...migrationVersions),
  unsigned: true,
  identifier: effectiveConfig.identifier,
  productName: effectiveConfig.productName,
  releaseTest:
    effectiveConfig.identifier === "com.cyberkindred.release-test"
    && effectiveConfig.productName === "CyberKindred Release Test",
  webView2InstallMode: effectiveConfig.bundle?.windows?.webviewInstallMode?.type,
  installerPolicy: {
    installMode: effectiveConfig.bundle?.windows?.nsis?.installMode,
    allowDowngrades: effectiveConfig.bundle?.windows?.allowDowngrades,
  },
  configuration: configPaths.map((path) => ({
    file: relative(workspaceRoot, path).replaceAll("\\", "/"),
    snapshot: relative(input, configSnapshots.get(path)).replaceAll("\\", "/"),
    sha256: hashFile(path),
  })),
  installer: {
    file: relative(input, installerFile).replaceAll("\\", "/"),
    sha256: hashFile(installerFile),
  },
  toolchain: {
    node: run("node", ["--version"]).trim(),
    pnpm: run("pnpm", ["--version"]).trim(),
    rustc: run("rustc", ["--version"]).trim(),
    cargo: run("cargo", ["--version"]).trim(),
    tauri: run("pnpm", ["tauri", "--version"]).trim().split(/\r?\n/u).at(-1),
  },
  artifacts: artifactFiles.map((path) => ({
    file: relative(input, path).replaceAll("\\", "/"),
    bytes: statSync(path).size,
    sha256: hashFile(path),
  })),
  buildInputs,
};
writeJson(output, manifest);
console.log(`Artifact manifest written: ${basename(output)} (${manifest.artifacts.length} artifacts, ${buildInputs.length} cached Tauri build inputs).`);
