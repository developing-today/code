// Participant clamp regression (#2271, Codex P2 on PR #3726): the native
// host clamps participant IDs to i32::MAX (sandbox.rs:
// i32::try_from(participant).unwrap_or(i32::MAX)); PlazaGuest used to wrap
// them with `| 0`, so 4294967296 became participant 0. roc-chat routes
// participant 0 through its host-event handler (input not appended), while
// any real participant appends — so the wrap is observable in the view:
//
//   node smoke/participant-clamp.mjs
import { readFileSync } from "node:fs";
import assert from "node:assert/strict";
import { loadHostedGuest } from "../src/manifest.ts";

const bytes = readFileSync(new URL("../../examples/roc-chat/roc-chat.wasm", import.meta.url));
const enc = new TextEncoder();
const dec = new TextDecoder();

async function viewAfter(participant, event) {
  const guest = await loadHostedGuest({ name: "roc-chat", portability: "portable" }, bytes, 0);
  guest.update(participant, enc.encode(event));
  return dec.decode(guest.view());
}

const atMax = await viewAfter(2147483647, "x");
const aboveMax = await viewAfter(4294967296, "x");
const atZero = await viewAfter(0, "x");
assert.equal(aboveMax, atMax, "participant 4294967296 must behave as i32::MAX (clamped, not wrapped)");
assert.notEqual(aboveMax, atZero, "clamped participant must not collapse into participant 0");
console.log(`participant-clamp: ok (i32::MAX view ${JSON.stringify(atMax)}; participant 0 view ${JSON.stringify(atZero)})`);
