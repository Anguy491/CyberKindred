import { createHash } from "node:crypto";
import { mkdir, readFile, readdir, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const schemaDirectory = join(repositoryRoot, "docs", "contracts", "schemas");
const baselinePath = join(repositoryRoot, "tests", "contracts", "schema-baseline.json");
const typescriptOutput = join(repositoryRoot, "src", "contracts", "generated.ts");
const rustOutput = join(
  repositoryRoot,
  "src-tauri",
  "src",
  "contracts",
  "generated.rs",
);

const mode = process.argv[2];
if (!["--write", "--check", "--print-baseline"].includes(mode)) {
  throw new Error(
    "Usage: node scripts/generate-contracts.mjs --write|--check|--print-baseline",
  );
}

const allowedKeywords = new Set([
  "$defs",
  "$id",
  "$ref",
  "$schema",
  "additionalProperties",
  "allOf",
  "const",
  "contains",
  "enum",
  "format",
  "if",
  "items",
  "maxItems",
  "maxLength",
  "maximum",
  "minContains",
  "minItems",
  "minLength",
  "minimum",
  "oneOf",
  "pattern",
  "properties",
  "required",
  "then",
  "title",
  "type",
  "uniqueItems",
]);

function canonicalize(value) {
  if (Array.isArray(value)) {
    return value.map(canonicalize);
  }
  if (value !== null && typeof value === "object") {
    return Object.fromEntries(
      Object.keys(value)
        .sort()
        .map((key) => [key, canonicalize(value[key])]),
    );
  }
  return value;
}

function digest(value) {
  return createHash("sha256")
    .update(JSON.stringify(canonicalize(value)))
    .digest("hex");
}

function pascal(value) {
  return value
    .split(/[^A-Za-z0-9]+/u)
    .filter(Boolean)
    .map((part) => `${part[0].toUpperCase()}${part.slice(1)}`)
    .join("");
}

function enumVariant(value) {
  return value
    .split(/[^A-Za-z0-9]+/u)
    .filter(Boolean)
    .map((part) => {
      const normalized = part.toLowerCase();
      return `${normalized[0].toUpperCase()}${normalized.slice(1)}`;
    })
    .join("");
}

function snake(value) {
  return value.replace(/([a-z0-9])([A-Z])/gu, "$1_$2").toLowerCase();
}

function validateSchemaNode(node, location) {
  if (node === null || typeof node !== "object" || Array.isArray(node)) {
    throw new Error(`Expected schema object at ${location}`);
  }
  for (const [key, value] of Object.entries(node)) {
    if (!allowedKeywords.has(key)) {
      throw new Error(`Unsupported schema keyword ${key} at ${location}`);
    }
    if (key === "properties" || key === "$defs") {
      for (const [childName, child] of Object.entries(value)) {
        validateSchemaNode(child, `${location}/${key}/${childName}`);
      }
    } else if (key === "oneOf" || key === "allOf") {
      value.forEach((child, index) =>
        validateSchemaNode(child, `${location}/${key}/${index}`),
      );
    } else if (["if", "then", "items", "contains"].includes(key)) {
      validateSchemaNode(value, `${location}/${key}`);
    }
  }
}

const schemaFiles = (await readdir(schemaDirectory))
  .filter((file) => file.endsWith(".schema.json"))
  .sort();

const records = [];
for (const file of schemaFiles) {
  const schema = JSON.parse(await readFile(join(schemaDirectory, file), "utf8"));
  validateSchemaNode(schema, file);
  if (
    schema.$schema !== "https://json-schema.org/draft/2020-12/schema" ||
    typeof schema.$id !== "string" ||
    typeof schema.title !== "string" ||
    schema.properties?.schemaVersion?.const !== "1.0.0"
  ) {
    throw new Error(`${file} is missing the approved Draft 2020-12 v1 identity`);
  }
  records.push({
    file,
    schema,
    id: schema.$id,
    title: schema.title,
    version: schema.properties.schemaVersion.const,
    sha256: digest(schema),
  });
}

if (records.length !== 6) {
  throw new Error(`Expected exactly six public schemas, found ${records.length}`);
}

const recordById = new Map(records.map((record) => [record.id, record]));
const bundleSha256 = createHash("sha256")
  .update(
    records
      .map((record) => `${record.file}\0${record.sha256}`)
      .join("\n"),
  )
  .digest("hex");

const baseline = {
  schemaBundleSha256: bundleSha256,
  schemas: Object.fromEntries(
    records.map((record) => [
      record.file,
      {
        id: record.id,
        schemaVersion: record.version,
        sha256: record.sha256,
      },
    ]),
  ),
};

if (mode === "--print-baseline") {
  console.log(JSON.stringify(baseline, null, 2));
  process.exit(0);
}

async function verifyOrCreateBaseline() {
  let existing;
  try {
    existing = JSON.parse(await readFile(baselinePath, "utf8"));
  } catch (error) {
    if (error.code !== "ENOENT" || mode !== "--write") {
      throw error;
    }
    await mkdir(dirname(baselinePath), { recursive: true });
    await writeFile(baselinePath, `${JSON.stringify(baseline, null, 2)}\n`);
    return;
  }

  for (const record of records) {
    const previous = existing.schemas?.[record.file];
    if (previous === undefined) {
      throw new Error(`Schema baseline is missing ${record.file}`);
    }
    if (previous.id !== record.id) {
      throw new Error(`Schema $id changed for ${record.file}`);
    }
    if (previous.schemaVersion === record.version && previous.sha256 !== record.sha256) {
      throw new Error(`Unversioned breaking schema drift detected in ${record.file}`);
    }
    if (previous.schemaVersion !== record.version || previous.sha256 !== record.sha256) {
      throw new Error(
        `Schema baseline requires an explicit reviewed update for ${record.file}`,
      );
    }
  }
  if (existing.schemaBundleSha256 !== bundleSha256) {
    throw new Error("Schema bundle digest does not match the reviewed baseline");
  }
}

await verifyOrCreateBaseline();

function resolveReference(reference, currentRecord) {
  if (reference.startsWith("#/$defs/")) {
    const definition = reference.slice("#/$defs/".length);
    const schema = currentRecord.schema.$defs?.[definition];
    if (schema === undefined) {
      throw new Error(`Unknown local reference ${reference} in ${currentRecord.file}`);
    }
    return {
      name: `${currentRecord.title}${pascal(definition)}`,
      record: currentRecord,
      schema,
      variant: pascal(definition),
    };
  }
  const target = recordById.get(reference);
  if (target === undefined) {
    throw new Error(`Unknown or non-local schema reference ${reference}`);
  }
  return {
    name: target.title,
    record: target,
    schema: target.schema,
    variant: target.title,
  };
}

function typescriptType(schema, contextName, currentRecord) {
  if (schema.$ref !== undefined) {
    return resolveReference(schema.$ref, currentRecord).name;
  }
  if (schema.oneOf !== undefined) {
    return schema.oneOf
      .map((item, index) =>
        typescriptType(item, `${contextName}Variant${index + 1}`, currentRecord),
      )
      .join(" | ");
  }
  if (schema.enum !== undefined) {
    return schema.enum.map((value) => JSON.stringify(value)).join(" | ");
  }
  if (schema.const !== undefined) {
    return JSON.stringify(schema.const);
  }
  if (schema.type === "string") return "string";
  if (schema.type === "number" || schema.type === "integer") return "number";
  if (schema.type === "boolean") return "boolean";
  if (schema.type === "null") return "null";
  if (schema.type === "array") {
    return `ReadonlyArray<${typescriptType(schema.items, `${contextName}Item`, currentRecord)}>`;
  }
  if (schema.type === "object") {
    return typescriptObjectBody(schema, contextName, currentRecord);
  }
  throw new Error(`Unsupported TypeScript shape for ${contextName}`);
}

function typescriptObjectBody(schema, contextName, currentRecord) {
  const required = new Set(schema.required ?? []);
  const fields = Object.entries(schema.properties ?? {}).map(([name, child]) => {
    const optional = required.has(name) ? "" : "?";
    return `  readonly ${name}${optional}: ${typescriptType(child, `${contextName}${pascal(name)}`, currentRecord)};`;
  });
  return `{\n${fields.join("\n")}\n}`;
}

const typescriptDeclarations = [];
for (const record of records) {
  typescriptDeclarations.push(
    `export interface ${record.title} ${typescriptObjectBody(record.schema, record.title, record)}`,
  );
  for (const [name, schema] of Object.entries(record.schema.$defs ?? {})) {
    const typeName = `${record.title}${pascal(name)}`;
    typescriptDeclarations.push(
      `export interface ${typeName} ${typescriptObjectBody(schema, typeName, record)}`,
    );
  }
}

const contractNames = records.map((record) => record.title).join(" | ");
const typescriptSource = `// @generated by scripts/generate-contracts.mjs; do not edit.\n// schema-bundle-sha256: ${bundleSha256}\n\nexport const CONTRACT_SCHEMA_BUNDLE_SHA256 = "${bundleSha256}" as const;\n\n${typescriptDeclarations.join("\n\n")}\n\nexport type ContractDocument = ${contractNames};\n`;

const rustDeclarations = [];
const rustDeclarationNames = new Set();

function ensureRustEnum(name, values) {
  if (rustDeclarationNames.has(name)) return;
  rustDeclarationNames.add(name);
  const variants = values.map(
    (value) => `    #[serde(rename = ${JSON.stringify(value)})]\n    ${enumVariant(value)},`,
  );
  rustDeclarations.push(
    `#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]\npub enum ${name} {\n${variants.join("\n")}\n}`,
  );
}

function ensureRustUnion(name, schemas, currentRecord) {
  if (rustDeclarationNames.has(name)) return;
  rustDeclarationNames.add(name);
  const variants = schemas.map((schema, index) => {
    const target = schema.$ref
      ? resolveReference(schema.$ref, currentRecord)
      : undefined;
    const variantName = target?.variant ?? `Variant${index + 1}`;
    const variantType = rustType(
      schema,
      `${name}Variant${index + 1}`,
      currentRecord,
    );
    return `    ${variantName}(${variantType}),`;
  });
  rustDeclarations.push(
    `#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]\n#[serde(untagged)]\npub enum ${name} {\n${variants.join("\n")}\n}`,
  );
}

function rustType(schema, contextName, currentRecord) {
  if (schema.$ref !== undefined) {
    const target = resolveReference(schema.$ref, currentRecord);
    ensureRustObject(target.name, target.schema, target.record);
    return target.name;
  }
  if (schema.oneOf !== undefined) {
    const nonNull = schema.oneOf.filter((item) => item.type !== "null");
    if (nonNull.length + 1 === schema.oneOf.length && nonNull.length === 1) {
      return `Option<${rustType(nonNull[0], contextName, currentRecord)}>`;
    }
    ensureRustUnion(contextName, schema.oneOf, currentRecord);
    return contextName;
  }
  if (schema.enum !== undefined) {
    ensureRustEnum(contextName, schema.enum);
    return contextName;
  }
  if (schema.const !== undefined) {
    return typeof schema.const === "boolean" ? "bool" : "String";
  }
  if (schema.type === "string") return "String";
  if (schema.type === "number") return "f64";
  if (schema.type === "integer") return "u64";
  if (schema.type === "boolean") return "bool";
  if (schema.type === "array") {
    return `Vec<${rustType(schema.items, `${contextName}Item`, currentRecord)}>`;
  }
  if (schema.type === "object") {
    ensureRustObject(contextName, schema, currentRecord);
    return contextName;
  }
  throw new Error(`Unsupported Rust shape for ${contextName}`);
}

const rustReserved = new Set(["type"]);
function ensureRustObject(name, schema, currentRecord) {
  if (rustDeclarationNames.has(name)) return;
  rustDeclarationNames.add(name);
  const required = new Set(schema.required ?? []);
  const fields = Object.entries(schema.properties ?? {}).map(([field, child]) => {
    let type = rustType(child, `${name}${pascal(field)}`, currentRecord);
    if (!required.has(field)) type = `Option<${type}>`;
    const rustName = snake(field);
    const safeName = rustReserved.has(rustName) ? `r#${rustName}` : rustName;
    return `    pub ${safeName}: ${type},`;
  });
  rustDeclarations.push(
    `#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]\n#[serde(rename_all = "camelCase", deny_unknown_fields)]\npub struct ${name} {\n${fields.join("\n")}\n}`,
  );
}

for (const record of records) {
  ensureRustObject(record.title, record.schema, record);
}

const rustSource = `// @generated by scripts/generate-contracts.mjs; do not edit.\n// schema-bundle-sha256: ${bundleSha256}\n#![allow(clippy::module_name_repetitions, clippy::struct_excessive_bools)]\n\nuse serde::{Deserialize, Serialize};\n\npub const CONTRACT_SCHEMA_BUNDLE_SHA256: &str =\n    "${bundleSha256}";\n\n${rustDeclarations.join("\n\n")}\n`;

async function writeOrCheck(path, expected) {
  if (mode === "--write") {
    await mkdir(dirname(path), { recursive: true });
    await writeFile(path, expected);
    return;
  }
  const current = (await readFile(path, "utf8")).replaceAll("\r\n", "\n");
  if (current !== expected) {
    throw new Error(`Generated contract drift detected in ${path}`);
  }
}

await writeOrCheck(typescriptOutput, typescriptSource);
await writeOrCheck(rustOutput, rustSource);
console.log(`Contract generation ${mode === "--write" ? "updated" : "verified"}: ${bundleSha256}`);
