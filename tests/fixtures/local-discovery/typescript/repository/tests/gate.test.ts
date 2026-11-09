import assert from "node:assert/strict";
import { BurstGate } from "../src/gate.ts";

const gate = new BurstGate(1);
assert.equal(gate.accept("a"), true);
assert.equal(gate.accept("a"), false);
assert.equal(gate.accept("b"), true);
gate.reset("a");
assert.equal(gate.accept("a"), true);
