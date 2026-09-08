import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const workspaceRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");

function requireInteger(value, label) {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new Error(`${label} must be a non-negative integer`);
  }
  return value;
}

function requireMetric(metric, label) {
  if (metric === null || typeof metric !== "object") {
    throw new Error(`${label} is missing`);
  }
  const total = requireInteger(metric.total ?? metric.count, `${label}.total`);
  const covered = requireInteger(metric.covered, `${label}.covered`);
  if (total === 0) {
    throw new Error(`${label}.total must be greater than zero`);
  }
  if (covered > total) {
    throw new Error(`${label}.covered cannot exceed total`);
  }
  return { total, covered };
}

function percentage(covered, total) {
  return Number(((covered * 100) / total).toFixed(2));
}

function meetsThreshold(metric, threshold) {
  return metric.covered * 100 >= metric.total * threshold;
}

export function summarizeCoverage(typeScriptSummary, rustExport, thresholds) {
  const typeScript = {
    lines: requireMetric(typeScriptSummary?.total?.lines, "typescript.lines"),
    branches: requireMetric(typeScriptSummary?.total?.branches, "typescript.branches"),
  };
  if (!Array.isArray(rustExport?.data) || rustExport.data.length !== 1) {
    throw new Error("rust.data must contain exactly one llvm-cov export");
  }
  const rustTotals = rustExport.data[0]?.totals;
  const rust = {
    lines: requireMetric(rustTotals?.lines, "rust.lines"),
    branches: requireMetric(rustTotals?.branches, "rust.branches"),
  };
  const minimumLines = Number(thresholds.lines);
  const minimumBranches = Number(thresholds.branches);
  for (const [label, value] of [
    ["minimum lines", minimumLines],
    ["minimum branches", minimumBranches],
  ]) {
    if (!Number.isFinite(value) || value < 0 || value > 100) {
      throw new Error(`${label} must be from 0 to 100`);
    }
  }

  const repository = {
    lines: {
      covered: typeScript.lines.covered + rust.lines.covered,
      total: typeScript.lines.total + rust.lines.total,
    },
    branches: {
      covered: typeScript.branches.covered + rust.branches.covered,
      total: typeScript.branches.total + rust.branches.total,
    },
  };
  for (const language of [typeScript, rust, repository]) {
    language.lines.percent = percentage(language.lines.covered, language.lines.total);
    language.branches.percent = percentage(language.branches.covered, language.branches.total);
  }
  const passed =
    meetsThreshold(repository.lines, minimumLines) &&
    meetsThreshold(repository.branches, minimumBranches);
  return {
    schemaVersion: 1,
    testId: "TEST-MAINT-002",
    thresholds: { lines: minimumLines, branches: minimumBranches },
    typeScript,
    rust,
    repository,
    status: passed ? "passed" : "failed",
  };
}

function argument(name, fallback) {
  const index = process.argv.indexOf(name);
  if (index === -1) return fallback;
  const value = process.argv[index + 1];
  if (value === undefined || value.startsWith("--")) {
    throw new Error(`${name} requires a value`);
  }
  return value;
}

function requiredArgument(name) {
  const value = argument(name);
  if (value === undefined) throw new Error(`${name} is required`);
  return value;
}

function readJson(file) {
  return JSON.parse(readFileSync(path.resolve(workspaceRoot, file), "utf8"));
}

function safeOutputPath(value) {
  const resolved = path.resolve(workspaceRoot, value);
  const allowedRoot = path.resolve(workspaceRoot, "target", "m7-evidence");
  if (resolved !== allowedRoot && !resolved.startsWith(`${allowedRoot}${path.sep}`)) {
    throw new Error("--output must stay below target/m7-evidence");
  }
  return resolved;
}

function main() {
  const typeScriptPath = requiredArgument("--typescript");
  const rustPath = requiredArgument("--rust");
  const result = summarizeCoverage(readJson(typeScriptPath), readJson(rustPath), {
    lines: argument("--min-lines", "80"),
    branches: argument("--min-branches", "70"),
  });
  const encoded = `${JSON.stringify(result, null, 2)}\n`;
  const output = argument("--output");
  if (output !== undefined) {
    const resolvedOutput = safeOutputPath(output);
    mkdirSync(path.dirname(resolvedOutput), { recursive: true });
    writeFileSync(resolvedOutput, encoded, "utf8");
  }
  process.stdout.write(encoded);
  if (result.status !== "passed") process.exitCode = 1;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    main();
  } catch (error) {
    process.stderr.write(`coverage gate failed: ${error instanceof Error ? error.message : "unknown error"}\n`);
    process.exitCode = 2;
  }
}
