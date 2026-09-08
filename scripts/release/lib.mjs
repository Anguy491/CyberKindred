import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

export const workspaceRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
export const defaultLicenseRoot = resolve(workspaceRoot, "src-tauri/resources/licenses");
export const sbomPath = resolve(defaultLicenseRoot, "cyberkindred.cdx.json");
export const noticesPath = resolve(defaultLicenseRoot, "THIRD-PARTY-NOTICES.txt");
export const licenseManifestPath = resolve(defaultLicenseRoot, "license-manifest.json");

export function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

export function hashFile(path) {
  return sha256(readFileSync(path));
}

export function readJson(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

export function writeJson(path, value) {
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`, "utf8");
}

export function run(command, args, options = {}) {
  const usesWindowsShim =
    process.platform === "win32" && ["npm", "npx", "pnpm", "pnpx"].includes(command);
  if (usesWindowsShim && [command, ...args].some((value) => !/^[A-Za-z0-9_@./:=+-]+$/u.test(String(value)))) {
    throw new Error(`unsafe argument passed to Windows package-manager shim: ${command}`);
  }
  const executable = usesWindowsShim ? (process.env.ComSpec ?? "cmd.exe") : command;
  const executableArgs = usesWindowsShim
    ? [
        "/d",
        "/s",
        "/c",
        [command, ...args].join(" "),
      ]
    : args;
  const result = spawnSync(executable, executableArgs, {
    cwd: workspaceRoot,
    encoding: "utf8",
    env: process.env,
    maxBuffer: 64 * 1024 * 1024,
    shell: false,
    ...options,
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    const details = [result.stdout, result.stderr].filter(Boolean).join("\n").trim();
    throw new Error(`${command} ${args.join(" ")} failed with ${result.status}${details ? `:\n${details}` : ""}`);
  }
  return result.stdout;
}

function cargoPurl(name, version) {
  return `pkg:cargo/${encodeURIComponent(name)}@${encodeURIComponent(version)}`;
}

function normalizeLicenseExpression(value) {
  if (typeof value !== "string" || value.trim().length === 0) {
    return "NOASSERTION";
  }
  return value.trim().replace(/\s*\/\s*/gu, " OR ");
}

function npmPurl(name, version) {
  if (name.startsWith("@")) {
    const separator = name.indexOf("/");
    const scope = name.slice(0, separator);
    const packageName = name.slice(separator + 1);
    return `pkg:npm/${encodeURIComponent(scope)}/${encodeURIComponent(packageName)}@${encodeURIComponent(version)}`;
  }
  return `pkg:npm/${encodeURIComponent(name)}@${encodeURIComponent(version)}`;
}

function parseCargoLock() {
  const text = readFileSync(resolve(workspaceRoot, "Cargo.lock"), "utf8");
  const records = new Map();
  const blocks = text.matchAll(/\[\[package\]\]\s*\r?\n([\s\S]*?)(?=\r?\n\[\[package\]\]|\s*$)/g);
  for (const match of blocks) {
    const body = match[1];
    const name = body.match(/^name = "([^"]+)"$/m)?.[1];
    const version = body.match(/^version = "([^"]+)"$/m)?.[1];
    const source = body.match(/^source = "([^"]+)"$/m)?.[1] ?? "";
    const checksum = body.match(/^checksum = "([a-fA-F0-9]+)"$/m)?.[1];
    if (name && version) {
      records.set(`${name}\0${version}\0${source}`, checksum);
    }
  }
  return records;
}

function componentReferences(packageRecord, ecosystem) {
  const references = [];
  if (typeof packageRecord.repository === "string" && packageRecord.repository.startsWith("https://")) {
    references.push({ type: "vcs", url: packageRecord.repository });
  }
  if (ecosystem === "cargo" && packageRecord.source?.startsWith("registry+")) {
    references.push({
      type: "distribution",
      url: `https://crates.io/crates/${encodeURIComponent(packageRecord.name)}/${encodeURIComponent(packageRecord.version)}`,
    });
  }
  return references;
}

