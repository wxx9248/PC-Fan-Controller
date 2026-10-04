// Run with: pnpm test
//
// Guards the mock against drifting from the hardware and from the HTTP contract.
// The fan numbers are the measured sweep from 2026-09-29, read from the same
// fixture the daemon's stall tests use, not the model's own output.
import { test } from "node:test";
import assert from "node:assert/strict";
import { once } from "node:events";
import { readFileSync } from "node:fs";
import { createMockServer } from "./server.mjs";
import { rpmForDuty, quantiseRpm } from "./fan-model.mjs";
import { FLOOR_RPM, TACH_QUANTUM_RPM } from "./constants.mjs";
import { parseStatus, STATUS_FIELDS, StatusContractError } from "../src/lib/status.js";

// duty -> RPM, median of three samples per step, measured on the real fan.
const MEASURED = readFileSync(
  new URL("../../daemon/tests/fixtures/fan-sweep-2026-09-29.csv", import.meta.url),
  "utf8",
)
  .split("\n")
  .map((line) => line.trim())
  .filter((line) => line !== "" && !line.startsWith("#"))
  .slice(1) // the column header
  .map((line) => line.split(",").map(Number));

// The model is a least-squares fit (R^2 = 0.997) to measurements that are
// themselves quantised to 30 RPM and carry about one quantum of window jitter,
// so individual points can sit up to two quanta off the line. That is loose
// enough to be fair to the fit and still far tighter than a model with no
// floor, which is 16 quanta out at 0% duty and 24 quanta out at 100%.
const TOLERANCE_RPM = 2 * TACH_QUANTUM_RPM;

test("the sweep fixture was read", () => {
  assert.ok(MEASURED.length >= 10, `only ${MEASURED.length} rows`);
  for (const row of MEASURED) {
    assert.equal(row.length, 2);
    assert.ok(row.every(Number.isInteger), `bad row ${row}`);
  }
});

test("the mock reproduces the measured fan within the fit's residual", () => {
  for (const [duty, measured] of MEASURED) {
    const modelled = quantiseRpm(rpmForDuty(duty));
    assert.ok(
      Math.abs(modelled - measured) <= TOLERANCE_RPM,
      `duty ${duty}%: modelled ${modelled} vs measured ${measured} RPM`,
    );
  }
});

test("a model with no floor would fail the comparison above", () => {
  // Guards the guard: if TOLERANCE_RPM is ever loosened to the point of
  // vacuousness, this fails.
  const old = (duty) => 350 + duty * 24;
  const worst = Math.max(
    ...MEASURED.map(([duty, measured]) => Math.abs(quantiseRpm(old(duty)) - measured)),
  );
  assert.ok(worst > TOLERANCE_RPM, `old model was only ${worst} RPM off`);
});

test("the fan never stops, even at zero duty", () => {
  for (const duty of [0, 1, 10, 20, 31]) {
    assert.equal(rpmForDuty(duty), FLOOR_RPM, `duty ${duty}% must hold the floor`);
  }
});

test("reported RPM is always a multiple of the tach quantum", () => {
  for (let duty = 0; duty <= 100; duty++) {
    assert.equal(quantiseRpm(rpmForDuty(duty)) % TACH_QUANTUM_RPM, 0);
  }
});

test("RPM is monotonic in duty", () => {
  for (let duty = 1; duty <= 100; duty++) {
    assert.ok(rpmForDuty(duty) >= rpmForDuty(duty - 1), `dip at ${duty}%`);
  }
});

test("full duty is near the measured ceiling", () => {
  const top = rpmForDuty(100);
  assert.ok(top > 1900 && top < 2050, `${top} RPM at 100% is implausible`);
});

test("the mock serves the documented HTTP contract", async () => {
  const server = createMockServer(0);
  await once(server, "listening");
  try {
    const base = `http://127.0.0.1:${server.address().port}`;
    const res = await fetch(`${base}/api/status`);
    assert.equal(res.status, 200);
    assert.match(res.headers.get("content-type"), /^application\/json/);
    const body = await res.json();
    assert.deepEqual(Object.keys(body).sort(), [...STATUS_FIELDS].sort(), "exactly the five fields");
    parseStatus(body); // throws on anything the page would reject

    assert.equal((await fetch(`${base}/nope`)).status, 404, "unknown path");
    const post = await fetch(`${base}/api/status`, { method: "POST" });
    assert.equal(post.status, 405, "non-GET on the status path, as the firmware answers");
  } finally {
    server.close();
  }
});

test("a port already in use is reported, not thrown", async () => {
  const first = createMockServer(0);
  await once(first, "listening");
  const second = createMockServer(first.address().port);
  try {
    const [err] = await once(second, "error");
    assert.equal(err.code, "EADDRINUSE");
  } finally {
    first.close();
  }
});

test("the contract rejects what the page must not draw", () => {
  const good = { pwmPercent: 31, rpm: 840, failsafe: false, uptimeSeconds: 7, wifiRssiDbm: null };
  assert.deepEqual(parseStatus(good), good);
  for (const [field, value] of [
    ["pwmPercent", 101],
    ["pwmPercent", 31.5],
    ["rpm", -30],
    ["rpm", "840"],
    ["failsafe", "false"],
    ["uptimeSeconds", -1],
    ["wifiRssiDbm", "-40"],
    ["rpm", undefined],
  ]) {
    assert.throws(
      () => parseStatus({ ...good, [field]: value }),
      StatusContractError,
      `${field}=${JSON.stringify(value)} must be rejected`,
    );
  }
  assert.throws(() => parseStatus([good]), StatusContractError, "an array is not the object");
});
