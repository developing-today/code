// Local Wrangler D1 journal-fit smoke. Run from worker-poc:
//   node d1-journal-poc/smoke.mjs
// Requires Node >=22 and `wrangler` on PATH. Uses only a unique temporary
// persistence directory; it does not use or configure the main worker-poc D1.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdtemp, rm } from "node:fs/promises";
import { createServer } from "node:net";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as delay } from "node:timers/promises";

const pocDir = path.dirname(fileURLToPath(import.meta.url));
const workerPocDir = path.resolve(pocDir, "..");
const configPath = path.join(pocDir, "wrangler.toml");
const worldId = "d1-journal-poc-world";
const maxCapturedLogChars = 60_000;

let child;
let childClosed;
let childFailure;
let stdout = "";
let stderr = "";
let stopPromise;
let stateDir;
let interruptedBy;
const interrupt = new AbortController();

function appendLog(existing, chunk) {
  return (existing + chunk.toString()).slice(-maxCapturedLogChars);
}

function captureLogs(processHandle) {
  processHandle.stdout.on("data", (chunk) => {
    stdout = appendLog(stdout, chunk);
  });
  processHandle.stderr.on("data", (chunk) => {
    stderr = appendLog(stderr, chunk);
  });
  processHandle.on("error", (error) => {
    childFailure = error;
  });
  childClosed = new Promise((resolve) => {
    processHandle.once("close", (code, signal) => resolve({ code, signal }));
  });
}

function signalProcessGroup(signal) {
  if (!child || child.pid === undefined) return;
  try {
    if (process.platform !== "win32") process.kill(-child.pid, signal);
    else child.kill(signal);
  } catch (error) {
    if (error?.code !== "ESRCH") throw error;
  }
}

async function stopServer() {
  if (!child || !childClosed) return;
  if (stopPromise) return stopPromise;
  stopPromise = (async () => {
    signalProcessGroup("SIGTERM");
    let timeout;
    const graceful = await Promise.race([
      childClosed.then(() => true),
      new Promise((resolve) => {
        timeout = setTimeout(() => resolve(false), 5_000);
      }),
    ]);
    clearTimeout(timeout);
    if (!graceful) {
      signalProcessGroup("SIGKILL");
      await childClosed;
    }
  })();
  return stopPromise;
}

async function freePort() {
  const server = createServer();
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const address = server.address();
  assert(address && typeof address === "object");
  const { port } = address;
  await new Promise((resolve, reject) => {
    server.close((error) => (error ? reject(error) : resolve()));
  });
  return port;
}

function abortSignal() {
  return AbortSignal.any([interrupt.signal, AbortSignal.timeout(5_000)]);
}

function serverLogs() {
  return [stdout && `Wrangler stdout:\n${stdout}`, stderr && `Wrangler stderr:\n${stderr}`]
    .filter(Boolean)
    .join("\n");
}

async function waitForServer(baseUrl) {
  const deadline = Date.now() + 45_000;
  while (Date.now() < deadline) {
    if (childFailure) throw childFailure;
    if (child.exitCode !== null || child.signalCode !== null) {
      throw new Error(`Wrangler exited before becoming ready (${serverLogs()})`);
    }
    try {
      const response = await fetch(`${baseUrl}/health`, { signal: abortSignal() });
      if (response.ok && (await response.text()) === "ok") return;
    } catch (error) {
      if (interrupt.signal.aborted) throw error;
    }
    await delay(200);
  }
  throw new Error(`Timed out waiting for local Wrangler (${serverLogs()})`);
}

async function responseError(response) {
  const text = await response.text();
  return `${response.status} ${response.statusText}: ${text}`;
}

async function append(baseUrl, bytes) {
  const response = await fetch(`${baseUrl}/append`, {
    method: "POST",
    headers: { "content-type": "application/x-ndjson" },
    body: bytes,
    signal: abortSignal(),
  });
  if (!response.ok) assert.fail(`append failed: ${await responseError(response)}`);
  return response.json();
}

async function readJsonl(response, label) {
  if (!response.ok) assert.fail(`${label} failed: ${await responseError(response)}`);
  const bytes = Buffer.from(await response.arrayBuffer());
  const first = Number(response.headers.get("x-journal-first-ordinal"));
  const last = Number(response.headers.get("x-journal-last-ordinal"));
  const count = Number(response.headers.get("x-journal-entry-count"));
  const cursor = Number(response.headers.get("x-journal-stream-cursor"));
  assert(Number.isSafeInteger(first) && first >= 0, `${label}: missing first ordinal`);
  assert(Number.isSafeInteger(last) && last >= 0, `${label}: missing last ordinal`);
  assert(Number.isSafeInteger(count) && count >= 0, `${label}: missing entry count`);
  assert.equal(cursor, last, `${label}: stream cursor should name the last returned ordinal`);
  if (count > 0) assert.equal(last - first + 1, count, `${label}: ordinal range contains a gap`);
  return { bytes, first, last, count, cursor };
}

