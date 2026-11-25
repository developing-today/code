// Kv blob upload door tests (#13318). Oracle: design
// doc/2026-11-23T12-45-00Z_design_fleet_kv §8 to §10, D011, D012 — the
// endpoint recomputes length and BLAKE3-256 on receipt in chunks,
// refuses bytes that differ from the assertion, deduplicates only
// inside a (jurisdiction, tier) partition, records the object as
// published centrally, and returns the KvBlobRefV1 the client writes by
// publish-CAS. The hasher's oracle is the official BLAKE3 vectors and
// the id worker's one-shot BLAKE3.

import assert from "node:assert/strict";
import { test } from "node:test";
import { blake3 as oneShot } from "../../../id/hosted-worker/src/blake3.ts";
import { sha256Hex } from "../src/auth.ts";
import { Blake3Hasher, hex } from "../src/blake3.ts";
import { configFromEnv } from "../src/config.ts";
import { encodeKvBlobRef, MemoryKvBlobBucket, publishedKey } from "../src/kv-blobs.ts";
import { ensureKvMigrated, KV_CATALOG, privateScopePrefix, runKvTransaction, type KvValue } from "../src/kv-door.ts";
import { dispatch } from "../src/router.ts";
import { AUDIENCE, BINDING_A, BINDING_B, ISSUER, WEBHOOK_SECRET, bearerReq, makeHarness } from "./helpers.ts";
import { SqlitePrimary } from "./kv-primary.ts";

const h = await makeHarness();
const REGISTRY = "fleet-kv-central";
const UPLOAD_TOKEN = "svc-token-kv-upload-0123456789abcdef01";
const PIPELINE_ONLY_TOKEN = "svc-token-kv-pipeline-0123456789abcdef";

// --- the streaming hasher ----------------------------------------------

function pattern(len: number): Uint8Array {
  const bytes = new Uint8Array(len);
  for (let i = 0; i < len; i++) bytes[i] = i % 251;
  return bytes;
}

function streamed(bytes: Uint8Array, piece: number): string {
  const hasher = new Blake3Hasher();
  for (let i = 0; i < bytes.length; i += piece) hasher.update(bytes.subarray(i, i + piece));
  assert.equal(hasher.length, bytes.length);
  return hex(hasher.finalize());
}

test("streaming BLAKE3 matches the official vectors", () => {
  assert.equal(streamed(new Uint8Array(0), 1), "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262");
  assert.equal(
    streamed(new TextEncoder().encode("abc"), 1),
    "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85",
  );
});

test("streaming BLAKE3 equals the one-shot hash at every chunk boundary and split", () => {
  const sizes = [1, 63, 64, 65, 1023, 1024, 1025, 2047, 2048, 2049, 3072, 3073, 4096, 5121, 8193, 31745, 65536, 70000];
  for (const size of sizes) {
    const bytes = pattern(size);
    const expected = hex(oneShot(bytes));
    for (const piece of [1, 7, 64, 1000, 1024, 4096, size]) {
      assert.equal(streamed(bytes, piece), expected, `${size} bytes fed ${piece} at a time`);
    }
  }
});

// --- the door -------------------------------------------------------------

async function deps(options: { primary?: SqlitePrimary; bucket?: MemoryKvBlobBucket } = {}) {
  const fixture = JSON.stringify({
    subjects: [
      {
        issuer: ISSUER,
        subject: "user-sub-1",
        binding_id: BINDING_A,
        active: true,
      },
      {
        issuer: ISSUER,
        subject: "user-sub-2",
        binding_id: BINDING_B,
        active: true,
      },
    ],
    service_tokens: [
      {
        sha256: await sha256Hex(UPLOAD_TOKEN),
        binding_id: BINDING_A,
        routes: ["kv", "kv-upload"],
        active: true,
        label: "kv-host",
      },
      {
        sha256: await sha256Hex(PIPELINE_ONLY_TOKEN),
        binding_id: BINDING_A,
        routes: ["kv"],
        active: true,
        label: "kv-pipeline-only",
      },
    ],
  });
  const config = configFromEnv({
    RP_ISSUER: ISSUER,
    RP_AUDIENCE: AUDIENCE,
    TENANCY_FIXTURE_JSON: fixture,
    GITHUB_WEBHOOK_SECRET: WEBHOOK_SECRET,
  });
  return {
    ...h.deps,
    config,
    ...(options.primary ? { kvPrimary: options.primary } : {}),
    ...(options.bucket ? { kvBlobs: options.bucket } : {}),
  };
}

const text = (value: string): KvValue => ({ type: "text", value });
const int = (value: number): KvValue => ({
  type: "integer",
  value: String(value),
});

