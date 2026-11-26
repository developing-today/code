// H5 conformance (#14263, child of #9086; design
// doc/2026-11-24T04-50-00Z_design_sso-relying-party-9086 §4.2/§5):
// the hosted tenant acts as an ordinary LogSync peer. Sync (bridge)
// endpoints authenticate by key-based peer auth only — an Ed25519
// proof by the ID key the binding row names (iroh_id, #2852/#3818
// ceremony) — and bearer <redacted> are neither sent nor accepted on
// the sync path. This file pins that property at the Durable Object
// boundary: a full sync session succeeds with no Authorization
// header anywhere, and a Bearer token, alone or alongside a bad
// proof, never authenticates a sync call.
import { test } from "node:test";
import assert from "node:assert/strict";
import { DatabaseSync } from "node:sqlite";
import { HDR_BINDING, HDR_IROH, HDR_JURISDICTION, HDR_ROUTED, UserDO } from "../src/user-do.ts";
import { authTranscript, bodyHashHex } from "../src/bridge-auth.ts";
import { bytesToHex, importIdKey, type IdKey } from "../src/ed25519.ts";
import { signEvent, type SignedEvent } from "../src/event.ts";
import { encodeBatch } from "../src/bridge-session.ts";

const BINDING = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const BEARER = "Bearer <redacted>";

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

function makeDO() {
  const bucket = fakeBucket();
  const sql = fakeSql();
  const state = {
    id: { name: BINDING },
    storage: { sql },
    blockConcurrencyWhile: async <T>(fn: () => Promise<T>): Promise<T> => fn(),
  };
  return { doA: new UserDO(state as never, { BLOBS_US: bucket } as never), bucket, sql };
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
  context: "push" | "pull" | "heads",
  body: Uint8Array,
): Promise<Record<string, string>> {
  const transcript = authTranscript(BINDING, nonce, context, bodyHashHex(body));
  const signature = await key.sign(transcript);
  return { "x-id-nonce": nonce, "x-id-proof": bytesToHex(signature) };
}

