// Counter pipeline smoke for the Worker isolate host (#2271, acceptance 1 + 4
// + a guest-level preview of 3). Runs the prebuilt, unmodified counter.wasm
// through PlazaGuest under plain Node (>=22 strips the .ts types natively):
//
//   node smoke/counter-smoke.mjs
//
// Sequence: seed 0, then inc x3 from participant 1. After every step it
// prints (view, records, snapshot sha256). It then forces a cold start at
// the guest level — fresh instance, plaza_restore from the step-2 snapshot —
// applies the remaining inc, and asserts the final snapshot hash equals the
// uninterrupted run's. Finally it asserts both hosted-loader refusals name
// their reasons (portability=native; declared memory over the 64MB ceiling).
import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { PlazaGuest } from "../src/guest.ts";
import { assertHostable, loadHostedGuest } from "../src/manifest.ts";

const wasmPath = new URL("../../examples/roc-counter/counter.wasm", import.meta.url);
const bytes = readFileSync(wasmPath);
const enc = new TextEncoder();
const dec = new TextDecoder();
const hash = (b) => createHash("sha256").update(b).digest("hex");

function stepLine(label, guest, step) {
  const view = dec.decode(guest.view());
  const records = guest.records();
  const snapshot = guest.snapshot();
  console.log(
    `${label} step=${step} view=${JSON.stringify(view)} records=${JSON.stringify(records ? dec.decode(records) : null)} snapshot_sha256=${snapshot ? hash(snapshot) : null}`,
  );
  return snapshot;
}

// Uninterrupted run.
const guest = await loadHostedGuest({ name: "roc-counter", portability: "portable" }, bytes, 0);
const snapshots = [];
stepLine("run", guest, 0);
for (let i = 1; i <= 3; i++) {
  guest.update(1, enc.encode("inc"));
  snapshots[i] = stepLine("run", guest, i);
}

// Guest-level cold start: restore the step-2 snapshot into a fresh instance.
const revived = await PlazaGuest.instantiate(bytes, 0);
revived.restore(snapshots[2]);
revived.update(1, enc.encode("inc"));
const revivedSnap = stepLine("cold", revived, 3);
if (hash(revivedSnap) !== hash(snapshots[3])) {
  console.error("FAIL: cold-start snapshot hash differs from the uninterrupted run");
  process.exit(1);
}
console.log("cold-start equality: OK (step-3 snapshot hash matches)");

// Hosted-loader refusals (acceptance 4).
for (const [manifest, needle] of [
  [{ name: "native-world", portability: "native" }, "portability=native"],
  [{ name: "fat-world", wasm_memory_bytes: 128 * 1024 * 1024 }, "exceeds hosted ceiling"],
]) {
  try {
    assertHostable(manifest);
    console.error(`FAIL: manifest ${manifest.name} was not refused`);
    process.exit(1);
  } catch (err) {
    if (!String(err.message).includes(needle)) {
      console.error(`FAIL: refusal for ${manifest.name} did not name '${needle}': ${err.message}`);
      process.exit(1);
    }
    console.log(`refusal OK: ${err.message}`);
  }
}
console.log("counter smoke: PASS");
