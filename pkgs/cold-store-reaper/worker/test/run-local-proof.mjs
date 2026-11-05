// Local proof for the rehydrator Worker (run: npm test).
//
// 1. Uses the real Python reaper (`cold_store.ColdStore.pack`) to build a
//    bundle from a sample file, and reads the member's offset/length/hash
//    back out of the manifest SQLite it writes.
// 2. Slices exactly that frame out of the bundle file.
// 3. Decodes it with ../src/decoder.js — the identical module esbuild
//    bundles into dist/worker.js — and asserts the plaintext sha256.
//
// Exits non-zero on any mismatch; prints PASS on success.

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { decompressFrame } from "../src/decoder.js";
import { BundleFlight } from "../src/index.js";

const pkgDir = join(dirname(fileURLToPath(import.meta.url)), "..", "..");

const PACK_SNIPPET = `
import json, sqlite3, sys
from pathlib import Path
sys.path.insert(0, sys.argv[1])
from cold_store import ColdStore
home, sample = Path(sys.argv[2]), sys.argv[3]
store = ColdStore(home)
store.init()
res = store.pack([sys.argv[4], sample])  # decoy member first -> offset > 0
conn = store.db()
row = conn.execute(
    "SELECT object_hash, offset, length FROM members WHERE bundle_id = ?"
    " ORDER BY offset DESC LIMIT 1",
    (res["bundle_id"],),
).fetchone()
print(json.dumps({
    "bundle_path": res["path"],
    "hash": row["object_hash"],
    "offset": row["offset"],
    "length": row["length"],
}))
`;

const work = mkdtempSync(join(tmpdir(), "cold-store-proof-"));
const otherPath = join(work, "other.txt");
writeFileSync(otherPath, "an unrelated first member, just to shift the offset\n");
const samplePath = join(work, "sample.txt");
// Line-numbered content: realistic size, does not collapse to a stub frame.
const sample = Buffer.from(
  Array.from(
    { length: 2000 },
    (_, i) => `rehydrator local proof line ${i} — cold store`,
  ).join("\n"),
);
writeFileSync(samplePath, sample);

const out = execFileSync(
  "python3",
  ["-c", PACK_SNIPPET, pkgDir, join(work, "home"), samplePath, otherPath],
  { encoding: "utf8" },
);
const info = JSON.parse(out.trim().split("\n").pop());

const bundle = readFileSync(info.bundle_path);
const frame = bundle.subarray(info.offset, info.offset + info.length);
const plain = Buffer.from(decompressFrame(new Uint8Array(frame)));
const digest = createHash("sha256").update(plain).digest("hex");

if (digest !== info.hash) {
  console.error(`FAIL: sha256 mismatch: decoded ${digest}, manifest ${info.hash}`);
  process.exit(1);
}
if (!plain.equals(sample)) {
  console.error("FAIL: decoded bytes differ from the packed sample");
  process.exit(1);
}

const originalFetch = globalThis.fetch;
const flight = new BundleFlight(
  { storage: { get: async () => ({ token: "proof", expires_at_ms: Date.now() + 3_600_000 }) } },
  {},
);
const job = {
  drive_file_id: "proof",
  offset: info.offset,
  frame_length: info.length,
  sha256: info.hash,
  bundle_id: "proof",
};
const rehydrate = async (status, body, sha256 = info.hash, offset = info.offset) => {
  globalThis.fetch = async () => new Response(body, { status });
  try {
    return await flight.rehydrate({ ...job, sha256, offset });
  } finally {
    globalThis.fetch = originalFetch;
  }
};

let pulls = 0;
let cancelled = false;
let tailChunks = 0;
let prefixBytes = 0;
let frameSent = false;
const streamOffset = 2 * 1024 * 1024;
const frameChunk = bundle.subarray(info.offset, info.offset + info.length);
const ignoredRangeBody = new ReadableStream(
  {
    pull(controller) {
      pulls += 1;
      if (prefixBytes < streamOffset) {
        const length = Math.min(64 * 1024, streamOffset - prefixBytes);
        prefixBytes += length;
        controller.enqueue(new Uint8Array(length));
      } else if (!frameSent) {
        frameSent = true;
        controller.enqueue(frameChunk);
      } else {
        tailChunks += 1;
        controller.enqueue(new Uint8Array(64 * 1024));
      }
    },
    cancel() {
      cancelled = true;
    },
  },
  { highWaterMark: 0 },
);
const streamed = await rehydrate(200, ignoredRangeBody, info.hash, streamOffset);
assert.deepEqual(Buffer.from(streamed.bytes), sample);
assert.equal(cancelled, true, "ignored-Range stream should be cancelled after the frame");
assert.equal(pulls, streamOffset / (64 * 1024) + 1);
assert.equal(tailChunks, 0, "ignored-Range stream should not read the bundle tail");

const ranged = await rehydrate(206, frame);
assert.deepEqual(Buffer.from(ranged.bytes), sample);
await assert.rejects(
  rehydrate(200, bundle.subarray(0, info.offset + info.length - 1)),
  /drive range download short:/,
);
await assert.rejects(rehydrate(206, frame, "0".repeat(64)), /sha256 mismatch/);
await assert.rejects(rehydrate(206, new Uint8Array(info.length)), /zstd frame decompression failed/);

console.log(
  `PASS: frame at offset ${info.offset} (${info.length} bytes) decoded and hash-verified; ` +
    "ignored-Range stream stopped at the frame; 206, truncation, malformed-frame, and hash checks passed",
);