function sqlOf(name: string): string {
  const entry = KV_CATALOG.statements.find((s) => s.name === name);
  assert.ok(entry, `catalogue has ${name}`);
  return entry.sql;
}

async function primaryWithNamespaces(): Promise<SqlitePrimary> {
  const primary = new SqlitePrimary();
  await ensureKvMigrated(primary);
  for (const [namespace, scope] of [
    ["files", "shared:team"],
    ["notes", null],
  ] as const) {
    await runKvTransaction(primary, [
      {
        sql: sqlOf("namespace.define"),
        args: [
          text(REGISTRY),
          text(namespace),
          text(scope === null ? "private" : "shared"),
          scope === null ? { type: "null" } : text(scope),
          { type: "null" },
          int(0),
          int(262144),
          int(1),
          int(Date.now()),
        ],
      },
    ]);
  }
  return primary;
}

interface Upload {
  bytes: Uint8Array;
  namespace?: string;
  scope?: string;
  jurisdiction?: string;
  tier?: string;
  digest?: string;
  bytesLen?: number;
}

function uploadReq(token: string, upload: Upload): Request {
  const q = new URLSearchParams({
    registry: REGISTRY,
    namespace: upload.namespace ?? "files",
    scope: upload.scope ?? "shared:team",
    jurisdiction: upload.jurisdiction ?? "eu",
    tier: upload.tier ?? "standard",
    digest: upload.digest ?? hex(oneShot(upload.bytes)),
    bytes_len: String(upload.bytesLen ?? upload.bytes.length),
  });
  return bearerReq(`/v1/kv/blobs?${q}`, token, {
    method: "PUT",
    body: upload.bytes,
  });
}

interface Answer {
  reference: string;
  digest: string;
  bytes_len: number;
  locator: string;
  deduplicated: boolean;
}

function publishedRows(primary: SqlitePrimary): { content_id: string; jurisdiction: string; tier: string }[] {
  return primary.db
    .prepare("SELECT jurisdiction, tier, content_id FROM kv_blobs ORDER BY jurisdiction, content_id")
    .all() as { content_id: string; jurisdiction: string; tier: string }[];
}

test("the upload door verifies, publishes and returns the reference", async () => {
  const primary = await primaryWithNamespaces();
  const bucket = new MemoryKvBlobBucket();
  const d = await deps({ primary, bucket });
  const bytes = pattern(70000);
  const digest = hex(oneShot(bytes));
  const res = await dispatch(uploadReq(UPLOAD_TOKEN, { bytes }), d);
  assert.equal(res.status, 201, await res.clone().text());
  const body = (await res.json()) as Answer;
  const locator = `r2:${publishedKey("eu", "standard", digest)}`;
  assert.equal(body.locator, locator);
  assert.equal(body.deduplicated, false);
  assert.equal(
    body.reference,
    `profile=raw-blake3-256\ndigest=${digest}\nbytes_len=70000\njurisdiction=eu\ntier=standard\nlocator=${locator}\nstate=published\n`,
  );
  // Only the published content key is left: staging is cleared.
  assert.deepEqual([...bucket.objects.keys()], [publishedKey("eu", "standard", digest)]);
  assert.deepEqual(bucket.objects.get(publishedKey("eu", "standard", digest)), bytes);
  assert.deepEqual(publishedRows(primary), [{ jurisdiction: "eu", tier: "standard", content_id: digest }]);
});

test("the upload door refuses bytes that differ from the assertion and keeps nothing", async () => {
  const primary = await primaryWithNamespaces();
  const bucket = new MemoryKvBlobBucket();
  const d = await deps({ primary, bucket });
  const bytes = pattern(3000);
  const wrongDigest = hex(oneShot(pattern(2999)));
  let res = await dispatch(uploadReq(UPLOAD_TOKEN, { bytes, digest: wrongDigest }), d);
  assert.equal(res.status, 422);
  assert.equal(((await res.json()) as { error: string }).error, "blob_digest_mismatch");
  // A body shorter or longer than bytes_len.
  for (const bytesLen of [2999, 3001]) {
    res = await dispatch(
      bearerReq(
        `/v1/kv/blobs?${new URLSearchParams({
          namespace: "files",
          scope: "shared:team",
          jurisdiction: "eu",
          tier: "standard",
          digest: hex(oneShot(bytes)),
          bytes_len: String(bytesLen),
        })}`,
        UPLOAD_TOKEN,
        { method: "PUT", body: bytes },
      ),
      d,
    );
    assert.equal(res.status, 422, `bytes_len ${bytesLen}`);
    assert.equal(((await res.json()) as { error: string }).error, "blob_length_mismatch");
  }
  assert.equal(bucket.objects.size, 0);
  assert.deepEqual(publishedRows(primary), []);
});