export function collectCargoInventory() {
  const metadata = JSON.parse(run("cargo", [
    "metadata",
    "--locked",
    "--filter-platform",
    "x86_64-pc-windows-msvc",
    "--format-version",
    "1",
  ]));
  const packageById = new Map(metadata.packages.map((entry) => [entry.id, entry]));
  const nodeById = new Map(metadata.resolve.nodes.map((entry) => [entry.id, entry]));
  const rootPackage = metadata.packages.find(
    (entry) => entry.name === "cyberkindred" && metadata.workspace_members.includes(entry.id),
  );
  if (!rootPackage) {
    throw new Error("cargo metadata did not contain the cyberkindred workspace package");
  }

  const included = new Set();
  const queue = [rootPackage.id];
  while (queue.length > 0) {
    const id = queue.shift();
    if (included.has(id)) {
      continue;
    }
    included.add(id);
    const node = nodeById.get(id);
    for (const dependency of node?.deps ?? []) {
      if (dependency.dep_kinds.some((kind) => kind.kind !== "dev")) {
        queue.push(dependency.pkg);
      }
    }
  }

  const lock = parseCargoLock();
  const referenceById = new Map();
  const components = [];
  const packageRecords = [];
  for (const id of [...included].sort()) {
    const entry = packageById.get(id);
    if (!entry) {
      throw new Error(`cargo metadata package missing for ${id}`);
    }
    const reference = cargoPurl(entry.name, entry.version);
    referenceById.set(id, reference);
    const checksum = lock.get(`${entry.name}\0${entry.version}\0${entry.source ?? ""}`);
    const component = {
      type: entry.id === rootPackage.id ? "application" : "library",
      "bom-ref": reference,
      name: entry.name,
      version: entry.version,
      purl: reference,
      scope: "required",
      licenses: [{ expression: normalizeLicenseExpression(entry.license) }],
      properties: [
        { name: "cyberkindred:ecosystem", value: "cargo" },
        {
          name: "cyberkindred:source",
          value: entry.source?.startsWith("registry+") ? "registry" : "workspace",
        },
      ],
    };
    if (checksum) {
      component.hashes = [{ alg: "SHA-256", content: checksum.toLowerCase() }];
    }
    const externalReferences = componentReferences(entry, "cargo");
    if (externalReferences.length > 0) {
      component.externalReferences = externalReferences;
    }
    components.push(component);
    packageRecords.push({
      ecosystem: "cargo",
      name: entry.name,
      version: entry.version,
      license: normalizeLicenseExpression(entry.license),
      source: entry.source?.startsWith("registry+") ? "crates.io" : "workspace",
      packageRoot: dirname(entry.manifest_path),
      purl: reference,
    });
  }

  const dependencies = [];
  for (const id of [...included].sort()) {
    const node = nodeById.get(id);
    const dependsOn = (node?.deps ?? [])
      .filter((dependency) => dependency.dep_kinds.some((kind) => kind.kind !== "dev"))
      .map((dependency) => referenceById.get(dependency.pkg))
      .filter(Boolean)
      .sort();
    dependencies.push({ ref: referenceById.get(id), dependsOn: [...new Set(dependsOn)] });
  }

  return {
    rootRef: referenceById.get(rootPackage.id),
    version: rootPackage.version,
    components: components.sort((left, right) => left["bom-ref"].localeCompare(right["bom-ref"])),
    dependencies,
    packageRecords,
  };
}

