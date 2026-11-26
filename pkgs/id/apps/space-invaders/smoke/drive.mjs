#!/usr/bin/env node
// Playability smoke test for the built space-invaders world module
// (#14939, slice 5 of #14861). Drives the import-free wasm through
// the plaza ABI (pkgs/id/examples/roc-world/ABI.md) with the same
// scripted game as the playthrough expects in program/main.roc:
// step right, fire, five host ticks. Asserts the game progresses —
// the bullet kills an alien on the fifth tick (score 0 -> 10,
// aliens 32 -> 31), nothing traps, the frame JSON shows the new
// score, and a snapshot/restore round-trip reproduces the records
// exactly.
//
// Usage: node drive.mjs [path-to-space-invaders.wasm]
// Default path: ../program/space-invaders.wasm relative to this file.

import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const wasmPath = process.argv[2] ?? join(here, "..", "program", "space-invaders.wasm");

const fail = (msg) => {
  console.error(`smoke: FAIL: ${msg}`);
  process.exit(1);
};
const assert = (cond, msg) => {
  if (!cond) fail(msg);
};

const bytes = await readFile(wasmPath).catch((e) => fail(`cannot read ${wasmPath}: ${e.message}`));
const { instance } = await WebAssembly.instantiate(bytes, {}).catch((e) =>
  fail(`cannot instantiate ${wasmPath}: ${e.message}`),
);
const ex = instance.exports;
for (const name of [
  "memory",
  "plaza_alloc",
  "plaza_free",
  "plaza_init",
  "plaza_update",
  "plaza_view",
  "plaza_records",
  "plaza_snapshot",
  "plaza_restore",
  "plaza_out_len",
]) {
  assert(ex[name] !== undefined, `module is missing export ${name}`);
}

const encoder = new TextEncoder();
const decoder = new TextDecoder();
const mem = () => new Uint8Array(ex.memory.buffer);

// Read the shared output slot: the pointer a projection returned,
// with its byte length left in plaza_out_len.
const readOut = (ptr) => {
  const len = ex.plaza_out_len();
  return decoder.decode(mem().subarray(ptr, ptr + len));
};

// One host event, allocated and freed per the ABI buffer contract.
const send = (model, text) => {
  const data = encoder.encode(text);
  const allocLen = Math.max(data.length, 1);
  const ptr = ex.plaza_alloc(allocLen);
  mem().set(data, ptr);
  try {
    return ex.plaza_update(model, 0, ptr, data.length);
  } finally {
    ex.plaza_free(ptr, allocLen);
  }
};

const tick = "{\"cap\":\"time.tick\",\"interval\":1000,\"now\":1}";

let model = ex.plaza_init(0n);
const records0 = JSON.parse(readOut(ex.plaza_records(model)));
assert(
  records0.score === 0 && records0.aliens_left === 32,
  `opening records unexpected: ${JSON.stringify(records0)}`,
);
const view0 = JSON.parse(readOut(ex.plaza_view(model)));
assert(
  view0.kind === "screen" && view0.width === 40 && view0.height === 22,
  "opening view is not the 40x22 screen document",
);
assert(view0.rows[0].startsWith("SCORE 0"), `opening HUD unexpected: ${view0.rows[0]}`);

model = send(model, "d");
model = send(model, " ");
for (let i = 0; i < 5; i++) model = send(model, tick);

const records1Text = readOut(ex.plaza_records(model));
const records1 = JSON.parse(records1Text);
assert(records1.score === 10, `expected score 10 after the scripted kill, got ${records1.score} (${records1Text})`);
assert(records1.aliens_left === 31, `expected 31 aliens left, got ${records1.aliens_left}`);
assert(
  records1.lives === 3 && records1.phase === "playing" && records1.ticks === 5,
  `unexpected game state: ${records1Text}`,
);
const view1 = JSON.parse(readOut(ex.plaza_view(model)));
assert(view1.rows[0].startsWith("SCORE 10"), `HUD did not update: ${view1.rows[0]}`);

// Snapshot/restore round-trip: the restored model publishes the
// same records as the live one.
const snapBytes = encoder.encode(readOut(ex.plaza_snapshot(model)));
const snapAlloc = Math.max(snapBytes.length, 1);
const snapPtr = ex.plaza_alloc(snapAlloc);
mem().set(snapBytes, snapPtr);
const restored = ex.plaza_restore(snapPtr, snapBytes.length);
ex.plaza_free(snapPtr, snapAlloc);
const records2Text = readOut(ex.plaza_records(restored));
assert(records2Text === records1Text, `restored records differ: ${records2Text} vs ${records1Text}`);

console.log(`SMOKE_OK ${records1Text}`);
