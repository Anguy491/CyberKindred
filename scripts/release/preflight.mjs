import { existsSync } from "node:fs";
import { resolve } from "node:path";

import {
  defaultLicenseRoot,
  readJson,
  run,
  workspaceRoot,
} from "./lib.mjs";

const development = process.argv.includes("--development");
const failures = [];
const warnings = [];
const packageJson = readJson(resolve(workspaceRoot, "package.json"));
const tauriConfig = readJson(resolve(workspaceRoot, "src-tauri/tauri.conf.json"));
const releaseTestConfig = readJson(resolve(workspaceRoot, "scripts/release/tauri.release-test.conf.json"));
const cargoMetadata = JSON.parse(run("cargo", ["metadata", "--locked", "--no-deps", "--format-version", "1"]));
const cargoPackage = cargoMetadata.packages.find((entry) => entry.name === "cyberkindred");

function requireCondition(condition, message) {
  if (!condition) {
    failures.push(message);
  }
}

requireCondition(packageJson.version === tauriConfig.version, "package.json and tauri.conf.json versions differ");
requireCondition(process.platform === "win32" && process.arch === "x64", "release build host must be Windows x64");
requireCondition(packageJson.version === cargoPackage?.version, "package.json and Cargo workspace versions differ");
requireCondition(packageJson.packageManager === "pnpm@11.16.0", "packageManager must remain pnpm@11.16.0");
requireCondition(run("node", ["--version"]).trim() === "v24.19.0", "Node must be 24.19.0");
requireCondition(run("pnpm", ["--version"]).trim() === "11.16.0", "pnpm must be 11.16.0");
requireCondition(run("rustc", ["--version"]).startsWith("rustc 1.98.0 "), "Rust must be 1.98.0");
requireCondition(run("cargo-audit", ["--version"]).trim() === "cargo-audit 0.22.2", "cargo-audit must be 0.22.2");
requireCondition(run("cargo-deny", ["--version"]).trim() === "cargo-deny 0.20.2", "cargo-deny must be 0.20.2");
requireCondition(
  run("pnpm", ["tauri", "--version"]).trim().split(/\r?\n/u).at(-1) === "tauri-cli 2.11.4",
  "Tauri CLI must be 2.11.4",
);
requireCondition(
  run("rustup", ["target", "list", "--installed"]).split(/\r?\n/u).includes("x86_64-pc-windows-msvc"),
  "x86_64-pc-windows-msvc target must be installed",
);
requireCondition(tauriConfig.bundle?.active === true, "Tauri bundle must be active");
requireCondition(
  Array.isArray(tauriConfig.bundle?.targets)
    && tauriConfig.bundle.targets.length === 1
    && tauriConfig.bundle.targets[0] === "nsis",
  "Tauri bundle target must be exactly nsis",
);
requireCondition(tauriConfig.bundle?.createUpdaterArtifacts === false, "unsigned beta must not create updater artifacts");
requireCondition(tauriConfig.bundle?.windows?.nsis?.installMode === "currentUser", "NSIS installMode must be currentUser");
requireCondition(tauriConfig.bundle?.windows?.nsis?.compression === "lzma", "NSIS compression must be pinned to lzma");
requireCondition(
  tauriConfig.bundle?.windows?.nsis?.installerHooks === "windows/nsis-hooks.nsh",
  "NSIS installer hooks must use the repository-owned retention notice",
);
requireCondition(tauriConfig.bundle?.windows?.allowDowngrades === false, "NSIS must reject installer downgrades");
requireCondition(
  tauriConfig.bundle?.windows?.webviewInstallMode?.type === "embedBootstrapper",
  "WebView2 install strategy must be explicit embedBootstrapper",
);
requireCondition(tauriConfig.bundle?.windows?.webviewInstallMode?.silent === true, "WebView2 bootstrapper must be silent");
requireCondition(tauriConfig.bundle?.useLocalToolsDir === true, "Tauri build tools must be retained in target/.tauri for hashing");
requireCondition(
  tauriConfig.bundle?.resources?.["resources/licenses/"] === "licenses/",
  "generated release assets must be bundled under the offline licenses directory",
);
requireCondition(
  releaseTestConfig.identifier === "com.cyberkindred.release-test"
    && releaseTestConfig.productName === "CyberKindred Release Test"
    && Object.keys(releaseTestConfig).sort().join(",") === "identifier,productName",
  "isolated installer config must use the fixed release-test product and identifier",
);

for (const path of ["Cargo.lock", "pnpm-lock.yaml", "rust-toolchain.toml", ".node-version"]) {
  requireCondition(existsSync(resolve(workspaceRoot, path)), `required locked build input missing: ${path}`);
}
if (!existsSync(defaultLicenseRoot)) {
  failures.push("generated offline license resource directory is missing");
}
for (const path of ["cyberkindred.cdx.json", "license-manifest.json", "THIRD-PARTY-NOTICES.txt"]) {
  requireCondition(existsSync(resolve(defaultLicenseRoot, path)), `generated release asset missing: ${path}`);
}

const forbiddenEnvironmentNames = Object.keys(process.env).filter((name) => (
  name.startsWith("CYBERKINDRED_LIVE_")
  || name === "CYBERKINDRED_LIVE_TEST"
  || name === "OPENAI_API_KEY"
));
requireCondition(forbiddenEnvironmentNames.length === 0, `release shell contains forbidden live/secret variables: ${forbiddenEnvironmentNames.join(", ")}`);

const status = run("git", ["status", "--porcelain=v1", "--untracked-files=all"]).trim();
if (status) {
  if (development) {
    warnings.push("working tree is dirty; this development preflight is not release-eligible");
  } else {
    failures.push("release build requires a clean working tree");
  }
}

const result = {
  schemaVersion: 1,
  version: packageJson.version,
  identifier: tauriConfig.identifier,
  releaseEligible: failures.length === 0 && !development,
  development,
  failures,
  warnings,
};
console.log(JSON.stringify(result, null, 2));
if (failures.length > 0) {
  process.exitCode = 1;
}
