// BLAKE3 for the bridge (src/blake3.ts).
//
// Oracle: the official BLAKE3 test vectors — the empty-input value
// below is the published vector, the rest are cross-pinned with the
// Rust implementation (p2panda-core's Hash) in
// pkgs/id/tests/bridge_fixtures.rs, which asserts the same table
// from the other side. Input byte i is i mod 251, the vectors'
// input pattern.
import { test } from "node:test";
import assert from "node:assert/strict";
import { blake3 } from "../src/blake3.ts";
import { bytesToHex } from "../src/ed25519.ts";

function pattern(n: number): Uint8Array {
  const out = new Uint8Array(n);
  for (let i = 0; i < n; i++) out[i] = i % 251;
  return out;
}

const VECTORS: [number, string][] = [
  [0, "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"],
  [1, "2d3adedff11b61f14c886e35afa036736dcd87a74d27b5c1510225d0f592e213"],
  [3, "e1be4d7a8ab5560aa4199eea339849ba8e293d55ca0a81006726d184519e647f"],
  [64, "4eed7141ea4a5cd4b788606bd23f46e212af9cacebacdc7d1f4c6dc7f2511b98"],
  [65, "de1e5fa0be70df6d2be8fffd0e99ceaa8eb6e8c93a63f2d8d1c30ecb6b263dee"],
  [100, "8e2eb1bba3040b8f611a1240a0e111c74b45cfc9caed10b95f6372db1c40b8b5"],
  [1023, "10108970eeda3eb932baac1428c7a2163b0e924c9a9e25b35bba72b28f70bd11"],
  [1024, "42214739f095a406f3fc83deb889744ac00df831c10daa55189b5d121c855af7"],
  [1025, "d00278ae47eb27b34faecf67b4fe263f82d5412916c1ffd97c8cb7fb814b8444"],
  [2048, "e776b6028c7cd22a4d0ba182a8bf62205d2ef576467e838ed6f2529b85fba24a"],
  [102400, "bc3e3d41a1146b069abffad3c0d44860cf664390afce4d9661f7902e7943e085"],
];

test("blake3 matches the official test vectors", () => {
  for (const [len, expected] of VECTORS) {
    assert.equal(bytesToHex(blake3(pattern(len))), expected, `length ${len}`);
  }
});

test("blake3 of a chunk boundary differs from its neighbours", () => {
  // The tree shape changes at 1024 bytes; a hash that ignored the
  // chunking would collide across the boundary inputs.
  const a = bytesToHex(blake3(pattern(1024)));
  const b = bytesToHex(blake3(pattern(1025)));
  assert.notEqual(a, b);
});
