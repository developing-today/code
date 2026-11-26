// §6 export production at the Durable Object boundary (slice 4b,
// #13157): a seeded per-user DO — signed events pushed through the
// bridge, blobs uploaded through the slice-A routes — produces the
// §6 object/blob sets through GET /v1/bridge/export. The produced
// sets are pinned to a committed fixture
// (test/fixtures/hosted-export-sets.json) that the Rust suite in
// pkgs/id/src/hosted_export.rs feeds through build_export +
// verify_import: the two suites pin the same mapping from both
// ends. Regenerate the fixture with UPDATE_EXPORT_FIXTURE=1.
import { test } from "node:test";
import assert from "node:assert/strict";
import { DatabaseSync } from "node:sqlite";
import { readFileSync, writeFileSync } from "node:fs";
import { HDR_BINDING, HDR_IROH, HDR_JURISDICTION, HDR_ROUTED, UserDO } from "../src/user-do.ts";
import { authTranscript, bodyHashHex, type AuthContext } from "../src/bridge-auth.ts";
import { blake3 } from "../src/blake3.ts";
import { bytesToHex, importIdKey, type IdKey } from "../src/ed25519.ts";
import { signEvent, type SignedEvent } from "../src/event.ts";
import { encodeBatch } from "../src/bridge-session.ts";
import type { CborValue } from "../src/canonical-cbor.ts";

const BINDING = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const FIXTURE_PATH = new URL("./fixtures/hosted-export-sets.json", import.meta.url);

/** The fixture author's seed: the Rust fixture test rebuilds the
 * binding with this same seed as its root key. */
const AUTHOR_SEED = new Uint8Array(32).fill(7);

const BLOB_ALPHA = new TextEncoder().encode("export fixture blob alpha");
const BLOB_BETA = new TextEncoder().encode("export fixture blob beta — second content");

function fakeSql() {
  const db = new DatabaseSync(":memory:");
  return {
    exec(query: string, ...params: unknown[]) {
      const stmt = db.prepare(query);
      if (/^\s*select/i.test(query)) {
        const rows = stmt.all(...(params as never[])) as Record<string, unknown>[];
        return { toArray: () => rows };
      }
      stmt.run(...(params as never[]));
      return { toArray: () => [] as Record<string, unknown>[] };
    },
  };
}

function fakeBucket() {
  const objects = new Map<string, Uint8Array>();
  return {
    objects,
    async head(key: string) {
      const v = objects.get(key);
      return v ? { size: v.byteLength } : null;
    },
    async get(key: string) {
      const v = objects.get(key);
      if (!v) return null;
      return {
        size: v.byteLength,
        body: new ReadableStream({
          start(c) {
            c.enqueue(v);
            c.close();
          },
        }),
      };
    },
    async put(key: string, value: ArrayBuffer | Uint8Array) {
      objects.set(key, value instanceof Uint8Array ? value : new Uint8Array(value));
    },
    async delete(key: string) {
      objects.delete(key);
    },
  };
}

type Bucket = ReturnType<typeof fakeBucket>;
type Sql = ReturnType<typeof fakeSql>;

function makeDO(sql: Sql, bucket: Bucket, env: Record<string, unknown> = {}) {
  const state = {
    id: { name: BINDING },
    storage: { sql },
    blockConcurrencyWhile: async <T>(fn: () => Promise<T>): Promise<T> => fn(),
  };
  return new UserDO(state as never, { BLOBS_US: bucket, ...env } as never);
}

function routed(path: string, irohHex: string | null, init: RequestInit = {}): Request {
  const headers = new Headers(init.headers);
  headers.set(HDR_BINDING, BINDING);
  headers.set(HDR_JURISDICTION, "us");
  headers.set(HDR_ROUTED, "restricted");
  if (irohHex !== null) headers.set(HDR_IROH, irohHex);
  return new Request("https://do" + path, { ...init, headers });
}

async function challenge(doA: UserDO, irohHex: string): Promise<string> {
  const res = await doA.fetch(routed("/v1/bridge/challenge", irohHex));
  assert.equal(res.status, 200);
  const body = (await res.json()) as { nonce: string };
  return body.nonce;
}

async function proofHeaders(
  key: IdKey,
  nonce: string,
  context: AuthContext,
  body: Uint8Array,
): Promise<Record<string, string>> {
  const transcript = authTranscript(BINDING, nonce, context, bodyHashHex(body));
  const signature = await key.sign(transcript);
  return { "x-id-nonce": nonce, "x-id-proof": bytesToHex(signature) };
}

async function sha256Hex(bytes: Uint8Array): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", bytes as BufferSource);
  return bytesToHex(new Uint8Array(digest));
}