function parsePnpmIntegrity() {
  const text = readFileSync(resolve(workspaceRoot, "pnpm-lock.yaml"), "utf8");
  const records = new Map();
  let inPackages = false;
  let key;
  for (const line of text.split(/\r?\n/u)) {
    if (line === "packages:") {
      inPackages = true;
      continue;
    }
    if (inPackages && /^\S/u.test(line) && line.endsWith(":")) {
      break;
    }
    const header = line.match(/^  (?:'([^']+)'|([^:]+)):\s*$/u);
    if (inPackages && header) {
      key = header[1] ?? header[2];
      continue;
    }
    const integrity = line.match(/^    resolution: \{integrity: ([^,}]+)[,}]\s*$/u)?.[1];
    if (inPackages && key && integrity) {
      records.set(key, integrity.replace(/^['"]|['"]$/gu, ""));
    }
  }
  return records;
}

function integrityHash(integrity) {
  const match = integrity?.match(/^(sha256|sha384|sha512)-(.+)$/u);
  if (!match) {
    return undefined;
  }
  return {
    alg: match[1].toUpperCase().replace("SHA", "SHA-"),
    content: Buffer.from(match[2], "base64").toString("hex"),
  };
}

function npmRepository(packageJson) {
  if (typeof packageJson.repository === "string") {
    return packageJson.repository.replace(/^git\+/u, "").replace(/\.git$/u, "");
  }
  if (typeof packageJson.repository?.url === "string") {
    return packageJson.repository.url.replace(/^git\+/u, "").replace(/\.git$/u, "");
  }
  return undefined;
}

export function collectNpmInventory() {
  const roots = JSON.parse(run("pnpm", ["list", "--prod", "--json", "--depth", "Infinity"]));
  const root = roots[0];
  if (!root || root.name !== "cyberkindred") {
    throw new Error("pnpm production tree did not contain the cyberkindred root package");
  }
  const integrityByKey = parsePnpmIntegrity();
  const componentsByRef = new Map();
  const dependencyMap = new Map();
  const packageRecordByRef = new Map();

  function visitDependencies(ownerRef, dependencies) {
    const childRefs = [];
    for (const [name, entry] of Object.entries(dependencies ?? {}).sort(([left], [right]) => left.localeCompare(right))) {
      if (!entry.version || !entry.path) {
        throw new Error(`pnpm production entry ${name} is missing version or installed path`);
      }
      const reference = npmPurl(name, entry.version);
      childRefs.push(reference);
      if (!componentsByRef.has(reference)) {
        const packageJson = readJson(resolve(entry.path, "package.json"));
        const repository = npmRepository(packageJson);
        const component = {
          type: "library",
          "bom-ref": reference,
          name,
          version: entry.version,
          purl: reference,
          scope: "required",
          licenses: [{ expression: normalizeLicenseExpression(packageJson.license) }],
          properties: [
            { name: "cyberkindred:ecosystem", value: "npm" },
            { name: "cyberkindred:source", value: "registry" },
          ],
          externalReferences: [],
        };
        if (entry.resolved?.startsWith("https://")) {
          component.externalReferences.push({ type: "distribution", url: entry.resolved });
        }
        if (repository?.startsWith("https://")) {
          component.externalReferences.push({ type: "vcs", url: repository });
        }
        if (component.externalReferences.length === 0) {
          delete component.externalReferences;
        }
        const hash = integrityHash(integrityByKey.get(`${name}@${entry.version}`));
        if (hash) {
          component.hashes = [hash];
        }
        componentsByRef.set(reference, component);
        packageRecordByRef.set(reference, {
          ecosystem: "npm",
          name,
          version: entry.version,
          license: normalizeLicenseExpression(packageJson.license),
          source: "npm",
          packageRoot: entry.path,
          purl: reference,
        });
        visitDependencies(reference, entry.dependencies);
      }
    }
    dependencyMap.set(ownerRef, [...new Set(childRefs)].sort());
  }

  visitDependencies("__root__", root.dependencies);
  return {
    components: [...componentsByRef.values()].sort((left, right) => left["bom-ref"].localeCompare(right["bom-ref"])),
    dependencies: [...dependencyMap.entries()].map(([ref, dependsOn]) => ({ ref, dependsOn })),
    rootDependencies: dependencyMap.get("__root__") ?? [],
    packageRecords: [...packageRecordByRef.values()].sort((left, right) => left.purl.localeCompare(right.purl)),
  };
}

export function collectLicenseFiles(packageRoot) {
  if (!existsSync(packageRoot) || !statSync(packageRoot).isDirectory()) {
    return [];
  }
  return readdirSync(packageRoot, { withFileTypes: true })
    .filter((entry) => entry.isFile() && /^(?:LICENSE|LICENCE|COPYING|NOTICE)(?:[._-].*)?$/iu.test(entry.name))
    .map((entry) => ({ name: entry.name, content: readFileSync(resolve(packageRoot, entry.name), "utf8") }))
    .sort((left, right) => left.name.localeCompare(right.name));
}

export function containsAbsolutePath(text) {
  return /(?:(?<![A-Za-z0-9])[A-Za-z]:[\\/]|file:\/\/\/)/u.test(text);
}
