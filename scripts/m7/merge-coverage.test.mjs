import assert from "node:assert/strict";
import test from "node:test";

import { summarizeCoverage } from "./merge-coverage.mjs";

function typescript(lines, branches) {
  return { total: { lines, branches } };
}

function rust(lines, branches) {
  return { data: [{ totals: { lines, branches } }] };
}

test("TEST-MAINT-002 aggregates covered and total counts instead of averaging percentages", () => {
  const result = summarizeCoverage(
    typescript({ total: 100, covered: 100 }, { total: 20, covered: 20 }),
    rust({ count: 900, covered: 700 }, { count: 80, covered: 50 }),
    { lines: 80, branches: 70 },
  );
  assert.equal(result.repository.lines.percent, 80);
  assert.equal(result.repository.branches.percent, 70);
  assert.equal(result.status, "passed");
});

test("TEST-MAINT-002 fails below either repository threshold", () => {
  const result = summarizeCoverage(
    typescript({ total: 100, covered: 79 }, { total: 100, covered: 70 }),
    rust({ count: 100, covered: 80 }, { count: 100, covered: 69 }),
    { lines: 80, branches: 70 },
  );
  assert.equal(result.repository.lines.percent, 79.5);
  assert.equal(result.repository.branches.percent, 69.5);
  assert.equal(result.status, "failed");
});

test("TEST-MAINT-002 does not turn a just-below-threshold result into a pass by display rounding", () => {
  const result = summarizeCoverage(
    typescript(
      { total: 100_000, covered: 79_999 },
      { total: 100_000, covered: 69_999 },
    ),
    rust({ count: 1, covered: 1 }, { count: 1, covered: 1 }),
    { lines: 80, branches: 70 },
  );
  assert.equal(result.repository.lines.percent, 80);
  assert.equal(result.repository.branches.percent, 70);
  assert.equal(result.status, "failed");
});

test("TEST-MAINT-002 rejects missing branch data instead of reporting a false pass", () => {
  assert.throws(
    () =>
      summarizeCoverage(
        typescript({ total: 1, covered: 1 }, { total: 1, covered: 1 }),
        rust({ count: 1, covered: 1 }, undefined),
        { lines: 80, branches: 70 },
      ),
    /rust\[0\]\.branches is missing/u,
  );
});

test("TEST-MAINT-002 adds multiple Rust package reports without averaging them", () => {
  const result = summarizeCoverage(
    typescript({ total: 10, covered: 10 }, { total: 10, covered: 10 }),
    [
      rust({ count: 90, covered: 70 }, { count: 90, covered: 60 }),
      rust({ count: 10, covered: 10 }, { count: 10, covered: 10 }),
    ],
    { lines: 80, branches: 70 },
  );
  assert.equal(result.rust.reports, 2);
  assert.equal(result.repository.lines.percent, 81.82);
  assert.equal(result.repository.branches.percent, 72.73);
  assert.equal(result.status, "passed");
});
