// Regression test for issue #6182: when Drive ignores the Range header
// and answers 200 with the whole bundle, BundleFlight.rehydrate must
// stream it — discard bytes before the frame, retain only the frame,
// and stop the download once the frame is complete — instead of
// buffering the entire bundle via resp.arrayBuffer().
//
// Run: node test/range-ignored.test.mjs   (also wired into `npm test`)

import { createHash } from "node:crypto";
import { zstdCompressSync } from "node:zlib";
import { BundleFlight } from "../src/index.js";

const CHUNK = 65536;

let failures = 0;
function check(name, cond, detail = "") {
  if (cond) {
    console.log(`ok: ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL: ${name} ${detail}`);
  }
}

const plaintext = new TextEncoder().encode(
  Array.from({ length: 3000 }, (_, i) => `range-ignored proof line ${i}`).join("\n"),
);
const frame = new Uint8Array(zstdCompressSync(plaintext));
const sha = createHash("sha256").update(plaintext).digest("hex");

function bundleBytes(trailing = 3_000_000, truncateTo = null) {
  const offset = 100_003; // deliberately not chunk-aligned
  const total = offset + frame.length + trailing;
  const bundle = new Uint8Array(truncateTo ?? total);
  bundle.fill(0x61);
  bundle.set(frame, offset);
  return { bundle, offset };
}

// A pull-based stream over `bytes` that records how much the consumer
// actually pulled and whether it cancelled before the end.
function trackedStream(bytes) {
  let idx = 0;
  let pulled = 0;
  let cancelled = false;
  const stream = new ReadableStream({
    pull(controller) {
      if (idx >= bytes.length) {
        controller.close();
        return;
      }
      const chunk = bytes.subarray(idx, idx + CHUNK);
      idx += chunk.length;
      pulled += chunk.length;
      controller.enqueue(chunk);
    },
    cancel() {
      cancelled = true;
    },
  });
  return { stream, stats: () => ({ pulled, cancelled }) };
}

function makeFlight(driveResponseFactory) {
  const flight = new BundleFlight(
    { storage: { async get() { return null; }, async put() {} } },
    { GOOGLE_CLIENT_ID: "id", GOOGLE_CLIENT_SECRET: "secret", GOOGLE_REFRESH_TOKEN: "rt" },
  );
  const realFetch = globalThis.fetch;
  globalThis.fetch = async (url) => {
    if (String(url).startsWith("https://oauth2.googleapis.com/token")) {
      return new Response(JSON.stringify({ access_token: "tok", expires_in: 3600 }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      });
    }
    return driveResponseFactory();
  };
  return { flight, restore: () => { globalThis.fetch = realFetch; } };
}

const job = (offset, shaOverride = sha) => ({
  drive_file_id: "file-1",
  offset,
  frame_length: frame.length,
  sha256: shaOverride,
  bundle_id: "bundle-1",
});

// 1. Range ignored (200, large streaming bundle): correct plaintext,
//    stream cancelled, and only a frame-sized prefix ever pulled.
{
  const { bundle, offset } = bundleBytes();
  const tracked = trackedStream(bundle);
  const { flight, restore } = makeFlight(
    () => new Response(tracked.stream, { status: 200 }),
  );
  const result = await flight.rehydrate(job(offset));
  restore();
  check("200 range-ignored returns the requested frame's plaintext",
    Buffer.from(result.bytes).equals(Buffer.from(plaintext)));
  const { pulled, cancelled } = tracked.stats();
  check("200 range-ignored cancels the stream once the frame is complete", cancelled);
  check("200 range-ignored never pulls the whole bundle",
    pulled <= offset + frame.length + 2 * CHUNK && pulled < bundle.length,
    `pulled=${pulled} bundle=${bundle.length}`);
}

// 2. Normal 206 range response: current behavior and hash check retained.
{
  const { offset } = bundleBytes(0);
  const { flight, restore } = makeFlight(
    () => new Response(frame, { status: 206 }),
  );
  const result = await flight.rehydrate(job(offset));
  restore();
  check("206 range response returns the requested frame's plaintext",
    Buffer.from(result.bytes).equals(Buffer.from(plaintext)));
}

// 3. Truncated 200 (stream ends inside the frame): fails closed.
{
  const { bundle, offset } = bundleBytes(0);
  const cut = bundle.subarray(0, offset + Math.floor(frame.length / 2));
  const tracked = trackedStream(cut);
  const { flight, restore } = makeFlight(
    () => new Response(tracked.stream, { status: 200 }),
  );
  let rejected = false;
  try {
    await flight.rehydrate(job(offset));
  } catch {
    rejected = true;
  }
  restore();
  check("truncated 200 response fails closed", rejected);
}

// 4. Hash mismatch (frame present but manifest hash differs): fails closed.
{
  const { bundle, offset } = bundleBytes(0);
  const tracked = trackedStream(bundle);
  const { flight, restore } = makeFlight(
    () => new Response(tracked.stream, { status: 200 }),
  );
  let rejected = false;
  try {
    await flight.rehydrate(job(offset, "0".repeat(64)));
  } catch {
    rejected = true;
  }
  restore();
  check("sha256 mismatch on a 200 response fails closed", rejected);
}

if (failures > 0) {
  console.error(`FAIL: ${failures} check(s) failed`);
  process.exit(1);
}
console.log("PASS: range-ignored streaming rehydration (issue #6182)");
