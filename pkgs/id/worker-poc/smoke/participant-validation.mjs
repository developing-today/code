// Participant validation regression (#4074): PlazaGuest.update must reject
// value classes the native u64 host can never produce instead of silently
// coercing them. Before the fix, NaN and -Infinity collapsed to participant 0
// (the host-event routing class where roc-chat drops input), and
// negative/fractional participants were delivered/truncated — all diverging
// from native state evolution (sandbox.rs: participant: u64, clamped via
// i32::try_from(participant).unwrap_or(i32::MAX)).
//
//   node smoke/participant-validation.mjs
import { readFileSync } from "node:fs";
import assert from "node:assert/strict";
import { loadHostedGuest } from "../src/manifest.ts";

const bytes = readFileSync(new URL("../../examples/roc-chat/roc-chat.wasm", import.meta.url));
const enc = new TextEncoder();
const dec = new TextDecoder();

async function freshGuest() {
  return loadHostedGuest({ name: "roc-chat", portability: "portable" }, bytes, 0);
}

async function viewAfter(participant, event) {
  const guest = await freshGuest();
  guest.update(participant, enc.encode(event));
  return dec.decode(guest.view());
}

async function rejects(participant, label) {
  const guest = await freshGuest();
  assert.throws(
    () => guest.update(participant, enc.encode("x")),
    /guest participant must be a non-negative integer/,
    `${label} must throw instead of coercing`,
  );
}

// The four native-impossible classes must throw and never reach the guest.
await rejects(-1, "negative participant -1");
await rejects(1.5, "fractional participant 1.5");
await rejects(NaN, "NaN participant");
await rejects(-Infinity, "-Infinity participant");

// Valid classes behave exactly as before: 0 routes as host event (input
// dropped), 1 appends, i32::MAX appends.
const atZero = await viewAfter(0, "x");
const atOne = await viewAfter(1, "x");
const atMax = await viewAfter(2147483647, "x");
assert.equal(atZero, "chat seen=0 last= input=", "participant 0 must keep host-event routing");
assert.equal(atOne, "chat seen=0 last= input=x", "participant 1 must append input");
assert.equal(atMax, "chat seen=0 last= input=x", "participant i32::MAX must append input");

// Top-end clamp from #2271 is preserved: huge IDs and +Infinity behave as
// i32::MAX, never collapsing into participant 0.
const aboveMax = await viewAfter(4294967296, "x");
assert.equal(aboveMax, atMax, "participant 4294967296 must behave as i32::MAX (clamped, not wrapped)");
assert.notEqual(aboveMax, atZero, "clamped participant must not collapse into participant 0");
const atInfinity = await viewAfter(Infinity, "x");
assert.equal(atInfinity, atMax, "+Infinity must clamp to i32::MAX like any ID above the range");

console.log("participant-validation: ok (rejects -1/1.5/NaN/-Infinity; preserves 0/1/i32::MAX clamp)");
