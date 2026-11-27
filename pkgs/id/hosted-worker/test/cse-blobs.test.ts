// §7 client-side-encryption threading through slice 3 of #2852
// (spec 2026-10-30T22-45-00Z_design_hosted_provisioning_sso_binding).
// Oracle: spec §5.1 + §7 — identical plaintext blobs dedupe across
// users; client-encrypted blobs dedupe only per user. The choice is
// made at binding time and, per §7, changes only by export/import
// to a new binding, so the DO binds it into its persisted identity
// and fails closed on a disagreeing routing context. Proven against
// the real UserDO class and the real front door, with in-memory
// SQLite and bucket fakes in the pattern of blobs.test.ts.
import { test } from "node:test";
import assert from "node:assert/strict";
import { DatabaseSync } from "node:sqlite";
import { UserDO, HDR_BINDING, HDR_CSE, HDR_JURISDICTION, HDR_ROUTED, blobObjectKey } from "../src/user-do.ts";
import worker from "../src/worker.ts";

const A = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

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

function makeDO(name: string, bucket: ReturnType<typeof fakeBucket>) {
  const state = {
    id: { name },
    storage: { sql: fakeSql() },
    blockConcurrencyWhile: async <T>(fn: () => Promise<T>): Promise<T> => fn(),
  };
  return new UserDO(state as never, { BLOBS_US: bucket } as never);
}

function routed(path: string, binding: string, cse: string | null = null, init: RequestInit = {}): Request {
  const headers = new Headers(init.headers);
  headers.set(HDR_BINDING, binding);
  headers.set(HDR_JURISDICTION, "us");
  headers.set(HDR_ROUTED, "restricted");
  if (cse !== null) headers.set(HDR_CSE, cse);
  return new Request("https://do" + path, { ...init, headers });
}

