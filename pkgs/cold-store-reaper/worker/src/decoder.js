import { Decompress } from "fzstd";

// Decompress exactly one zstd frame, bounded by the expected plaintext
// size. The reaper stores every object as its own self-contained zstd
// frame inside a bundle file, so a frame pulled out by byte range
// decompresses on its own — no bundle context needed.
//
// The bound is the point (#6590): the unbounded decompress() this
// module used before allocates the whole expansion, so a small frame
// could expand past the Worker's memory limit and kill the isolate
// instead of producing the designed clean 502. Here the frame is fed
// to a streaming decompressor in slices and the output is counted as
// it is produced; the moment it exceeds maxPlainBytes the output is
// dropped and ExpansionLimitError is thrown, so peak memory tracks
// the expected size (plus one output block of slack), never the
// frame's declared or actual expansion.
//
// This module is the single decoder implementation for the whole package:
// the Worker bundles it (esbuild -> dist/worker.js) and the local proof
// test (test/run-local-proof.mjs) imports it directly, so the code that is
// tested locally is byte-for-byte the code that ships.

export class ExpansionLimitError extends Error {
  constructor(limit, produced) {
    super(
      `zstd frame expands past the expected size: limit ${limit} bytes, ` +
        `already produced ${produced} bytes`,
    );
    this.name = "ExpansionLimitError";
    this.limit = limit;
  }
}

const INPUT_SLICE_BYTES = 16 * 1024;

export function decompressFrame(frameBytes, maxPlainBytes) {
  if (!Number.isSafeInteger(maxPlainBytes) || maxPlainBytes < 0) {
    throw new TypeError(
      `decompressFrame requires a non-negative integer byte limit, got ${maxPlainBytes}`,
    );
  }
  const chunks = [];
  let total = 0;
  let exceeded = false;
  const stream = new Decompress((data) => {
    if (exceeded) return; // drop late output; the throw happens below
    total += data.length;
    if (total > maxPlainBytes) {
      exceeded = true;
      chunks.length = 0;
      return;
    }
    chunks.push(data);
  });
  if (frameBytes.length === 0) {
    // Keep the old behaviour for an empty frame: fzstd rejects it as
    // invalid data instead of our returning an empty plaintext.
    stream.push(new Uint8Array(0), true);
  }
  for (let off = 0; off < frameBytes.length && !exceeded; off += INPUT_SLICE_BYTES) {
    const end = Math.min(frameBytes.length, off + INPUT_SLICE_BYTES);
    stream.push(frameBytes.subarray(off, end), end === frameBytes.length);
  }
  if (exceeded) {
    throw new ExpansionLimitError(maxPlainBytes, total);
  }
  const out = new Uint8Array(total);
  let pos = 0;
  for (const chunk of chunks) {
    out.set(chunk, pos);
    pos += chunk.length;
  }
  return out;
}
