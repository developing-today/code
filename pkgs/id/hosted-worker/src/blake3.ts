// BLAKE3 (hash mode, 32-byte output) in pure TypeScript.
//
// The bridge needs operation IDs, and an operation ID is BLAKE3 over
// the received event bytes (pkgs/id chain.rs / p2panda-core). Workers
// have no BLAKE3 in WebCrypto and this package carries no runtime
// dependencies, so the hash is implemented here from the BLAKE3
// reference specification: the compression function over the SHA-256
// IV with the message permutation, 1024-byte chunks, and the binary
// tree of parent nodes keyed by the chunk counter.
//
// Oracle: the official BLAKE3 test vectors (input byte i = i mod 251),
// cross-checked against the Rust `blake3` crate in
// pkgs/id/tests/bridge_fixtures.rs; see test/bridge-blake3.test.ts.

const IV = new Uint32Array([
  0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
]);
const MSG_PERMUTATION = [2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8];

const CHUNK_START = 1 << 0;
const CHUNK_END = 1 << 1;
const PARENT = 1 << 2;
const ROOT = 1 << 3;

const BLOCK_LEN = 64;
const CHUNK_LEN = 1024;

function rotr(x: number, n: number): number {
  return ((x >>> n) | (x << (32 - n))) >>> 0;
}

function g(state: Uint32Array, a: number, b: number, c: number, d: number, mx: number, my: number): void {
  state[a] = (state[a] + state[b] + mx) >>> 0;
  state[d] = rotr(state[d] ^ state[a], 16);
  state[c] = (state[c] + state[d]) >>> 0;
  state[b] = rotr(state[b] ^ state[c], 12);
  state[a] = (state[a] + state[b] + my) >>> 0;
  state[d] = rotr(state[d] ^ state[a], 8);
  state[c] = (state[c] + state[d]) >>> 0;
  state[b] = rotr(state[b] ^ state[c], 7);
}

function roundFn(state: Uint32Array, m: Uint32Array): void {
  g(state, 0, 4, 8, 12, m[0], m[1]);
  g(state, 1, 5, 9, 13, m[2], m[3]);
  g(state, 2, 6, 10, 14, m[4], m[5]);
  g(state, 3, 7, 11, 15, m[6], m[7]);
  g(state, 0, 5, 10, 15, m[8], m[9]);
  g(state, 1, 6, 11, 12, m[10], m[11]);
  g(state, 2, 7, 8, 13, m[12], m[13]);
  g(state, 3, 4, 9, 14, m[14], m[15]);
}

function permute(m: Uint32Array): Uint32Array {
  const out = new Uint32Array(16);
  for (let i = 0; i < 16; i++) out[i] = m[MSG_PERMUTATION[i]];
  return out;
}

/** One compression: returns the full 16-word output. The chaining
 * value is words 0..8; root output uses all 16 words per block. */
function compress(
  cv: Uint32Array,
  blockWords: Uint32Array,
  counter: bigint,
  blockLen: number,
  flags: number,
): Uint32Array {
  const state = new Uint32Array(16);
  state.set(cv, 0);
  state[8] = IV[0];
  state[9] = IV[1];
  state[10] = IV[2];
  state[11] = IV[3];
  state[12] = Number(counter & 0xffffffffn);
  state[13] = Number((counter >> 32n) & 0xffffffffn);
  state[14] = blockLen;
  state[15] = flags;
  let m = blockWords;
  for (let round = 0; round < 7; round++) {
    roundFn(state, m);
    if (round < 6) m = permute(m);
  }
  const out = new Uint32Array(16);
  for (let i = 0; i < 8; i++) out[i] = (state[i + 8] ^ state[i]) >>> 0;
  for (let i = 0; i < 8; i++) out[i + 8] = (state[i] ^ cv[i]) >>> 0;
  return out;
}

function wordsFromBlock(block: Uint8Array, offset: number): Uint32Array {
  const words = new Uint32Array(16);
  for (let i = 0; i < 16; i++) {
    const j = offset + i * 4;
    words[i] = (block[j] | (block[j + 1] << 8) | (block[j + 2] << 16) | (block[j + 3] << 24)) >>> 0;
  }
  return words;
}

