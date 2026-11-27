// BLAKE3 for the bridge (src/blake3.ts).
//
// Oracle: the official BLAKE3 test vectors — the empty-input value
// below is the published vector, the rest are cross-pinned with the
// Rust implementation (p2panda-core's Hash) in
// pkgs/id/tests/bridge_fixtures.rs. Both suites read the same table
// from pkgs/id/tests/fixtures/hosted-second-version-g1.json. Input
// byte i is i mod 251, the vectors' input pattern.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { blake3 } from "../src/blake3.ts";
import { bytesToHex } from "../src/ed25519.ts";

interface SharedVectors {
  blake3: { input_len: number; digest_hex: string }[];
}

const SHARED_VECTORS = JSON.parse(
  readFileSync(new URL("../../tests/fixtures/hosted-second-version-g1.json", import.meta.url), "utf8"),
) as SharedVectors;

function pattern(n: number): Uint8Array {
  const out = new Uint8Array(n);
  for (let i = 0; i < n; i++) out[i] = i % 251;
  return out;
}

test("blake3 matches the official test vectors", () => {
  assert.equal(SHARED_VECTORS.blake3.length, 11, "shared BLAKE3 fixture retains every vector");
  for (const { input_len, digest_hex } of SHARED_VECTORS.blake3) {
    assert.equal(bytesToHex(blake3(pattern(input_len))), digest_hex, `length ${input_len}`);
  }
});

test("blake3 of a chunk boundary differs from its neighbours", () => {
  // The tree shape changes at 1024 bytes; a hash that ignored the
  // chunking would collide across the boundary inputs.
  const a = bytesToHex(blake3(pattern(1024)));
  const b = bytesToHex(blake3(pattern(1025)));
  assert.notEqual(a, b);
});
