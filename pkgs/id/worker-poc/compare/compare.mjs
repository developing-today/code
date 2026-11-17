// #2271 journal-equality harness: native wasmtime host vs Worker isolate.
//
// Compares, per step, (view, records, snapshot) over the scripted
// sequences for roc-counter and tic-tac-toe. The snapshot is compared by
// sha256 of the raw bytes (the native side emits hex; both sides are
// hashed here). Equality must hold after every step, and each side must
// already have proven equality across its own forced cold start
// mid-sequence (asserted inside both runners).
//
// Native reference first (from pkgs/id):
//   WORKER_POC_REFERENCE_OUT=worker-poc/compare/native-reference.jsonl \
//     cargo test --test worker_poc_reference -- --nocapture
// Then:
//   node compare/compare.mjs [path-to-native-reference.jsonl]
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { runIsolateSequences } from "./isolate-runner.mjs";

const nativePath = process.argv[2] ?? new URL("./native-reference.jsonl", import.meta.url).pathname;
const sha256hex = (hexStr) =>
  hexStr === null ? null : createHash("sha256").update(Buffer.from(hexStr, "hex")).digest("hex");

const nativeLines = readFileSync(nativePath, "utf8")
  .trim()
  .split("\n")
  .map((l) => JSON.parse(l));
const isolateLines = await runIsolateSequences();

let failures = 0;
if (nativeLines.length !== isolateLines.length) {
  console.error(`FAIL: step count differs (native ${nativeLines.length}, isolate ${isolateLines.length})`);
  failures++;
}
for (let i = 0; i < Math.min(nativeLines.length, isolateLines.length); i++) {
  const n = nativeLines[i];
  const s = isolateLines[i];
  const diffs = [];
  if (n.world !== s.world || n.step !== s.step || n.event !== s.event)
    diffs.push(`key native=${n.world}/${n.step}/${n.event} isolate=${s.world}/${s.step}/${s.event}`);
  if (n.view !== s.view) diffs.push(`view native=${JSON.stringify(n.view)} isolate=${JSON.stringify(s.view)}`);
  if (n.records !== s.records)
    diffs.push(`records native=${JSON.stringify(n.records)} isolate=${JSON.stringify(s.records)}`);
  if (sha256hex(n.snapshot_hex) !== sha256hex(s.snapshot_hex))
    diffs.push(`snapshot sha256 native=${sha256hex(n.snapshot_hex)} isolate=${sha256hex(s.snapshot_hex)}`);
  if (diffs.length) {
    failures++;
    console.error(`DIVERGENCE at ${n.world} step ${n.step}: ${diffs.join("; ")}`);
  } else {
    console.log(
      `equal ${n.world} step=${n.step} event=${JSON.stringify(n.event)} view=${JSON.stringify(n.view)} records=${JSON.stringify(n.records)} snapshot_sha256=${sha256hex(n.snapshot_hex)}`,
    );
  }
}
if (failures) {
  console.error(`compare: FAIL (${failures} divergence(s))`);
  process.exit(1);
}
console.log(`compare: PASS (${nativeLines.length} steps equal on native and isolate hosts, cold starts included)`);
