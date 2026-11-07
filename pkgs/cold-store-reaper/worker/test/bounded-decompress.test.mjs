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
  bombJob({ size: 1024 * 1024 }), "past the expected size");

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

// --- frame-header pre-check (Codex P1 on PR #6726) -----------------
//
// fzstd's Decompress allocates the header's declared window on the
// first push, before any output flows, so the output bound alone
// cannot stop a header-driven allocation. These hand-built frames
// carry a small raw-block payload under an inflated header; the
// pre-check must refuse them before the streaming decoder runs.

function rawBlockFrame(content, windowLog) {
  // Non-single-segment frame, no FCS, no checksum, no dictID, one
  // raw (stored) block: the payload needs no compression machinery,
  // so the header bytes are exactly what this test says they are.
  const wd = (windowLog - 10) << 3; // mantissa 0 -> window = 2**windowLog
  const blockHeader = (content.length << 3) | 1; // last block, raw type
  return new Uint8Array([
    0x28, 0xb5, 0x2f, 0xfd, // zstd magic, little-endian
    0x00, // frame header descriptor
    wd,
    blockHeader & 0xff, (blockHeader >> 8) & 0xff, (blockHeader >> 16) & 0xff,
    ...content,
  ]);
}

const smallContent = new TextEncoder().encode("w".repeat(256));

{
  // A 512 MiB declared window over a 256-byte payload: refused even
  // though the payload would fit the limit exactly.
  let err = null;
  try {
    decompressFrame(rawBlockFrame(smallContent, 29), smallContent.length);
  } catch (e) {
    err = e;
  }
  check("decoder refuses a 512 MiB declared window over a small payload up front",
    err instanceof ExpansionLimitError && String(err.message).includes("window"),
  `got ${err}`);
}

{
  // An 8 MiB declared window is within the allowance (encoders use
  // windows of a few MiB for small objects): the frame decodes.
  const plain = decompressFrame(rawBlockFrame(smallContent, 23), smallContent.length);
  check("decoder accepts a declared window within the allowance and decodes the payload",
    Buffer.from(plain).equals(Buffer.from(smallContent)));
}

{
  // A 16 MiB window is over the allowance for a small object but
  // under a 32 MiB expected size: the cap scales with the limit.
  const plain = decompressFrame(rawBlockFrame(smallContent, 24), 32 * 1024 * 1024);
  check("decoder accepts a declared window under a larger expected-size cap",
    Buffer.from(plain).equals(Buffer.from(smallContent)));
}

{
  // Declared content size (4-byte FCS) of 1,000,000 over a 10-byte
  // raw payload, limit 100: refused on the header, before decoding.
  const blockHeader = (10 << 3) | 1;
  const frame = new Uint8Array([
    0x28, 0xb5, 0x2f, 0xfd,
    0x80, // FCS field size code 2 -> 4-byte frame content size
    0x00, // window descriptor: 1 KiB window
    0x40, 0x42, 0x0f, 0x00, // FCS = 1,000,000 little-endian
    blockHeader & 0xff, (blockHeader >> 8) & 0xff, (blockHeader >> 16) & 0xff,
    ...new TextEncoder().encode("0123456789"),
  ]);
  let err = null;
  try {
    decompressFrame(frame, 100);
  } catch (e) {
    err = e;
  }
  check("decoder refuses a declared content size past the expected size up front",
    err instanceof ExpansionLimitError && String(err.message).includes("content size"),
  `got ${err}`);
}

// --- property coverage (Codex P1 on PR #6726; testing doctrine) ----
//
// ORACLES (named): (1) round-trip -- the generated plaintext itself:
// a limit at or above its length must reproduce it byte-for-byte;
// (2) bound -- a limit below its length must throw
// ExpansionLimitError, never return truncated or over-limit bytes.
// The generator is a fixed-seed mulberry32, so the cases are a
// deterministic regression pin, not a fuzz run.

function mulberry32(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

{
  const rnd = mulberry32(0x6590);
  const shapes = [
    (n) => new Uint8Array(n), // zeros: maximum expansion ratio
    (n) => Uint8Array.from({ length: n }, () => Math.floor(rnd() * 256)), // incompressible
    (n) => Uint8Array.from({ length: n }, (_, i) => (i * 7 + 3) % 251), // periodic
    (n) => new TextEncoder().encode(("cold store property line\n").repeat(Math.ceil(n / 25)).slice(0, n)),
  ];
  let cases = 0;
  let propFailures = 0;
  for (const shape of shapes) {
    for (const n of [0, 1, 2, 17, 100, 1000, 4097, 65536]) {
      const plain = shape(n);
      const frame = new Uint8Array(zstdCompressSync(plain));
      for (const limit of [plain.length, plain.length + 1000]) {
        cases += 1;
        try {
          const out = decompressFrame(frame, limit);
          if (!Buffer.from(out).equals(Buffer.from(plain))) propFailures += 1;
        } catch {
          propFailures += 1;
        }
      }
      if (plain.length > 0) {
        cases += 1;
        let err = null;
        try {
          decompressFrame(frame, plain.length - 1);
        } catch (e) {
          err = e;
        }
        if (!(err instanceof ExpansionLimitError)) propFailures += 1;
      }
    }
  }
  check(`property: round-trip and bound hold across ${cases} generated cases`,
    propFailures === 0, `${propFailures} violated cases`);
}

// --- exactly one frame (Codex P1, second round, PR #6726) ----------
//
// The streaming decoder walks concatenated frames on its own and
// would allocate a second frame's declared window unchecked, so
// trailing data after the first frame is rejected up front.

{
  // The reported bypass shape: a small valid frame, then a frame
  // declaring a 512 MiB window over a 256-byte payload, limit 256.
  const first = new Uint8Array(zstdCompressSync(smallContent));
  const concat = new Uint8Array([...first, ...rawBlockFrame(smallContent, 29)]);
  let err = null;
  try {
    decompressFrame(concat, smallContent.length);
  } catch (e) {
    err = e;
  }
  check("decoder rejects a concatenated second frame before decoding it",
    err !== null && String(err.message).includes("exactly one zstd frame"),
  `got ${err}`);
}

{
  // Even two well-formed frames are refused: one object is one frame.
  const one = new Uint8Array(zstdCompressSync(smallContent));
  const concat = new Uint8Array([...one, ...one]);
  let err = null;
  try {
    decompressFrame(concat, smallContent.length * 2);
  } catch (e) {
    err = e;
  }
  check("decoder rejects two concatenated well-formed frames",
    err !== null && String(err.message).includes("exactly one zstd frame"),
  `got ${err}`);
}

{
  // A leading skippable frame before the one real frame is fine:
  // skippable frames carry no payload and allocate no window.
  const skippable = new Uint8Array([
    0x50, 0x2a, 0x4d, 0x18, // skippable magic 0x184d2a50, little-endian
    0x03, 0x00, 0x00, 0x00, // payload size 3
    0xaa, 0xbb, 0xcc,
  ]);
  const real = new Uint8Array(zstdCompressSync(smallContent));
  const plain = decompressFrame(new Uint8Array([...skippable, ...real]), smallContent.length);
  check("decoder still decodes a single frame preceded by a skippable frame",
    Buffer.from(plain).equals(Buffer.from(smallContent)));
}

if (failures > 0) {
  console.error(`FAIL: ${failures} check(s) failed`);
  process.exit(1);
}
console.log("PASS: bounded decompression (issue #6590)");