function wordsToBytes(words: Uint32Array, out: Uint8Array, offset: number, count: number): void {
  for (let i = 0; i < count; i++) {
    const w = words[i];
    out[offset + i * 4] = w & 0xff;
    out[offset + i * 4 + 1] = (w >>> 8) & 0xff;
    out[offset + i * 4 + 2] = (w >>> 16) & 0xff;
    out[offset + i * 4 + 3] = (w >>> 24) & 0xff;
  }
}

interface Output {
  cv: Uint32Array;
  block: Uint8Array;
  counter: bigint;
  blockLen: number;
  flags: number;
}

function outputChainingValue(output: Output): Uint32Array {
  return compress(output.cv, wordsFromBlock(output.block, 0), output.counter, output.blockLen, output.flags).slice(
    0,
    8,
  );
}

function outputRootBytes(output: Output, length: number): Uint8Array {
  const out = new Uint8Array(length);
  let written = 0;
  let counter = 0n;
  while (written < length) {
    const words = compress(output.cv, wordsFromBlock(output.block, 0), counter, output.blockLen, output.flags | ROOT);
    const block = new Uint8Array(64);
    wordsToBytes(words, block, 0, 16);
    const take = Math.min(64, length - written);
    out.set(block.subarray(0, take), written);
    written += take;
    counter += 1n;
  }
  return out;
}

/** The output object of one 1024-byte chunk. */
function chunkOutput(chunk: Uint8Array, counter: bigint): Output {
  let cv = new Uint32Array(IV);
  const blocks = Math.max(1, Math.ceil(chunk.length / BLOCK_LEN));
  for (let b = 0; b < blocks; b++) {
    const slice = chunk.subarray(b * BLOCK_LEN, (b + 1) * BLOCK_LEN);
    const block = new Uint8Array(BLOCK_LEN);
    block.set(slice);
    const isLast = b === blocks - 1;
    if (!isLast) {
      const flags = b === 0 ? CHUNK_START : 0;
      cv = compress(cv, wordsFromBlock(block, 0), counter, BLOCK_LEN, flags).slice(0, 8);
    } else {
      return {
        cv,
        block,
        counter,
        blockLen: slice.length,
        flags: (b === 0 ? CHUNK_START : 0) | CHUNK_END,
      };
    }
  }
  throw new Error("unreachable");
}

function parentOutput(leftCv: Uint32Array, rightCv: Uint32Array): Output {
  const block = new Uint8Array(BLOCK_LEN);
  wordsToBytes(leftCv, block, 0, 8);
  wordsToBytes(rightCv, block, 32, 8);
  return { cv: new Uint32Array(IV), block, counter: 0n, blockLen: BLOCK_LEN, flags: PARENT };
}

/** BLAKE3 of `data`, 32 bytes. */
export function blake3(data: Uint8Array): Uint8Array {
  const chunkCount = Math.max(1, Math.ceil(data.length / CHUNK_LEN));
  const cvStack: Uint32Array[] = [];
  let lastOutput: Output | null = null;
  for (let i = 0; i < chunkCount; i++) {
    const chunk = data.subarray(i * CHUNK_LEN, (i + 1) * CHUNK_LEN);
    lastOutput = chunkOutput(chunk, BigInt(i));
    if (i === chunkCount - 1) break;
    // Merge the completed chunk into the tree: while the number of
    // chunks completed so far is even, fold with the stack top.
    let cv = outputChainingValue(lastOutput);
    let total = BigInt(i + 1);
    while (total % 2n === 0n) {
      const left = cvStack.pop();
      if (!left) throw new Error("blake3 tree stack underflow");
      cv = outputChainingValue(parentOutput(left, cv));
      total /= 2n;
    }
    cvStack.push(cv);
  }
  let output = lastOutput as Output;
  while (cvStack.length > 0) {
    const left = cvStack.pop() as Uint32Array;
    output = parentOutput(left, outputChainingValue(output));
  }
  return outputRootBytes(output, 32);
}
