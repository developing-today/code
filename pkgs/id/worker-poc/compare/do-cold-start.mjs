// DO-level forced cold start for #2271, under `wrangler dev` (local
// workerd, SQLite-backed Durable Object journal).
//
// Runs the roc-counter sequence against the real Worker/DO host:
//   GET /state            -> step 0
//   POST /step inc x2     -> steps 1, 2
//   POST /cold-start      -> in-memory guest dropped; GET /state must
//                            restore step 2 from the SQLite journal
//   wrangler dev restart  -> a genuinely fresh DO instance; GET /state
//                            must again restore step 2 from the journal
//   POST /step inc        -> step 3; its snapshotHash must equal the
//                            native reference hash for counter step 3
//                            (compare/native-reference.jsonl).
//
// Usage: node compare/do-cold-start.mjs [path-to-native-reference.jsonl]
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";

const PORT = 8791;
const BASE = `http://127.0.0.1:${PORT}`;
const WORLD = `do-cold-start-${process.pid}`;
const url = (path) => `${BASE}${path}?world=${WORLD}`;

const nativePath = process.argv[2] ?? new URL("./native-reference.jsonl", import.meta.url).pathname;
const native = readFileSync(nativePath, "utf8").trim().split("\n").map((l) => JSON.parse(l));
const nativeStep3 = native.find((l) => l.world === "roc-counter" && l.step === 3);
const expectedHash = createHash("sha256").update(Buffer.from(nativeStep3.snapshot_hex, "hex")).digest("hex");
console.log(`expected counter step-3 snapshot sha256 (native): ${expectedHash}`);

let child = null;
function startDev() {
  child = spawn("wrangler", ["dev", "--port", String(PORT), "--local"], {
    cwd: new URL("..", import.meta.url).pathname,
    stdio: ["ignore", "pipe", "pipe"],
    detached: true, // own process group, so stopDev can reap workerd too
  });
  child.stderr.on("data", (d) => process.stderr.write(`[wrangler] ${d}`));
}
async function waitReady(tries = 60) {
  for (let i = 0; i < tries; i++) {
    try {
      const res = await fetch(url("/state"));
      if (res.ok) return await res.json();
    } catch {}
    await new Promise((r) => setTimeout(r, 2000));
  }
  throw new Error("wrangler dev did not become ready");
}
async function stopDev() {
  if (!child) return;
  const pid = child.pid;
  const done = new Promise((r) => child.once("exit", r));
  try { process.kill(-pid, "SIGTERM"); } catch { child.kill("SIGTERM"); }
  await Promise.race([
    done,
    new Promise((r) => setTimeout(r, 5000)).then(() => {
      try { process.kill(-pid, "SIGKILL"); } catch {}
    }),
  ]);
  child = null;
  // Wait until the port is actually free, so the next dev instance binds
  // instead of queueing behind a dying workerd.
  for (let i = 0; i < 30; i++) {
    try {
      await fetch(url("/state"));
    } catch {
      return;
    }
    await new Promise((r) => setTimeout(r, 1000));
  }
}
const step = async () => {
  const res = await fetch(url("/step"), {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ participant: 1, event: "inc" }),
  });
  if (!res.ok) throw new Error(`POST /step -> ${res.status}`);
  return res.json();
};
const state = async () => {
  const res = await fetch(url("/state"));
  if (!res.ok) throw new Error(`GET /state -> ${res.status}`);
  return res.json();
};
function assertEq(actual, expected, what) {
  if (actual !== expected) throw new Error(`FAIL: ${what}: got ${JSON.stringify(actual)}, want ${JSON.stringify(expected)}`);
  console.log(`ok: ${what} = ${JSON.stringify(actual)}`);
}

try {
  startDev();
  const s0 = await waitReady();
  assertEq(s0.step, 0, "initial step");
  const s1 = await step();
  const s2 = await step();
  assertEq(s2.step, 2, "step after inc x2");

  const cold = await (await fetch(url("/cold-start"), { method: "POST" })).json();
  assertEq(cold.journaledStep, 2, "journaled step at /cold-start");
  const restored = await state();
  assertEq(restored.step, 2, "step restored after /cold-start");
  assertEq(restored.snapshotHash, s2.snapshotHash, "snapshot hash restored after /cold-start");
  assertEq(restored.view, s2.view, "view restored after /cold-start");

  await stopDev();
  startDev();
  const revived = await waitReady();
  assertEq(revived.step, 2, "step restored after wrangler restart (fresh DO instance)");
  assertEq(revived.snapshotHash, s2.snapshotHash, "snapshot hash restored after wrangler restart");

  const s3 = await step();
  assertEq(s3.step, 3, "final step");
  assertEq(s3.snapshotHash, expectedHash, "DO step-3 snapshot hash equals native reference");
  console.log("do-cold-start: PASS");
} finally {
  await stopDev();
}
