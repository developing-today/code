// Hermetic tests for the rehost Worker's protocol helpers.
// Oracle: byte-exact pkt-line framing — the advertisement and request
// bodies are rebuilt here the way git builds them, and the helpers
// must round-trip them exactly.

import assert from "node:assert/strict";
import test from "node:test";

import {
  buildAdvertisement,
  knownOids,
  parsePktLines,
  parseUploadPackRequest,
  pktLine,
} from "../src/protocol.js";

const HEAD = "a".repeat(40);
const TAG_OBJ = "b".repeat(40);
const TAG_COMMIT = "c".repeat(40);
const OTHER = "d".repeat(40);

const DOC = {
  head: HEAD,
  head_ref: "refs/heads/main",
  refs: [
    { name: "refs/heads/main", oid: HEAD, peeled: null },
    { name: "refs/tags/v1", oid: TAG_OBJ, peeled: TAG_COMMIT },
  ],
};

test("pktLine prefixes the byte length including itself", () => {
  assert.equal(pktLine("NAK\n"), "0008NAK\n");
  assert.equal(pktLine(""), "0004");
});

test("advertisement is a well-formed v0 ref listing", () => {
  const body = buildAdvertisement(DOC);
  const lines = parsePktLines(body);
  assert.equal(lines[0], "# service=git-upload-pack\n");
  assert.equal(lines[1], null); // flush after the service line
  assert.ok(lines[2].startsWith(`${HEAD} HEAD\0`));
  assert.ok(lines[2].includes("symref=HEAD:refs/heads/main"));
  assert.ok(lines[2].includes("ofs-delta"));
  assert.equal(lines[3], `${HEAD} refs/heads/main\n`);
  assert.equal(lines[4], `${TAG_OBJ} refs/tags/v1\n`);
  assert.equal(lines[5], `${TAG_COMMIT} refs/tags/v1^{}\n`);
  assert.equal(lines[6], null); // terminating flush
  assert.equal(lines.length, 7);
});

test("parsePktLines rejects malformed lengths", () => {
  assert.throws(() => parsePktLines("zzzz"), /malformed/);
  assert.throws(() => parsePktLines("00ffshort"), /malformed/);
});

test("upload-pack request: wants, haves, done; capabilities stripped", () => {
  const body =
    pktLine(`want ${HEAD}\0multi_ack_detailed side-band-64k\n`) +
    pktLine(`want ${TAG_COMMIT}\n`) +
    "0000" +
    pktLine(`have ${OTHER}\n`) +
    pktLine("done\n");
  const parsed = parseUploadPackRequest(body);
  assert.deepEqual(parsed.wants, [HEAD, TAG_COMMIT]);
  assert.deepEqual(parsed.haves, [OTHER]);
  assert.equal(parsed.done, true);
});

test("knownOids covers tips, peeled tag targets, and HEAD", () => {
  const known = knownOids(DOC);
  assert.ok(known.has(HEAD));
  assert.ok(known.has(TAG_OBJ));
  assert.ok(known.has(TAG_COMMIT));
  assert.ok(!known.has(OTHER));
});