const T = (v: string): CborValue => ({ kind: "text", value: v });
const B = (v: Uint8Array): CborValue => ({ kind: "bytes", value: v });
const U = (v: bigint): CborValue => ({ kind: "uint", value: v });

function documentExtra(content: Uint8Array, size: number): [string, CborValue] {
  return [
    "document",
    {
      kind: "map",
      value: [
        [T("media_type"), T("text/plain")],
        [T("content"), B(blake3(content))],
        [T("size"), U(BigInt(size))],
      ],
    },
  ];
}

interface Seeded {
  doA: UserDO;
  sql: Sql;
  bucket: Bucket;
  key: IdKey;
  events: SignedEvent[];
  blobHashes: { alpha: string; beta: string };
}

/** Seeds the fixture store: two blobs through the slice-A routes,
 * then a four-event chain through the bridge — a plain message, a
 * document citing alpha, a document citing beta, and a second
 * document citing alpha again. */
async function seedStore(): Promise<Seeded> {
  const sql = fakeSql();
  const bucket = fakeBucket();
  const doA = makeDO(sql, bucket);
  const key = await importIdKey(AUTHOR_SEED);

  const putBlob = async (bytes: Uint8Array): Promise<string> => {
    const res = await doA.fetch(routed("/v1/blobs", null, { method: "PUT", body: bytes as BodyInit }));
    assert.equal(res.status, 200);
    const body = (await res.json()) as { hash: string };
    return body.hash;
  };
  const alpha = await putBlob(BLOB_ALPHA);
  const beta = await putBlob(BLOB_BETA);
  assert.equal(alpha, await sha256Hex(BLOB_ALPHA));
  assert.equal(beta, await sha256Hex(BLOB_BETA));

  const drafts: [string, [string, CborValue] | null][] = [
    ["message", null],
    ["document.put", documentExtra(BLOB_ALPHA, BLOB_ALPHA.byteLength)],
    ["document.put", documentExtra(BLOB_BETA, BLOB_BETA.byteLength)],
    ["document.put", documentExtra(BLOB_ALPHA, BLOB_ALPHA.byteLength)],
  ];
  const events: SignedEvent[] = [];
  for (let seq = 0; seq < drafts.length; seq++) {
    const [kind, extra] = drafts[seq];
    events.push(
      await signEvent(key, {
        seq,
        backlink: seq === 0 ? null : events[seq - 1].idBytes,
        kind,
        extra: extra ? [extra] : [],
      }),
    );
  }
  const batch = encodeBatch(events.map((e) => e.bytes));
  const nonce = await challenge(doA, key.publicHex);
  const headers = await proofHeaders(key, nonce, "push", batch);
  const push = await doA.fetch(
    routed("/v1/bridge/push", key.publicHex, { method: "POST", headers, body: batch as BodyInit }),
  );
  assert.equal(push.status, 200);
  const pushBody = (await push.json()) as { outcomes: { kind: string }[] };
  assert.deepEqual(
    pushBody.outcomes.map((o) => o.kind),
    ["accepted", "accepted", "accepted", "accepted"],
  );
  return { doA, sql, bucket, key, events, blobHashes: { alpha, beta } };
}

async function callExport(doA: UserDO, key: IdKey): Promise<Response> {
  const nonce = await challenge(doA, key.publicHex);
  const headers = await proofHeaders(key, nonce, "export", new Uint8Array(0));
  return doA.fetch(routed("/v1/bridge/export", key.publicHex, { headers }));
}

interface WireSets {
  objects: { hash: string; signer: string; signature: string; blob_refs: string[]; bytes: number[] }[];
  blobs: { hash: string; bytes: number[] }[];
  object_count: number;
  blob_count: number;
  objects_digest: string;
}

