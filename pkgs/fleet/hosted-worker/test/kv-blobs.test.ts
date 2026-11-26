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
import {
  CAPABILITY_TTL_MS,
  decodeKvBlobRef,
  encodeKvBlobRef,
  handleKvBlobRead,
  MemoryKvBlobBucket,
  publishedKey,
} from "../src/kv-blobs.ts";
import { ensureKvMigrated, KV_CATALOG, privateScopePrefix, runKvTransaction, type KvValue } from "../src/kv-door.ts";
import { dispatch } from "../src/router.ts";
import { AUDIENCE, BINDING_A, BINDING_B, ISSUER, WEBHOOK_SECRET, bearerReq, makeHarness } from "./helpers.ts";
import { SqlitePrimary } from "./kv-primary.ts";

const h = await makeHarness();
const REGISTRY = "fleet-kv-central";
const UPLOAD_TOKEN = "svc-token-kv-upload-0123456789abcdef01";
const PIPELINE_ONLY_TOKEN = "svc-token-kv-pipeline-0123456789abcdef";
const ADMIN_TOKEN = "svc-token-kv-admin-0123456789abcdef0123";

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

async function deps(options: { primary?: SqlitePrimary; bucket?: MemoryKvBlobBucket; capabilityKey?: string } = {}) {
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
      {
        sha256: await sha256Hex(ADMIN_TOKEN),
        binding_id: BINDING_A,
        routes: ["kv", "kv-admin"],
        active: true,
        label: "kv-operator",
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
    ...(options.capabilityKey ? { kvBlobCapabilityKey: options.capabilityKey } : {}),
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
  for (const [namespace, scope, jurisdiction] of [
    ["files", "shared:team", "eu"],
    ["notes", null, "eu"],
    ["archive", "shared:archive", "us"],
    ["loose", "shared:loose", null],
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
      ...(jurisdiction === null
        ? []
        : [
            {
              sql: sqlOf("placement.upsert"),
              args: [text(REGISTRY), text(namespace), text(jurisdiction), text("standard"), int(Date.now())],
            },
          ]),
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
    .all()
    .map((row) => ({ ...(row as { content_id: string; jurisdiction: string; tier: string }) }));
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
  const us = (await (
    await dispatch(
      uploadReq(UPLOAD_TOKEN, { bytes, namespace: "archive", scope: "shared:archive", jurisdiction: "us" }),
      d,
    )
  ).json()) as Answer;
  assert.equal(first.deduplicated, false);
  assert.equal(again.deduplicated, true);
  assert.equal(again.reference, first.reference);
  assert.equal(us.deduplicated, false);
  assert.notEqual(us.locator, first.locator);
  assert.equal(bucket.objects.size, 2);
  assert.equal(publishedRows(primary).length, 2);
});

// --- purge -------------------------------------------------------------

/** Retire `digest`'s central record as GC's fenced delete does. */
async function fencedDelete(primary: SqlitePrimary, digest: string): Promise<void> {
  const args: KvValue[] = [
    text("eu"),
    text("standard"),
    text(digest),
    int(0),
    int(Date.now()),
    text(crypto.randomUUID()),
  ];
  await runKvTransaction(primary, [
    { sql: sqlOf("blob.deletion_insert"), args },
    { sql: sqlOf("blob.delete"), args },
  ]);
}

function purgeReq(token: string): Request {
  return bearerReq("/v1/kv/blobs/purge?jurisdiction=eu&tier=standard", token, { method: "POST" });
}

interface Purge {
  purged: string[];
  released: string[];
  more: boolean;
}

function unpurged(primary: SqlitePrimary): number {
  const row = primary.db.prepare("SELECT COUNT(*) AS n FROM kv_blob_deletions WHERE purged_at_ms IS NULL").get() as {
    n: number;
  };
  return Number(row.n);
}

test("a purge removes the bytes of a fenced delete and the content publishes again", async () => {
  const primary = await primaryWithNamespaces();
  const bucket = new MemoryKvBlobBucket();
  const d = await deps({ primary, bucket });
  const bytes = pattern(3000);
  const digest = hex(oneShot(bytes));
  const key = publishedKey("eu", "standard", digest);
  assert.equal((await dispatch(uploadReq(UPLOAD_TOKEN, { bytes }), d)).status, 201);
  await fencedDelete(primary, digest);
  assert.deepEqual(publishedRows(primary), []);
  assert.ok(bucket.objects.has(key), "a fenced delete leaves the bytes");

  // Only kv-admin purges.
  let res = await dispatch(purgeReq(UPLOAD_TOKEN), d);
  assert.equal(res.status, 403);
  assert.equal(((await res.json()) as { error: string }).error, "kv_admin_required");

  res = await dispatch(purgeReq(ADMIN_TOKEN), d);
  assert.equal(res.status, 200, await res.clone().text());
  assert.deepEqual(await res.json(), {
    jurisdiction: "eu",
    tier: "standard",
    purged: [digest],
    released: [],
    more: false,
  });
  assert.equal(bucket.objects.has(key), false);
  assert.equal(unpurged(primary), 0);
  const again = (await (await dispatch(purgeReq(ADMIN_TOKEN), d)).json()) as Purge;
  assert.deepEqual(again.purged, []);

  // Uploading the same bytes again writes them afresh.
  res = await dispatch(uploadReq(UPLOAD_TOKEN, { bytes }), d);
  assert.equal(res.status, 201, await res.clone().text());
  assert.equal(((await res.json()) as Answer).deduplicated, false);
  assert.deepEqual(bucket.objects.get(key), bytes);
  assert.equal(publishedRows(primary).length, 1);
});

test("a purge never removes bytes a published row names, and a held purge refuses the publish", async () => {
  const primary = await primaryWithNamespaces();
  const bucket = new MemoryKvBlobBucket();
  const d = await deps({ primary, bucket });
  const bytes = pattern(4000);
  const digest = hex(oneShot(bytes));
  const key = publishedKey("eu", "standard", digest);
  assert.equal((await dispatch(uploadReq(UPLOAD_TOKEN, { bytes }), d)).status, 201);

  // Published again before the purge: the deletion is skipped.
  await fencedDelete(primary, digest);
  assert.equal((await dispatch(uploadReq(UPLOAD_TOKEN, { bytes }), d)).status, 201);
  let purge = (await (await dispatch(purgeReq(ADMIN_TOKEN), d)).json()) as Purge;
  assert.deepEqual(purge.purged, []);
  assert.ok(bucket.objects.has(key));

  // A purge in flight holds the content: the upload is refused and leaves
  // no staged bytes behind.
  await fencedDelete(primary, digest);
  primary.db
    .prepare("INSERT INTO kv_blob_purges (jurisdiction, tier, content_id, attempt, claimed_ms) VALUES (?, ?, ?, ?, ?)")
    .run("eu", "standard", digest, "another-purge", Date.now());
  const res = await dispatch(uploadReq(UPLOAD_TOKEN, { bytes }), d);
  assert.equal(res.status, 503);
  assert.equal(((await res.json()) as { error: string }).error, "kv_blob_purge_in_progress");
  assert.deepEqual([...bucket.objects.keys()], [key]);
  assert.deepEqual(publishedRows(primary), []);
  // The other purge's claim is fresh, so this purge leaves it alone.
  purge = (await (await dispatch(purgeReq(ADMIN_TOKEN), d)).json()) as Purge;
  assert.deepEqual(purge.purged, []);
  assert.equal(unpurged(primary), 2);
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

test("the upload door places blobs only in the namespace's partition", async () => {
  const primary = await primaryWithNamespaces();
  const bucket = new MemoryKvBlobBucket();
  const d = await deps({ primary, bucket });
  const bytes = pattern(64);
  // Another partition than the namespace's placement.
  let res = await dispatch(uploadReq(UPLOAD_TOKEN, { bytes, jurisdiction: "us" }), d);
  assert.equal(res.status, 403);
  assert.equal(((await res.json()) as { error: string }).error, "partition_mismatch");
  res = await dispatch(uploadReq(UPLOAD_TOKEN, { bytes, tier: "restricted" }), d);
  assert.equal(res.status, 403);
  assert.equal(((await res.json()) as { error: string }).error, "partition_mismatch");
  // A namespace with no placement takes no blob at all.
  res = await dispatch(uploadReq(UPLOAD_TOKEN, { bytes, namespace: "loose", scope: "shared:loose" }), d);
  assert.equal(res.status, 403);
  assert.equal(((await res.json()) as { error: string }).error, "kv_blob_placement_missing");
  // Nothing was staged or published.
  assert.equal(bucket.objects.size, 0);
  assert.equal(publishedRows(primary).length, 0);
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

// --- reads: capability mint and capability-only read ------------------------

const CAPABILITY_KEY = "test-capability-key-0123456789abcdef";

function setHead(
  primary: SqlitePrimary,
  key: string,
  value: string,
  state = "live",
  namespace = "files",
  envelope = "kv:ref-v1",
): void {
  const scope = namespace === "files" ? "shared:team" : `${privateScopePrefix(BINDING_A)}laptop`;
  const bytes = new TextEncoder().encode(value);
  primary.db
    .prepare(
      `INSERT OR REPLACE INTO kv_heads (scope_id, registry_id, namespace, key, version, state, value, writer, written_at_ms)
       VALUES (?, ?, ?, ?, 1, ?, ?, 'test', 0)`,
    )
    .run(scope, REGISTRY, namespace, key, state, bytes);
  primary.db
    .prepare(
      `INSERT OR REPLACE INTO kv_versions (scope_id, registry_id, namespace, key, version, parent_version, state, value,
         writer, host_incarnation, outbox_id, written_at_ms, envelope)
       VALUES (?, ?, ?, ?, 1, 0, ?, ?, 'test', 'test', ?, 0, ?)`,
    )
    .run(scope, REGISTRY, namespace, key, state, bytes, key, envelope);
}

function capabilityReq(token: string, body: unknown): Request {
  return bearerReq("/v1/kv/blobs/capability", token, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
}

interface Minted {
  read_url: string;
  expires_at_ms: number;
  digest: string;
  bytes_len: number;
}

async function uploadedAndHeaded(bytes: Uint8Array) {
  const primary = await primaryWithNamespaces();
  const bucket = new MemoryKvBlobBucket();
  const d = await deps({ primary, bucket, capabilityKey: CAPABILITY_KEY });
  const up = await dispatch(uploadReq(UPLOAD_TOKEN, { bytes }), d);
  assert.equal(up.status, 201, await up.clone().text());
  const answer = (await up.json()) as Answer;
  setHead(primary, "big", answer.reference);
  return { primary, bucket, d, answer };
}

async function mint(d: Awaited<ReturnType<typeof deps>>, key = "big"): Promise<Response> {
  return dispatch(
    capabilityReq(UPLOAD_TOKEN, { registry: REGISTRY, namespace: "files", scope: "shared:team", key }),
    d,
  );
}

function readReq(url: string): Request {
  return new Request(`https://fleet-hosted.test${url}`);
}

test("a minted capability reads the bytes back, checked against the digest", async () => {
  const bytes = pattern(70000);
  const { d, answer } = await uploadedAndHeaded(bytes);
  const res = await mint(d);
  assert.equal(res.status, 200, await res.clone().text());
  const minted = (await res.json()) as Minted;
  assert.equal(minted.digest, answer.digest);
  assert.equal(minted.bytes_len, 70000);
  assert.ok(minted.expires_at_ms > Date.now() && minted.expires_at_ms <= Date.now() + CAPABILITY_TTL_MS);
  // The read needs no credential but the capability.
  const read = await dispatch(readReq(minted.read_url), d);
  assert.equal(read.status, 200, await read.clone().text());
  assert.equal(read.headers.get("content-length"), "70000");
  assert.equal(read.headers.get("cache-control"), "no-store");
  assert.deepEqual(new Uint8Array(await read.arrayBuffer()), bytes);
});

test("a capability that is tampered with or expired reads nothing", async () => {
  const { d, answer } = await uploadedAndHeaded(pattern(100));
  const minted = (await (await mint(d)).json()) as Minted;
  const url = new URL(minted.read_url, "https://fleet-hosted.test");
  const other = hex(oneShot(pattern(99)));
  for (const [name, value] of [
    ["digest", other],
    ["bytes_len", "99"],
    ["jurisdiction", "us"],
    ["tier", "cold"],
    ["expires", String(minted.expires_at_ms + 1)],
    ["mac", "00".repeat(32)],
    ["mac", "not-hex"],
  ] as const) {
    const q = new URLSearchParams(url.search);
    q.set(name, value);
    const res = await dispatch(readReq(`/v1/kv/blobs/read?${q}`), d);
    assert.equal(res.status, 401, `${name}=${value}`);
    assert.equal(((await res.json()) as { error: string }).error, "capability_invalid");
  }
  // A capability for another deployment's key does not verify either.
  const elsewhere = await deps({
    primary: await primaryWithNamespaces(),
    bucket: new MemoryKvBlobBucket(),
    capabilityKey: "another-key-0123456789abcdef0123",
  });
  let res = await dispatch(readReq(minted.read_url), elsewhere);
  assert.equal(res.status, 401);
  // Expired: the same capability, read after its expiry.
  res = await handleKvBlobRead(readReq(minted.read_url), d, () => minted.expires_at_ms).catch(
    (e: { status: number; code: string }) => Response.json({ error: e.code }, { status: e.status }),
  );
  assert.equal(res.status, 401);
  assert.equal(((await res.json()) as { error: string }).error, "capability_expired");
  assert.equal(answer.bytes_len, 100);
});

test("a corrupt stored copy ends the read with an error instead of the value", async () => {
  const bytes = pattern(4096);
  const { d, bucket, answer } = await uploadedAndHeaded(bytes);
  const minted = (await (await mint(d)).json()) as Minted;
  const stored = publishedKey("eu", "standard", answer.digest);
  const flipped = bytes.slice();
  flipped[100] ^= 1;
  bucket.objects.set(stored, flipped);
  const res = await dispatch(readReq(minted.read_url), d);
  assert.equal(res.status, 200);
  await assert.rejects(res.arrayBuffer());
  // A shorter copy fails the same way.
  bucket.objects.set(stored, bytes.slice(0, 4000));
  await assert.rejects((await dispatch(readReq(minted.read_url), d)).arrayBuffer());
  // A missing copy is a 404.
  bucket.objects.delete(stored);
  const gone = await dispatch(readReq(minted.read_url), d);
  assert.equal(gone.status, 404);
});

test("the mint needs a live blob reference the caller may read", async () => {
  const { primary, d, bucket, answer } = await uploadedAndHeaded(pattern(200));
  // A plain value, a tombstone and an absent key are not references, and
  // neither is an inline value that only spells one: the envelope decides.
  setHead(primary, "plain", "just text");
  setHead(primary, "dead", answer.reference, "tombstone");
  setHead(primary, "spelled", answer.reference, "live", "files", "kv:raw-v1");
  for (const key of ["plain", "dead", "spelled", "absent"]) {
    const res = await mint(d, key);
    assert.equal(res.status, 404, key);
    assert.equal(((await res.json()) as { error: string }).error, "no_live_reference");
  }
  // A reference whose object is gone.
  bucket.objects.clear();
  let res = await mint(d);
  assert.equal(res.status, 404);
  assert.equal(((await res.json()) as { error: string }).error, "kv_blob_not_found");
  // The scope must be the namespace's.
  res = await dispatch(
    capabilityReq(UPLOAD_TOKEN, { registry: REGISTRY, namespace: "files", scope: "shared:elsewhere", key: "big" }),
    d,
  );
  assert.equal(res.status, 403);
  // A person may not mint for someone else's private scope.
  const userA = await h.sign(h.rpPayload({ sub: "user-sub-1" }));
  res = await dispatch(
    capabilityReq(userA, {
      registry: REGISTRY,
      namespace: "notes",
      scope: `${privateScopePrefix(BINDING_B)}laptop`,
      key: "big",
    }),
    d,
  );
  assert.equal(res.status, 403);
  assert.equal(((await res.json()) as { error: string }).error, "kv_scope_refused");
  // A malformed body.
  res = await dispatch(capabilityReq(UPLOAD_TOKEN, { namespace: "files" }), d);
  assert.equal(res.status, 400);
  res = await dispatch(capabilityReq(UPLOAD_TOKEN, ["files"]), d);
  assert.equal(res.status, 400);
});

test("a person reads a blob in their own private scope", async () => {
  const primary = await primaryWithNamespaces();
  const bucket = new MemoryKvBlobBucket();
  const d = await deps({ primary, bucket, capabilityKey: CAPABILITY_KEY });
  const userA = await h.sign(h.rpPayload({ sub: "user-sub-1" }));
  const bytes = pattern(300);
  const scope = `${privateScopePrefix(BINDING_A)}laptop`;
  const up = await dispatch(uploadReq(userA, { bytes, namespace: "notes", scope }), d);
  assert.equal(up.status, 201, await up.clone().text());
  setHead(primary, "mine", ((await up.json()) as Answer).reference, "live", "notes");
  const res = await dispatch(capabilityReq(userA, { registry: REGISTRY, namespace: "notes", scope, key: "mine" }), d);
  assert.equal(res.status, 200, await res.clone().text());
  const read = await dispatch(readReq(((await res.json()) as Minted).read_url), d);
  assert.deepEqual(new Uint8Array(await read.arrayBuffer()), bytes);
});

test("both read routes fail closed without a capability key", async () => {
  const primary = await primaryWithNamespaces();
  const d = await deps({ primary, bucket: new MemoryKvBlobBucket() });
  let res = await mint(d);
  assert.equal(res.status, 503);
  assert.equal(((await res.json()) as { error: string }).error, "kv_blobs_not_configured");
  res = await dispatch(readReq("/v1/kv/blobs/read?digest=x"), d);
  assert.equal(res.status, 503);
});

test("the reference decoder takes exactly the encoder's output", () => {
  const ref = { digest: "cd".repeat(32), bytesLen: 12, jurisdiction: "eu", tier: "t1", locator: "r2:kv/blobs/eu/t1/x" };
  assert.deepEqual(decodeKvBlobRef(encodeKvBlobRef(ref)), ref);
  const encoded = encodeKvBlobRef(ref);
  for (const bad of [
    encoded.slice(0, -1),
    encoded.replace("state=published", "state=staged"),
    encoded.replace("profile=raw-blake3-256", "profile=sha256"),
    encoded.replace("bytes_len=12", "bytes_len=-1"),
    encoded.replace("locator=r2:kv/blobs/eu/t1/x", "locator="),
    `extra=1\n${encoded}`,
  ]) {
    assert.equal(decodeKvBlobRef(bad), null, JSON.stringify(bad));
  }
});