test("the upload door deduplicates inside a partition and never across one", async () => {
  const primary = await primaryWithNamespaces();
  const bucket = new MemoryKvBlobBucket();
  const d = await deps({ primary, bucket });
  const bytes = pattern(5000);
  const first = (await (await dispatch(uploadReq(UPLOAD_TOKEN, { bytes }), d)).json()) as Answer;
  const again = (await (await dispatch(uploadReq(UPLOAD_TOKEN, { bytes }), d)).json()) as Answer;
  const us = (await (await dispatch(uploadReq(UPLOAD_TOKEN, { bytes, jurisdiction: "us" }), d)).json()) as Answer;
  assert.equal(first.deduplicated, false);
  assert.equal(again.deduplicated, true);
  assert.equal(again.reference, first.reference);
  assert.equal(us.deduplicated, false);
  assert.notEqual(us.locator, first.locator);
  assert.equal(bucket.objects.size, 2);
  assert.equal(publishedRows(primary).length, 2);
});

test("the upload door checks the caller's scope and route", async () => {
  const primary = await primaryWithNamespaces();
  const bucket = new MemoryKvBlobBucket();
  const d = await deps({ primary, bucket });
  const bytes = pattern(10);
  const userA = await h.sign(h.rpPayload({ sub: "user-sub-1" }));
  // A person may push into their own private scope...
  let res = await dispatch(
    uploadReq(userA, {
      bytes,
      namespace: "notes",
      scope: `${privateScopePrefix(BINDING_A)}laptop`,
    }),
    d,
  );
  assert.equal(res.status, 201, await res.clone().text());
  // ...not into someone else's.
  res = await dispatch(
    uploadReq(userA, {
      bytes,
      namespace: "notes",
      scope: `${privateScopePrefix(BINDING_B)}laptop`,
    }),
    d,
  );
  assert.equal(res.status, 403);
  assert.equal(((await res.json()) as { error: string }).error, "kv_scope_refused");
  // A shared scope that is not the namespace's.
  res = await dispatch(uploadReq(userA, { bytes, scope: "shared:elsewhere" }), d);
  assert.equal(res.status, 403);
  // An undefined namespace.
  res = await dispatch(uploadReq(userA, { bytes, namespace: "nowhere" }), d);
  assert.equal(res.status, 403);
  // A service token needs the kv-upload door in its scope.
  res = await dispatch(uploadReq(PIPELINE_ONLY_TOKEN, { bytes }), d);
  assert.equal(res.status, 403);
  assert.equal(((await res.json()) as { error: string }).error, "service_token_route_scope");
  // No credential at all.
  res = await dispatch(new Request("https://fleet-hosted.test/v1/kv/blobs", { method: "PUT" }), d);
  assert.equal(res.status, 401);
});

test("the upload door refuses a malformed query", async () => {
  const d = await deps({
    primary: await primaryWithNamespaces(),
    bucket: new MemoryKvBlobBucket(),
  });
  const bytes = pattern(4);
  for (const upload of [
    { bytes, jurisdiction: "EU" },
    { bytes, tier: "a/b" },
    { bytes, digest: "abc" },
  ]) {
    const res = await dispatch(uploadReq(UPLOAD_TOKEN, upload), d);
    assert.equal(res.status, 400, JSON.stringify(upload));
    assert.equal(((await res.json()) as { error: string }).error, "kv_blob_malformed");
  }
});

test("the upload door fails closed without a bucket or a primary", async () => {
  const bytes = pattern(4);
  let res = await dispatch(uploadReq(UPLOAD_TOKEN, { bytes }), await deps({ primary: await primaryWithNamespaces() }));
  assert.equal(res.status, 503);
  assert.equal(((await res.json()) as { error: string }).error, "kv_blobs_not_configured");
  res = await dispatch(uploadReq(UPLOAD_TOKEN, { bytes }), await deps({ bucket: new MemoryKvBlobBucket() }));
  assert.equal(res.status, 503);
  assert.equal(((await res.json()) as { error: string }).error, "kv_primary_not_configured");
});

test("the encoded reference has the Rust encoding's exact shape", () => {
  assert.equal(
    encodeKvBlobRef({
      digest: "ab".repeat(32),
      bytesLen: 0,
      jurisdiction: "eu",
      tier: "t1",
      locator: "r2:x",
    }),
    `profile=raw-blake3-256\ndigest=${"ab".repeat(32)}\nbytes_len=0\njurisdiction=eu\ntier=t1\nlocator=r2:x\nstate=published\n`,
  );
});
