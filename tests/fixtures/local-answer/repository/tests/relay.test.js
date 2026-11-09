import assert from "node:assert/strict";
import test from "node:test";
import { enqueue } from "../src/queue.js";
import { readSettings } from "../src/settings.js";
import { report } from "../src/report.js";

test("deduplication precedes the full check", () => {
  const queue = [{ id: "a", attempts: 2 }];
  assert.equal(enqueue(queue, { id: "a" }, 1), "duplicate");
  assert.equal(enqueue(queue, { id: "b" }, 1), "full");
  assert.equal(queue.length, 1);
});
test("enqueue initializes attempts and report preserves zero", () => {
  const queue = [];
  assert.equal(enqueue(queue, { id: "x", attempts: 99 }), "queued");
  assert.equal(report(queue), "x\t0");
});
test("settings accept defaults and reject fractional capacity", () => {
  assert.deepEqual(readSettings({}), { maxPending: 3 });
  assert.deepEqual(readSettings({ MAX_PENDING: "7" }), { maxPending: 7 });
  assert.throws(() => readSettings({ MAX_PENDING: "1.5" }), /invalid MAX_PENDING/);
});
