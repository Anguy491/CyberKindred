import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import {
  containsAbsolutePath,
  hashFile,
  licenseManifestPath,
  noticesPath,
  readJson,
  sbomPath,
  workspaceRoot,
} from "./lib.mjs";

const failures = [];
const bomText = readFileSync(sbomPath, "utf8");
const notices = readFileSync(noticesPath, "utf8");
const licenseManifestText = readFileSync(licenseManifestPath, "utf8");
const bom = JSON.parse(bomText);
const licenseManifest = JSON.parse(licenseManifestText);
const packageJson = readJson(resolve(workspaceRoot, "package.json"));

function requireCondition(condition, message) {
  if (!condition) {
    failures.push(message);
  }
}

requireCondition(bom.bomFormat === "CycloneDX", "SBOM bomFormat must be CycloneDX");
requireCondition(bom.specVersion === "1.6", "SBOM specVersion must be 1.6");
requireCondition(bom.version === 1, "SBOM document version must be 1");
requireCondition(bom.metadata?.component?.version === packageJson.version, "SBOM app version differs from package version");
requireCondition(Array.isArray(bom.components) && bom.components.length > 10, "SBOM component inventory is unexpectedly small");
requireCondition(Array.isArray(bom.dependencies) && bom.dependencies.length > 10, "SBOM dependency graph is unexpectedly small");
requireCondition(!containsAbsolutePath(bomText), "SBOM contains an absolute local path");
requireCondition(!containsAbsolutePath(notices), "notices contain an absolute local path");
requireCondition(!containsAbsolutePath(licenseManifestText), "license manifest contains an absolute local path");
requireCondition(licenseManifest.appVersion === packageJson.version, "license manifest app version differs from package version");

const references = new Set([bom.metadata.component["bom-ref"]]);
for (const component of bom.components ?? []) {
  requireCondition(!references.has(component["bom-ref"]), `duplicate SBOM reference: ${component["bom-ref"]}`);
  references.add(component["bom-ref"]);
  requireCondition(component.purl === component["bom-ref"], `component purl/ref mismatch: ${component.name}`);
  requireCondition(component.licenses?.[0]?.expression, `component license missing: ${component.name}`);
  requireCondition(!/(?:AGPL|SSPL|BUSL|Commons Clause|NOASSERTION)/iu.test(component.licenses?.[0]?.expression ?? ""), `forbidden or unknown license: ${component.name}`);
  const isWorkspaceComponent = component.properties?.some(
    (property) => property.name === "cyberkindred:source" && property.value === "workspace",
  );
  requireCondition(isWorkspaceComponent || component.hashes?.length > 0, `locked checksum missing: ${component["bom-ref"]}`);
}
for (const dependency of bom.dependencies ?? []) {
  requireCondition(references.has(dependency.ref), `dependency source ref is absent: ${dependency.ref}`);
  for (const target of dependency.dependsOn ?? []) {
    requireCondition(references.has(target), `dependency target ref is absent: ${target}`);
  }
}

for (const font of licenseManifest.fonts ?? []) {
  const actual = hashFile(resolve(workspaceRoot, "src/assets/fonts", font.file));
  requireCondition(actual === font.sha256, `font hash differs from release manifest: ${font.file}`);
  requireCondition(notices.includes(font.file) && notices.includes(font.licenseFile), `font notice missing: ${font.file}`);
}

const packageKeys = new Set();
for (const entry of licenseManifest.packages ?? []) {
  const key = `${entry.ecosystem}:${entry.name}@${entry.version}`;
  requireCondition(!packageKeys.has(key), `duplicate license entry: ${key}`);
  packageKeys.add(key);
  requireCondition(entry.license && entry.license !== "NOASSERTION", `unknown package license: ${key}`);
  requireCondition(notices.includes(`${entry.ecosystem}: ${entry.name}@${entry.version}`), `notice entry missing: ${key}`);
}

if (failures.length > 0) {
  console.error(failures.map((failure) => `- ${failure}`).join("\n"));
  process.exitCode = 1;
} else {
  console.log(`Release assets verified: ${bom.components.length + 1} components, ${licenseManifest.packages.length} dependency notices, ${licenseManifest.fonts.length} fonts.`);
}
