import { mkdirSync, readFileSync } from "node:fs";
import { resolve } from "node:path";

import {
  collectCargoInventory,
  collectLicenseFiles,
  collectNpmInventory,
  defaultLicenseRoot,
  licenseManifestPath,
  noticesPath,
  readJson,
  sbomPath,
  sha256,
  workspaceRoot,
  writeJson,
} from "./lib.mjs";

mkdirSync(defaultLicenseRoot, { recursive: true });

const cargo = collectCargoInventory();
const npm = collectNpmInventory();
const dependencyByRef = new Map(cargo.dependencies.map((entry) => [entry.ref, entry.dependsOn]));
for (const entry of npm.dependencies) {
  if (entry.ref !== "__root__") {
    dependencyByRef.set(entry.ref, entry.dependsOn);
  }
}
dependencyByRef.set(cargo.rootRef, [
  ...(dependencyByRef.get(cargo.rootRef) ?? []),
  ...npm.rootDependencies,
].filter((value, index, values) => values.indexOf(value) === index).sort());

const rootComponent = cargo.components.find((component) => component["bom-ref"] === cargo.rootRef);
const components = [
  ...cargo.components.filter((component) => component["bom-ref"] !== cargo.rootRef),
  ...npm.components,
].sort((left, right) => left["bom-ref"].localeCompare(right["bom-ref"]));
const bom = {
  $schema: "https://cyclonedx.org/schema/bom-1.6.schema.json",
  bomFormat: "CycloneDX",
  specVersion: "1.6",
  version: 1,
  metadata: { component: rootComponent },
  components,
  dependencies: [...dependencyByRef.entries()]
    .map(([ref, dependsOn]) => ({ ref, dependsOn }))
    .sort((left, right) => left.ref.localeCompare(right.ref)),
};
writeJson(sbomPath, bom);

const fontManifest = readJson(resolve(workspaceRoot, "src/assets/fonts/MANIFEST.json"));
const packageRecords = [...cargo.packageRecords, ...npm.packageRecords]
  .sort((left, right) => left.purl.localeCompare(right.purl));
const textsByHash = new Map();
const manifestPackages = packageRecords.map((record) => {
  const texts = collectLicenseFiles(record.packageRoot).map((file) => {
    const normalized = file.content.replace(/\r\n/gu, "\n")
      .split("\n")
      .map((line) => line.trimEnd())
      .join("\n")
      .trimEnd();
    const digest = sha256(normalized);
    if (!textsByHash.has(digest)) {
      textsByHash.set(digest, normalized);
    }
    return { file: file.name, sha256: digest };
  });
  return {
    ecosystem: record.ecosystem,
    name: record.name,
    version: record.version,
    license: record.license,
    source: record.source,
    purl: record.purl,
    licenseTexts: texts,
  };
});

for (const font of fontManifest.files) {
  const path = resolve(workspaceRoot, "src/assets/fonts", font.licenseFile);
  const normalized = readFileSync(path, "utf8").replace(/\r\n/gu, "\n")
    .split("\n")
    .map((line) => line.trimEnd())
    .join("\n")
    .trimEnd();
  const digest = sha256(normalized);
  if (!textsByHash.has(digest)) {
    textsByHash.set(digest, normalized);
  }
}

const licenseManifest = {
  schemaVersion: 1,
  appVersion: cargo.version,
  generatedFrom: ["Cargo.lock", "pnpm-lock.yaml", "src/assets/fonts/MANIFEST.json"],
  packages: manifestPackages,
  fonts: fontManifest.files.map((font) => ({
    file: font.file,
    sha256: font.sha256.toLowerCase(),
    copyright: font.copyright,
    license: font.license,
    licenseFile: font.licenseFile,
  })),
};
writeJson(licenseManifestPath, licenseManifest);

const noticeLines = [
  "CYBERKINDRED THIRD-PARTY NOTICES",
  `Version ${cargo.version}`,
  "",
  "This file is generated from the locked Windows x64 production/build dependency graph.",
  "Package source and checksum details are recorded in cyberkindred.cdx.json.",
  "",
  "DEPENDENCIES",
];
for (const entry of manifestPackages) {
  const texts = entry.licenseTexts.length > 0
    ? entry.licenseTexts.map((text) => `${text.file}#sha256:${text.sha256}`).join(", ")
    : "no packaged license text discovered; SPDX expression retained for review";
  noticeLines.push(`${entry.ecosystem}: ${entry.name}@${entry.version} | ${entry.license} | ${entry.source} | ${texts}`);
}
noticeLines.push("", "BUNDLED FONTS");
for (const font of licenseManifest.fonts) {
  noticeLines.push(`${font.file} | ${font.license} | ${font.copyright} | ${font.licenseFile}`);
}
noticeLines.push("", "LICENSE AND NOTICE TEXTS");
for (const [digest, content] of [...textsByHash.entries()].sort(([left], [right]) => left.localeCompare(right))) {
  noticeLines.push("", `--- sha256:${digest} ---`, content);
}
noticeLines.push("");
await import("node:fs").then(({ writeFileSync }) => {
  writeFileSync(noticesPath, noticeLines.join("\n"), "utf8");
});

console.log(`Release assets generated: ${components.length + 1} CycloneDX components, ${manifestPackages.length} dependency records, ${textsByHash.size} unique notice texts.`);