test("export production: seeded DO yields the §6 sets, pinned to the fixture", async () => {
  const { doA, key, events, blobHashes } = await seedStore();
  const res = await callExport(doA, key);
  assert.equal(res.status, 200);
  const wire = (await res.json()) as WireSets;

  assert.equal(wire.object_count, 4);
  assert.equal(wire.blob_count, 2);
  assert.equal(wire.objects.length, 4);
  assert.equal(wire.blobs.length, 2);

  // Canonical (hash) order, strictly ascending.
  const hashes = wire.objects.map((o) => o.hash);
  assert.deepEqual(hashes, [...hashes].sort());
  const blobHashList = wire.blobs.map((b) => b.hash);
  assert.deepEqual(blobHashList, [...blobHashList].sort());

  // Every object maps its stored envelope verbatim: bytes are the
  // pushed bytes, signer is the author, signature is the envelope's
  // own — nothing re-encoded, nothing re-signed.
  const byHash = new Map(wire.objects.map((o) => [o.hash, o]));
  for (const event of events) {
    const object = byHash.get(await sha256Hex(event.bytes));
    assert.ok(object, `event ${event.id} is in the export`);
    assert.deepEqual(object.bytes, [...event.bytes]);
    assert.equal(object.signer, event.authorHex);
    assert.equal(object.signature, event.signatureHex);
  }

  // Citations resolved by content to the §6 (SHA-256) blob names:
  // alpha cited by two objects, beta by one; each blob carried once.
  const docRefs = wire.objects.filter((o) => o.blob_refs.length > 0).map((o) => o.blob_refs[0]);
  assert.deepEqual(docRefs.sort(), [blobHashes.alpha, blobHashes.alpha, blobHashes.beta].sort());
  assert.deepEqual(blobHashList, [blobHashes.alpha, blobHashes.beta].sort());
  const blobBytes = new Map(wire.blobs.map((b) => [b.hash, b.bytes]));
  assert.deepEqual(blobBytes.get(blobHashes.alpha), [...BLOB_ALPHA]);
  assert.deepEqual(blobBytes.get(blobHashes.beta), [...BLOB_BETA]);

  // The digest is SHA-256 of the concatenated object hash bytes in
  // canonical order, computed here independently of the producer.
  const concatenated = new Uint8Array(hashes.length * 32);
  hashes.forEach((hash, i) => {
    for (let j = 0; j < 32; j++) {
      concatenated[i * 32 + j] = parseInt(hash.slice(j * 2, j * 2 + 2), 16);
    }
  });
  assert.equal(wire.objects_digest, await sha256Hex(concatenated));

  // The cross-language contract: the Rust suite verifies an export
  // assembled from exactly these sets.
  if (process.env.UPDATE_EXPORT_FIXTURE) {
    writeFileSync(FIXTURE_PATH, JSON.stringify(wire, null, 2) + "\n");
  } else {
    const fixture = JSON.parse(readFileSync(FIXTURE_PATH, "utf8")) as WireSets;
    assert.deepEqual(wire, fixture);
  }
});

test("export stays available from a read-only, at-cap store", async () => {
  const { doA, sql, bucket, key } = await seedStore();
  const usage = (await (await doA.fetch(routed("/v1/usage", null))).json()) as {
    storage_bytes: number;
  };
  // A second DO over the same store, capped at exactly its current
  // usage: writes fail closed there, reads and export remain.
  const capped = makeDO(sql, bucket, { USER_CEILING_BYTES: String(usage.storage_bytes) });
  const res = await callExport(capped, key);
  assert.equal(res.status, 200);
  const wire = (await res.json()) as WireSets;
  assert.equal(wire.object_count, 4);
  assert.equal(wire.blob_count, 2);
});

test("a referenced blob R2 cannot produce fails the export loudly, naming the hash", async () => {
  const { doA, bucket, key, blobHashes } = await seedStore();
  bucket.objects.delete(blobHashes.beta);
  const res = await callExport(doA, key);
  assert.equal(res.status, 502);
  const body = (await res.json()) as { error: string; message: string };
  assert.equal(body.error, "export_production_failed");
  assert.ok(body.message.includes(blobHashes.beta), `failure names the missing blob hash: ${body.message}`);
});

test("an object citing content no referenced blob produces fails loudly, naming the cited hash", async () => {
  const sql = fakeSql();
  const bucket = fakeBucket();
  const doA = makeDO(sql, bucket);
  const key = await importIdKey(AUTHOR_SEED);
  const ghost = new TextEncoder().encode("content that was never uploaded");
  const event = await signEvent(key, {
    seq: 0,
    kind: "document.put",
    extra: [documentExtra(ghost, ghost.byteLength)],
  });
  const batch = encodeBatch([event.bytes]);
  const nonce = await challenge(doA, key.publicHex);
  const headers = await proofHeaders(key, nonce, "push", batch);
  const push = await doA.fetch(
    routed("/v1/bridge/push", key.publicHex, { method: "POST", headers, body: batch as BodyInit }),
  );
  assert.equal(push.status, 200);
  const res = await callExport(doA, key);
  assert.equal(res.status, 502);
  const body = (await res.json()) as { error: string; message: string };
  assert.equal(body.error, "export_production_failed");
  assert.ok(body.message.includes(bytesToHex(blake3(ghost))), `failure names the cited content hash: ${body.message}`);
});

test("the export leg fails closed without a proof, like every bridge leg", async () => {
  const { doA, key } = await seedStore();
  const res = await doA.fetch(routed("/v1/bridge/export", key.publicHex));
  assert.equal(res.status, 401);
});