async function sha256Hex(bytes: Uint8Array): Promise<string> {
  const d = await crypto.subtle.digest("SHA-256", bytes as BufferSource);
  return [...new Uint8Array(d)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

test("blobObjectKey: plaintext is the bare hash, encrypted is binding-scoped", () => {
  assert.equal(blobObjectKey(A, "ab12", false), "ab12");
  assert.equal(blobObjectKey(A, "ab12", true), `cse/${A}/ab12`);
  assert.notEqual(blobObjectKey(A, "ab12", true), blobObjectKey(B, "ab12", true));
});

test("client-encrypted blobs dedupe only per user (§5.1/§7 oracle)", async () => {
  const bucket = fakeBucket();
  const doA = makeDO(A, bucket);
  const doB = makeDO(B, bucket);
  const bytes = new TextEncoder().encode("identical ciphertext");
  const hash = await sha256Hex(bytes);

  const putA = await doA.fetch(routed("/v1/blobs", A, "1", { method: "PUT", body: bytes }));
  assert.equal(putA.status, 200);
  const putABody = (await putA.json()) as { hash: string; deduped: boolean; client_side_encryption: boolean };
  assert.equal(putABody.hash, hash);
  assert.equal(putABody.deduped, false);
  assert.equal(putABody.client_side_encryption, true);
  assert.deepEqual([...bucket.objects.keys()], [`cse/${A}/${hash}`], "A's ciphertext is stored under A's scope only");

  const putB = await doB.fetch(routed("/v1/blobs", B, "1", { method: "PUT", body: bytes }));
  const putBBody = (await putB.json()) as { deduped: boolean };
  assert.equal(putBBody.deduped, false, "B's identical ciphertext is NOT deduped against A's object");
  assert.equal(bucket.objects.size, 2);
  assert.ok(bucket.objects.has(`cse/${B}/${hash}`));

  const putA2 = await doA.fetch(routed("/v1/blobs", A, "1", { method: "PUT", body: bytes }));
  assert.equal(
    ((await putA2.json()) as { deduped: boolean }).deduped,
    true,
    "A's own re-upload dedupes within A's scope",
  );
  assert.equal(bucket.objects.size, 2);

  assert.equal((await doA.fetch(routed("/v1/blobs/" + hash, A, "1"))).status, 200);
  assert.equal((await doB.fetch(routed("/v1/blobs/" + hash, B, "1"))).status, 200);
});

test("plaintext and client-encrypted copies of identical bytes are separate objects", async () => {
  const bucket = fakeBucket();
  const doA = makeDO(A, bucket);
  const doB = makeDO(B, bucket);
  const bytes = new TextEncoder().encode("same bytes, different choices");
  const hash = await sha256Hex(bytes);

  await doA.fetch(routed("/v1/blobs", A, "0", { method: "PUT", body: bytes }));
  const putB = await doB.fetch(routed("/v1/blobs", B, "1", { method: "PUT", body: bytes }));
  assert.equal(((await putB.json()) as { deduped: boolean }).deduped, false);
  assert.equal(bucket.objects.size, 2);
  assert.ok(bucket.objects.has(hash), "the plaintext copy keeps the bare-hash key");
  assert.ok(bucket.objects.has(`cse/${B}/${hash}`), "the encrypted copy is binding-scoped");
  assert.equal((await doA.fetch(routed("/v1/blobs/" + hash, A, "0"))).status, 200);
  assert.equal((await doB.fetch(routed("/v1/blobs/" + hash, B, "1"))).status, 200);
});

test("the bound CSE choice is immutable identity; an absent context keeps the bound choice", async () => {
  const bucket = fakeBucket();
  const doA = makeDO(A, bucket);
  const bytes = new TextEncoder().encode("bound at first request");
  const hash = await sha256Hex(bytes);

  const put = await doA.fetch(routed("/v1/blobs", A, "1", { method: "PUT", body: bytes }));
  assert.equal(put.status, 200);
  assert.deepEqual([...bucket.objects.keys()], [`cse/${A}/${hash}`]);

  const flip = await doA.fetch(routed("/v1/state", A, "0"));
  assert.equal(flip.status, 409, "a routing context that disagrees with the bound choice fails closed");
  assert.equal(((await flip.json()) as { error: string }).error, "identity_mismatch");

  const state = await doA.fetch(routed("/v1/state", A));
  assert.equal(state.status, 200, "an absent context is not a disagreement");
  assert.equal(((await state.json()) as { client_side_encryption: boolean }).client_side_encryption, true);
  assert.equal((await doA.fetch(routed("/v1/blobs/" + hash, A))).status, 200, "reads keep the bound scoped keying");
});

test("a malformed CSE context fails closed before any route runs", async () => {
  const bucket = fakeBucket();
  const doA = makeDO(A, bucket);
  const res = await doA.fetch(routed("/v1/blobs", A, "yes", { method: "PUT", body: "x" }));
  assert.equal(res.status, 400);
  assert.equal(((await res.json()) as { error: string }).error, "invalid_cse_context");
  assert.equal(bucket.objects.size, 0, "nothing was stored");
});

test("state and export report the bound CSE choice", async () => {
  const bucket = fakeBucket();
  const doA = makeDO(A, bucket);
  const doB = makeDO(B, bucket);
  await doA.fetch(routed("/v1/state", A, "1"));
  await doB.fetch(routed("/v1/state", B));

  const stateA = (await (await doA.fetch(routed("/v1/state", A, "1"))).json()) as { client_side_encryption: boolean };
  const stateB = (await (await doB.fetch(routed("/v1/state", B))).json()) as { client_side_encryption: boolean };
  assert.equal(stateA.client_side_encryption, true);
  assert.equal(stateB.client_side_encryption, false, "no context binds as off — the pre-threading behaviour");

  const exportA = (await (await doA.fetch(routed("/v1/export", A, "1"))).json()) as { client_side_encryption: boolean };
  assert.equal(exportA.client_side_encryption, true, "the §6 export surface names the choice in force");
});

// --- the front door: caller context in, client copies stripped ---

function makeFrontEnv(bucket: ReturnType<typeof fakeBucket>) {
  const userDO = new UserDO(
    {
      id: { name: A },
      storage: { sql: fakeSql() },
      blockConcurrencyWhile: async <T>(fn: () => Promise<T>): Promise<T> => fn(),
    } as never,
    { BLOBS_US: bucket } as never,
  );
  const ns = {
    jurisdiction() {
      return ns;
    },
    idFromName() {
      return { name: A };
    },
    get() {
      return { fetch: (r: Request) => userDO.fetch(r) };
    },
  };
  const TENANCY_DB = {
    prepare() {
      return {
        bind(bindingId: string) {
          return {
            async first() {
              return { binding_id: bindingId, binding_status: "active", account_state: "active" };
            },
          };
        },
      };
    },
  };
  return { USERS: ns, TENANCY_DB };
}

function front(path: string, init: RequestInit = {}): Request {
  const headers = new Headers(init.headers);
  headers.set("x-binding-id", A);
  headers.set("x-jurisdiction", "us");
  return new Request("https://hosted.example" + path, { ...init, headers });
}

test("front door forwards the caller's CSE context to the DO", async () => {
  const bucket = fakeBucket();
  const env = makeFrontEnv(bucket);
  const res = await worker.fetch(front("/v1/state", { headers: { "x-client-side-encryption": "1" } }), env as never);
  assert.equal(res.status, 200);
  assert.equal(((await res.json()) as { client_side_encryption: boolean }).client_side_encryption, true);
});

test("front door strips a client-supplied internal CSE header", async () => {
  const bucket = fakeBucket();
  const env = makeFrontEnv(bucket);
  // The internal header name, supplied directly by the client with
  // no caller context: it must not reach the DO as context.
  const res = await worker.fetch(front("/v1/state", { headers: { [HDR_CSE]: "1" } }), env as never);
  assert.equal(res.status, 200);
  assert.equal(
    ((await res.json()) as { client_side_encryption: boolean }).client_side_encryption,
    false,
    "a client cannot turn on CSE scoping by naming the internal header",
  );
});
