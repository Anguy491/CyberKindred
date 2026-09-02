import { readFileSync } from "node:fs";
import { join } from "node:path";

import Ajv2020, { type ValidateFunction } from "ajv/dist/2020.js";
import addFormats from "ajv-formats";

import type { ContractDocument } from "../../src/contracts";
import cases from "./cases.json";

const schemaDirectory = join(process.cwd(), "docs", "contracts", "schemas");
const exampleDirectory = join(process.cwd(), "docs", "contracts", "examples");
const schemaFiles = [
  "memory-record.schema.json",
  "playback-event.schema.json",
  "playback-state.schema.json",
  "program-plan.schema.json",
  "provider-error.schema.json",
  "schedule-rule.schema.json",
] as const;

function readJson(path: string): unknown {
  return JSON.parse(readFileSync(path, "utf8"));
}

function createRegistry() {
  const ajv = new Ajv2020({
    allErrors: true,
    strict: true,
    validateFormats: true,
  });
  addFormats(ajv, ["date-time", "uuid"]);
  for (const file of schemaFiles) {
    ajv.addSchema(readJson(join(schemaDirectory, file)));
  }
  return ajv;
}

function validatorFor(
  ajv: Ajv2020,
  schema: string,
): ValidateFunction<ContractDocument> {
  const id = `cyberkindred://contracts/v1/${schema}.schema.json`;
  const validator = ajv.getSchema<ContractDocument>(id);
  if (validator === undefined) {
    throw new Error(`Missing local validator for ${schema}`);
  }
  return validator;
}

describe("[NFR-MAINT-001][NFR-MAINT-003][TEST-MAINT-001] public schemas", () => {
  const ajv = createRegistry();

  it.each(cases)("validates $example as expected", ({ schema, example, expected }) => {
    const document = readJson(join(exampleDirectory, example));
    const validator = validatorFor(ajv, schema);

    expect(validator(document), JSON.stringify(validator.errors)).toBe(expected);
    if (expected && validator(document)) {
      const typedDocument: ContractDocument = document;
      expect(structuredClone(typedDocument)).toEqual(document);
    }
  });

  it("resolves the playback event reference from the in-memory registry", () => {
    const event = readJson(join(exampleDirectory, "playback-event.valid.json"));
    expect(validatorFor(ajv, "playback-event")(event)).toBe(true);
  });

  it.each([
    ["memory-record", "memory-record.valid.json", "memoryId", "not-a-uuid"],
    ["memory-record", "memory-record.valid.json", "createdAt", "not-a-date"],
    ["playback-state", "playback-state.valid.json", "schemaVersion", "2.0.0"],
  ])("rejects a format/version mutation for %s", (schema, example, key, value) => {
    const document = readJson(join(exampleDirectory, example));
    if (document === null || typeof document !== "object") {
      throw new Error("Contract fixture root must be an object");
    }
    const mutation = { ...document, [key]: value };
    expect(validatorFor(ajv, schema)(mutation)).toBe(false);
  });

  it("rejects missing required and unknown properties", () => {
    const document = readJson(join(exampleDirectory, "schedule-rule.valid.json"));
    if (document === null || typeof document !== "object") {
      throw new Error("Contract fixture root must be an object");
    }
    const { scheduleId: _removed, ...missingRequired } = document;
    expect(validatorFor(ajv, "schedule-rule")(missingRequired)).toBe(false);
    expect(
      validatorFor(ajv, "schedule-rule")({ ...document, unexpected: true }),
    ).toBe(false);
  });
});