test("sync session: key-based peer auth works with no bearer credential sent", async () => {
  const key = await importIdKey(new Uint8Array(32).fill(61));
  const author = await importIdKey(new Uint8Array(32).fill(62));
  const { doA } = makeDO();
  const event: SignedEvent = await signEvent(author, {
    seq: 0,
    backlink: null,
    kind: "message",
    body: new TextEncoder().encode("h5 conformance"),
  });
  const batch = encodeBatch([event.bytes]);

  // Push: authenticated solely by the bound ID key's proof.
  const nonce = await challenge(doA, key.publicHex);
  const headers = await proofHeaders(key, nonce, "push", batch);
  const pushReq = routed("/v1/bridge/push", key.publicHex, { method: "POST", headers, body: batch as BodyInit });
  // The sync path never sends a bearer credential.
  assert.equal(pushReq.headers.get("authorization"), null);
  const push = await doA.fetch(pushReq);
  assert.equal(push.status, 200);

  // The same author publishing a different event at sequence zero is
  // a fork. The hosted peer must preserve both branches and surface a
  // durable conflict report, matching BridgeStore's fork semantics.
  const rival: SignedEvent = await signEvent(author, {
    seq: 0,
    backlink: null,
    kind: "message",
    body: new TextEncoder().encode("forked author"),
  });
  const rivalBatch = encodeBatch([rival.bytes]);
  const rivalNonce = await challenge(doA, key.publicHex);
  const rivalProof = await proofHeaders(key, rivalNonce, "push", rivalBatch);
  const rivalPush = await doA.fetch(
    routed("/v1/bridge/push", key.publicHex, {
      method: "POST",
      headers: rivalProof,
      body: rivalBatch as BodyInit,
    }),
  );
  assert.equal(rivalPush.status, 200);
  const rivalResult = (await rivalPush.json()) as {
    outcomes: { kind: string }[];
    conflicts: { author: string; seq: number; event_ids: string[] }[];
  };
  assert.equal(rivalResult.outcomes[0].kind, "fork");
  assert.equal(rivalResult.conflicts.length, 1);
  assert.equal(rivalResult.conflicts[0].author, author.publicHex);
  assert.equal(rivalResult.conflicts[0].seq, 0);
  assert.equal(rivalResult.conflicts[0].event_ids.length, 2);

  // The conflict feed is proof-protected like the other hosted sync legs.
  const conflictNonce = await challenge(doA, key.publicHex);
  const conflictHeaders = await proofHeaders(key, conflictNonce, "conflicts", new Uint8Array(0));
  const conflicts = await doA.fetch(
    routed("/v1/bridge/conflicts", key.publicHex, { headers: conflictHeaders }),
  );
  assert.equal(conflicts.status, 200);
  assert.equal(((await conflicts.json()) as { conflicts: unknown[] }).conflicts.length, 1);

  // The hosted ingress gate refuses a tampered signed event before it
  // reaches the store; malformed outcomes are explicit and not applied.
  const tampered = new Uint8Array(event.bytes);
  tampered[tampered.length - 1] ^= 1;
  const invalidBatch = encodeBatch([tampered]);
  const invalidNonce = await challenge(doA, key.publicHex);
  const invalidHeaders = await proofHeaders(key, invalidNonce, "push", invalidBatch);
  const invalidPush = await doA.fetch(
    routed("/v1/bridge/push", key.publicHex, {
      method: "POST",
      headers: invalidHeaders,
      body: invalidBatch as BodyInit,
    }),
  );
  const invalidResult = (await invalidPush.json()) as { outcomes: { kind: string; reason?: string }[] };
  assert.equal(invalidResult.outcomes[0].kind, "refused");
  assert.equal(invalidResult.outcomes[0].reason, "malformed");

  // Heads: same key-auth, still no Authorization header.
  const headsNonce = await challenge(doA, key.publicHex);
  const headsHeaders = await proofHeaders(key, headsNonce, "heads", new Uint8Array(0));
  const headsReq = routed("/v1/bridge/heads", key.publicHex, { headers: headsHeaders });
  assert.equal(headsReq.headers.get("authorization"), null);
  const heads = await doA.fetch(headsReq);
  assert.equal(heads.status, 200);
});

test("sync endpoints do not accept bearer credentials", async () => {
  const key = await importIdKey(new Uint8Array(32).fill(61));
  const wrong = await importIdKey(new Uint8Array(32).fill(63));
  const { doA } = makeDO();
  const batch = encodeBatch([]);
  const bearerInit = { headers: { authorization: BEARER } };

  // Bearer alone on each sync leg: refused as proof-required (401),
  // exactly as if no credential had been presented at all.
  const push = await doA.fetch(
    routed("/v1/bridge/push", key.publicHex, { method: "POST", body: batch as BodyInit, ...bearerInit }),
  );
  assert.equal(push.status, 401);
  assert.equal(((await push.json()) as { error?: string }).error, "bridge_proof_required");
  const heads = await doA.fetch(routed("/v1/bridge/heads", key.publicHex, { ...bearerInit }));
  assert.equal(heads.status, 401);
  const pullBody = new TextEncoder().encode(JSON.stringify({ positions: [] }));
  const pull = await doA.fetch(
    routed("/v1/bridge/pull", key.publicHex, { method: "POST", body: pullBody as BodyInit, ...bearerInit }),
  );
  assert.equal(pull.status, 401);

  // Bearer plus a proof from the wrong key: still refused (403) —
  // the token does not rescue, supplement, or substitute for the
  // key proof.
  const nonce = await challenge(doA, key.publicHex);
  const badHeaders = { ...(await proofHeaders(wrong, nonce, "push", batch)), authorization: BEARER };
  const rescued = await doA.fetch(
    routed("/v1/bridge/push", key.publicHex, { method: "POST", headers: badHeaders, body: batch as BodyInit }),
  );
  assert.equal(rescued.status, 403);
});