async function main() {
  stateDir = await mkdtemp(path.join(os.tmpdir(), "id-d1-journal-poc-"));
  const persistDir = path.join(stateDir, "wrangler-state");
  const port = await freePort();
  const baseUrl = `http://127.0.0.1:${port}`;
  child = spawn(
    "wrangler",
    [
      "dev",
      "--local",
      "--ip",
      "127.0.0.1",
      "--port",
      String(port),
      "--persist-to",
      persistDir,
      "--config",
      configPath,
    ],
    {
      cwd: workerPocDir,
      detached: process.platform !== "win32",
      env: { ...process.env, WRANGLER_SEND_METRICS: "false" },
      stdio: ["ignore", "pipe", "pipe"],
    },
  );
  captureLogs(child);
  await waitForServer(baseUrl);

  const entries = [
    Buffer.from(`{"entry":"created","world_id":"${worldId}","version":1}\n`),
    Buffer.from('{"entry":"caps_granted","granted":[]}\n'),
    // Different legal whitespace makes byte preservation observable: the
    // Worker must store and return the submitted bytes, not reserialize JSON.
    Buffer.from('{ "entry": "caps_granted", "granted": [ ] }\n'),
  ];

  const firstAppend = await append(baseUrl, entries[0]);
  const secondAppend = await append(baseUrl, entries[1]);
  assert.equal(firstAppend.ordinal, 1, "first entry should have ordinal 1");
  assert.equal(secondAppend.ordinal, 2, "ordinals should increase monotonically");
  assert.equal(typeof secondAppend.bookmark, "string");
  assert(secondAppend.bookmark.length > 0, "append should return a D1 session bookmark");

  // This is a distinct HTTP request and therefore a fresh D1 session. The
  // append bookmark must make the just-written prefix visible to it.
  const snapshotResponse = await fetch(`${baseUrl}/snapshot`, {
    headers: { "x-d1-bookmark": secondAppend.bookmark },
    signal: abortSignal(),
  });
  const snapshot = await readJsonl(snapshotResponse, "bookmark snapshot");
  assert.equal(snapshot.first, 1);
  assert.equal(snapshot.last, 2);
  assert.equal(snapshot.count, 2);
  assert.deepEqual(snapshot.bytes, Buffer.concat(entries.slice(0, 2)), "snapshot must be byte-identical JSONL");

  const thirdAppend = await append(baseUrl, entries[2]);
  assert.equal(thirdAppend.ordinal, 3, "later append should follow the snapshot cursor");

  const tailResponse = await fetch(`${baseUrl}/tail?after=${snapshot.cursor}`, {
    headers: { "x-d1-bookmark": thirdAppend.bookmark },
    signal: abortSignal(),
  });
  const tail = await readJsonl(tailResponse, "ordered tail");
  assert.equal(tail.first, snapshot.cursor + 1, "tail should start immediately after the snapshot");
  assert.equal(tail.last, 3);
  assert.equal(tail.count, 1);
  assert.deepEqual(tail.bytes, entries[2], "tail must return the exact appended bytes");
  assert.deepEqual(
    Buffer.concat([snapshot.bytes, tail.bytes]),
    Buffer.concat(entries),
    "snapshot plus ordered tail must have no duplicate, gap, or reorder",
  );

  console.log("D1 journal-fit smoke: PASS");
  console.log("  append ordinals: 1, 2, 3");
  console.log("  bookmark carried to a fresh request: read-your-write PASS");
  console.log("  snapshot + ordered tail: byte-identical JSONL, no gap/duplicate/reorder");
}

function onSignal(signal) {
  interruptedBy = signal;
  interrupt.abort(new Error(`interrupted by ${signal}`));
  void stopServer();
}

process.once("SIGINT", onSignal);
process.once("SIGTERM", onSignal);

try {
  await main();
} catch (error) {
  if (!interruptedBy) {
    console.error(`D1 journal-fit smoke: FAIL: ${error?.stack ?? error}`);
    const logs = serverLogs();
    if (logs) console.error(logs);
    process.exitCode = 1;
  } else {
    process.exitCode = interruptedBy === "SIGINT" ? 130 : 143;
  }
} finally {
  process.removeListener("SIGINT", onSignal);
  process.removeListener("SIGTERM", onSignal);
  await stopServer();
  if (stateDir) await rm(stateDir, { recursive: true, force: true });
}
