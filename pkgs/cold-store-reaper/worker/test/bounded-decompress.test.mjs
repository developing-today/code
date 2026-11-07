// Regression test for issue #6590: zstd decompression in the
// rehydration path is bounded by the manifest's expected plaintext
// size (the job's `size`, selected as o.size by the entry point).
//
// Before the fix, decompressFrame was a bare fzstd decompress(): a
// small frame expanded without limit before the sha256 check, so a
// corrupt or hostile bundle member could expand past the Worker's
// memory limit and kill the isolate instead of producing the
// designed clean 502. The job's size field was selected and posted
// to the Durable Object but never read.
//
// Run: node test/bounded-decompress.test.mjs   (also wired into `npm test`)

import { createHash } from "node:crypto";
import { zstdCompressSync } from "node:zlib";

import { decompressFrame, ExpansionLimitError } from "../src/decoder.js";
import { BundleFlight } from "../src/index.js";

let failures = 0;
function check(name, cond, detail = "") {
  if (cond) {
    console.log(`ok: ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL: ${name} ${detail}`);
  }
}

const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");

// A high-expansion frame: 4 MiB of zeros compresses to a few KiB.
const bombPlain = new Uint8Array(4 * 1024 * 1024);
const bombFrame = new Uint8Array(zstdCompressSync(bombPlain));
const bombSha = sha256(bombPlain);
check("bomb frame is small relative to its expansion",
  bombFrame.length < bombPlain.length / 100,
  `frame=${bombFrame.length} plain=${bombPlain.length}`);

// --- decoder level -------------------------------------------------

{
  let err = null;
  try {
    decompressFrame(bombFrame, 1024 * 1024);
  } catch (e) {
    err = e;
  }
  check("decoder throws ExpansionLimitError past the expected size",
    err instanceof ExpansionLimitError, `got ${err}`);
}

{
  const plain = decompressFrame(bombFrame, bombPlain.length);
  check("decoder serves a frame expanding to exactly the expected size",
    plain.length === bombPlain.length && sha256(plain) === bombSha);
}

{
  let err = null;
  try {
    decompressFrame(bombFrame, bombPlain.length - 1);
  } catch (e) {
    err = e;
  }
  check("decoder throws when the expansion is one byte over the expected size",
    err instanceof ExpansionLimitError, `got ${err}`);
}

{
  const small = new TextEncoder().encode("a short, legitimate object");
  const smallFrame = new Uint8Array(zstdCompressSync(small));
  const plain = decompressFrame(smallFrame, small.length);
  check("decoder is unaffected for a short frame at its exact size",
    Buffer.from(plain).equals(Buffer.from(small)));
}

for (const badLimit of [undefined, -1, 1.5, "100", null]) {
  let err = null;
  try {
    decompressFrame(bombFrame, badLimit);
  } catch (e) {
    err = e;
  }
  check(`decoder refuses an unusable byte limit (${String(badLimit)})`,
    err instanceof TypeError, `got ${err}`);
}

// --- rehydrate level (the range-ignored.test.mjs harness shape) ---

function makeFlight(frame) {
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
    return new Response(frame, { status: 206 });
  };
  return { flight, restore: () => { globalThis.fetch = realFetch; } };
}

const bombJob = (overrides = {}) => ({
  drive_file_id: "file-1",
  offset: 0,
  frame_length: bombFrame.length,
  sha256: bombSha,
  bundle_id: "bundle-1",
  size: bombPlain.length,
  ...overrides,
});

async function expect502(name, job, messagePart) {
  const { flight, restore } = makeFlight(bombFrame);
  let err = null;
  try {
    await flight.rehydrate(job);
  } catch (e) {
    err = e;
  }
  restore();
  check(name,
    err !== null && err.status === 502 && String(err.message).includes(messagePart),
    `got ${err && `${err.status} ${err.message}`}`);
}

// The sha would match and the frame is well-formed: only the bound
// can refuse this run, and it must do so with the 502 shape.
await expect502("rehydrate refuses a frame expanding past the job size with a 502",
  bombJob({ size: 1024 * 1024 }), "expands past the expected");

// A job whose size never arrived (or is not numeric) fails closed
// instead of falling back to unbounded decompression.
await expect502("rehydrate fails closed when the job has no size",
  bombJob({ size: undefined }), "no usable expected size");
await expect502("rehydrate fails closed when the job size is not numeric",
  bombJob({ size: "not-a-size" }), "no usable expected size");

// Control: exact expected size + matching sha is served as before.
{
  const { flight, restore } = makeFlight(bombFrame);
  const result = await flight.rehydrate(bombJob());
  restore();
  check("rehydrate serves an exact-size frame whose sha256 matches",
    result.bytes.length === bombPlain.length && result.sha256 === bombSha);
}

if (failures > 0) {
  console.error(`FAIL: ${failures} check(s) failed`);
  process.exit(1);
}
console.log("PASS: bounded decompression (issue #6590)");
