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
// The output bound alone is not enough: fzstd's Decompress allocates
// the frame header's declared window on the first push, before any
// output flows (Codex P1 on PR #6726), and a header can declare a
// huge window — or a huge content size — over a tiny payload. The
// header is therefore parsed and refused up front when its declared
// content size exceeds the limit, or when its window exceeds the
// larger of the limit and MIN_WINDOW_ALLOWANCE_BYTES (encoders
// legitimately use windows up to a few MiB even for small objects:
// the reaper's own packer runs zstd -3, whose window caps at 2 MiB).
// The same bypass would otherwise reopen through a SECOND frame:
// fzstd's streaming decoder walks concatenated frames on its own,
// allocating each frame's declared window as it reaches it (Codex
// P1, second round). This decoder's contract has always been
// exactly one frame — the reaper stores one frame per object — so
// the first frame's block chain is walked to its exact end and any
// trailing bytes are rejected before fzstd runs at all.
//
// This module is the single decoder implementation for the whole package:
// the Worker bundles it (esbuild -> dist/worker.js) and the local proof
// test (test/run-local-proof.mjs) imports it directly, so the code that is
// tested locally is byte-for-byte the code that ships.

export class ExpansionLimitError extends Error {
  constructor(limit, produced, message) {
    super(
      message ??
        `zstd frame expands past the expected size: limit ${limit} bytes, ` +
          `already produced ${produced} bytes — refusing to return unverified bytes`,
    );
    this.name = "ExpansionLimitError";
    this.limit = limit;
  }
}

const INPUT_SLICE_BYTES = 16 * 1024;
const MIN_WINDOW_ALLOWANCE_BYTES = 8 * 1024 * 1024;

const ZSTD_MAGIC = 0xfd2fb528;
const SKIPPABLE_MAGIC_MIN = 0x184d2a50;
const SKIPPABLE_MAGIC_MAX = 0x184d2a5f;

/** Parse the first real frame's header. Returns
 * { contentSize, windowSize, frameStart, headerLength, checksum },
 * or null when the header cannot be positively parsed (too short,
 * bad magic, truncated) — in which case the streaming decoder is
 * left to succeed or fail exactly as it would without the
 * pre-check. Only a positively parsed violation is refused. */
function parseZstdFrameHeader(bytes) {
  let off = 0;
  for (;;) {
    if (bytes.length - off < 5) return null;
    const view = new DataView(bytes.buffer, bytes.byteOffset + off);
    const magic = view.getUint32(0, true);
    if (magic >= SKIPPABLE_MAGIC_MIN && magic <= SKIPPABLE_MAGIC_MAX) {
      if (bytes.length - off < 8) return null;
      off += 8 + view.getUint32(4, true);
      continue;
    }
    if (magic !== ZSTD_MAGIC) return null;
    break;
  }
  const head = off + 4;
  const descriptor = bytes[head];
  const fcsCode = descriptor >> 6;
  const singleSegment = (descriptor & 0x20) !== 0;
  const checksum = (descriptor & 0x04) !== 0;
  const dictIdCode = descriptor & 0x03;
  const dictIdBytes = [0, 1, 2, 4][dictIdCode];
  const fcsBytes = singleSegment ? [1, 2, 4, 8][fcsCode] : [0, 2, 4, 8][fcsCode];
  const headerLength = 1 + (singleSegment ? 0 : 1) + dictIdBytes + fcsBytes;
  if (bytes.length - head < headerLength) return null;

  const view = new DataView(bytes.buffer, bytes.byteOffset + head);
  let pos = 1;
  let windowSize;
  if (singleSegment) {
    windowSize = null; // set from the content size below
  } else {
    const wd = view.getUint8(pos);
    pos += 1;
    const windowLog = 10 + (wd >> 3);
    const windowBase = 2 ** windowLog;
    windowSize = windowBase + (windowBase / 8) * (wd & 0x07);
  }
  pos += dictIdBytes;
  let contentSize = null;
  if (fcsBytes > 0) {
    if (fcsBytes === 8) {
      const v = view.getBigUint64(pos, true);
      contentSize = v > BigInt(Number.MAX_SAFE_INTEGER) ? Number.MAX_SAFE_INTEGER : Number(v);
    } else {
      let v = 0;
      for (let i = fcsBytes - 1; i >= 0; i--) v = v * 256 + view.getUint8(pos + i);
      contentSize = fcsBytes === 2 ? v + 256 : v;
    }
  }
  if (singleSegment) windowSize = contentSize;
  return { contentSize, windowSize, frameStart: off, headerLength, checksum };
}

/** The exact end offset of the frame whose header `info` describes,
 * found by walking its block chain (raw and compressed blocks carry
 * their size; an RLE block carries one byte), plus the optional
 * content checksum. Returns null when the chain cannot be walked
 * positively (truncated, reserved block type, overrun) — the
 * streaming decoder is then left to fail as it would anyway. */
function zstdFrameEnd(bytes, info) {
  let pos = info.frameStart + 4 + info.headerLength;
  for (;;) {
    if (pos + 3 > bytes.length) return null;
    const bh = bytes[pos] | (bytes[pos + 1] << 8) | (bytes[pos + 2] << 16);
    pos += 3;
    const last = (bh & 1) !== 0;
    const type = (bh >> 1) & 3;
    const size = bh >> 3;
    if (type === 3) return null;
    pos += type === 1 ? 1 : size;
    if (pos > bytes.length) return null;
    if (last) break;
  }
  if (info.checksum) pos += 4;
  return pos <= bytes.length ? pos : null;
}

export function decompressFrame(frameBytes, maxPlainBytes) {
  if (!Number.isSafeInteger(maxPlainBytes) || maxPlainBytes < 0) {
    throw new TypeError(
      `decompressFrame requires a non-negative integer byte limit, got ${maxPlainBytes}`,
    );
  }
  const header = parseZstdFrameHeader(frameBytes);
  if (header !== null) {
    if (header.contentSize !== null && header.contentSize > maxPlainBytes) {
      throw new ExpansionLimitError(
        maxPlainBytes,
        header.contentSize,
        `zstd frame declares a content size of ${header.contentSize} bytes, past the ` +
          `expected size (limit ${maxPlainBytes} bytes) — refusing to return unverified bytes`,
      );
    }
    const windowCap = Math.max(maxPlainBytes, MIN_WINDOW_ALLOWANCE_BYTES);
    if (header.windowSize !== null && header.windowSize > windowCap) {
      throw new ExpansionLimitError(
        maxPlainBytes,
        header.windowSize,
        `zstd frame declares a ${header.windowSize}-byte window, over the ${windowCap}-byte ` +
          `bound for an expected size of ${maxPlainBytes} bytes — refusing to return unverified bytes`,
      );
    }
    const end = zstdFrameEnd(frameBytes, header);
    if (end !== null && end < frameBytes.length) {
      // Exactly one frame per object is the storage contract; a
      // second frame would be decoded by the streaming decoder on
      // its own, allocating its declared window unchecked.
      throw new Error(
        `expected exactly one zstd frame, but ${frameBytes.length - end} trailing ` +
          "byte(s) follow it — refusing to decode concatenated frames",
      );
    }
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
