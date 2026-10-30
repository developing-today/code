import { decompress } from "fzstd";

// Decompress exactly one zstd frame. The reaper stores every object as its
// own self-contained zstd frame inside a bundle file, so a frame pulled out
// by byte range decompresses on its own — no bundle context needed.
//
// This module is the single decoder implementation for the whole package:
// the Worker bundles it (esbuild -> dist/worker.js) and the local proof
// test (test/run-local-proof.mjs) imports it directly, so the code that is
// tested locally is byte-for-byte the code that ships.
export function decompressFrame(frameBytes) {
  return decompress(frameBytes);
}
