// #15479: the front door authenticates callers and resolves the
// binding/jurisdiction server-side from the credential. Oracle:
// client-supplied X-Binding-Id / X-Jurisdiction on their own grant
// NOTHING (the pre-#15479 behaviour let anyone name any binding);
// a valid credential reaches only its own binding; unauthenticated
// and mis-bound requests are refused with an explicit error before
// any namespace access.
import { test } from "node:test";
import assert from "node:assert/strict";
import { DatabaseSync } from "node:sqlite";
import worker from "../src/worker.ts";
import { UserDO, HDR_IROH } from "../src/user-do.ts";

const A = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const TOKEN_A = "caller-token-a";
const TOKEN_B = "caller-token-b";
const IROH_A = "11".repeat(32);

const CALLERS_AB = JSON.stringify([
  { token: TOKEN_A, bindingId: A, jurisdiction: "us", irohId: IROH_A },
  { token: TOKEN_B, bindingId: B, jurisdiction: "eu" },
]);

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
    async head(key: string) {
      const v = objects.get(key);
      return v ? { size: v.byteLength } : null;
    },
    async get(key: string) {
      const v = objects.get(key);
      return v ? { size: v.byteLength, body: new Blob([v]).stream() } : null;
    },
    async put(key: string, value: Uint8Array) {
      objects.set(key, value);
    },
    async delete(key: string) {
      objects.delete(key);
    },
  };
}

/** DO-backed env, like front-door.test.ts. */
function makeEnv(callers: string | undefined = CALLERS_AB) {
  const calls = { stub: 0 };
  const userDO = new UserDO(
    {
      id: { name: A },
      storage: { sql: fakeSql() },
      blockConcurrencyWhile: async <T>(fn: () => Promise<T>): Promise<T> => fn(),
    } as never,
    { BLOBS: fakeBucket() } as never,
  );
  const ns = {
    jurisdiction() {
      return ns;
    },
    idFromName(name: string) {
      return { name };
    },
    get() {
      return {
        fetch: (r: Request) => {
          calls.stub++;
          return userDO.fetch(r);
        },
      };
    },
  };
  const env: Record<string, unknown> = { USERS: ns };
  if (callers !== undefined) env.FRONT_DOOR_CALLERS = callers;
  return { env: env as never, calls };
}

/** Recording-stub env: captures exactly what the front door forwards. */
function recordingEnv(callers: string) {
  const seen = { requests: [] as Request[], names: [] as string[], jurisdictions: [] as string[] };
  const ns = {
    jurisdiction(j: string) {
      seen.jurisdictions.push(j);
      return ns;
    },
    idFromName(name: string) {
      seen.names.push(name);
      return { name };
    },
    get() {
      return {
        fetch: async (r: Request) => {
          seen.requests.push(r);
          return new Response(JSON.stringify({ ok: true }), {
            status: 200,
            headers: { "content-type": "application/json" },
          });
        },
      };
    },
  };
  return { env: { USERS: ns, FRONT_DOOR_CALLERS: callers } as never, seen };
}

function req(path: string, headers: Record<string, string> = {}): Request {
  return new Request("https://hosted.example" + path, { headers });
}

test("forged binding headers without a credential are refused 401 before any namespace access", async () => {
  const { env, calls } = makeEnv();
  const r = await worker.fetch(req("/v1/state", { "x-binding-id": A, "x-jurisdiction": "us" }), env);
  assert.equal(r.status, 401);
  assert.equal(((await r.json()) as { error: string }).error, "unauthenticated_caller");
  assert.equal(calls.stub, 0, "the DO was never reached");
});

test("an unknown credential is refused 401, and a non-Bearer credential is not a credential", async () => {
  const { env, calls } = makeEnv();
  const forged = await worker.fetch(
    req("/v1/state", { authorization: "Bearer not-a-real-token", "x-binding-id": A, "x-jurisdiction": "us" }),
    env,
  );
  assert.equal(forged.status, 401);
  assert.equal(((await forged.json()) as { error: string }).error, "invalid_caller_credential");
  const basic = await worker.fetch(req("/v1/state", { authorization: `Basic ${TOKEN_A}` }), env);
  assert.equal(basic.status, 401);
  assert.equal(((await basic.json()) as { error: string }).error, "unauthenticated_caller");
  assert.equal(calls.stub, 0);
});

test("with no caller directory configured the front door fails closed 503, headers and token notwithstanding", async () => {
  const { env, calls } = makeEnv(undefined);
  const r = await worker.fetch(
    req("/v1/state", { authorization: `Bearer ${TOKEN_A}`, "x-binding-id": A, "x-jurisdiction": "us" }),
    env,
  );
  assert.equal(r.status, 503);
  assert.equal(((await r.json()) as { error: string }).error, "front_door_auth_unconfigured");
  assert.equal(calls.stub, 0);
});

test("a malformed caller directory fails closed 503 instead of admitting a subset", async () => {
  for (const bad of ["not json", "[]", JSON.stringify([{ token: "t", bindingId: "google:123", jurisdiction: "us" }])]) {
    const { env, calls } = makeEnv(bad);
    const r = await worker.fetch(req("/v1/state", { authorization: "Bearer t" }), env);
    assert.equal(r.status, 503, bad);
    assert.equal(((await r.json()) as { error: string }).error, "front_door_auth_misconfigured");
    assert.equal(calls.stub, 0);
  }
});

test("a valid credential with no routing headers at all reaches its own binding, resolved server-side", async () => {
  const { env, calls } = makeEnv();
  const r = await worker.fetch(req("/v1/state", { authorization: `Bearer ${TOKEN_A}` }), env);
  assert.equal(r.status, 200);
  const body = (await r.json()) as { binding_id: string; jurisdiction: string };
  assert.equal(body.binding_id, A);
  assert.equal(body.jurisdiction, "us");
  assert.equal(calls.stub, 1);
});

test("a valid credential naming another binding in a header is refused 403 as mis-bound", async () => {
  const { env, calls } = makeEnv();
  const r = await worker.fetch(
    req("/v1/state", { authorization: `Bearer ${TOKEN_A}`, "x-binding-id": B, "x-jurisdiction": "us" }),
    env,
  );
  assert.equal(r.status, 403);
  assert.equal(((await r.json()) as { error: string }).error, "caller_binding_mismatch");
  assert.equal(calls.stub, 0, "the DO was never reached");
});

test("a valid credential naming another jurisdiction in a header is refused 403 as mis-bound", async () => {
  const { env, calls } = makeEnv();
  const r = await worker.fetch(
    req("/v1/state", { authorization: `Bearer ${TOKEN_A}`, "x-binding-id": A, "x-jurisdiction": "eu" }),
    env,
  );
  assert.equal(r.status, 403);
  assert.equal(((await r.json()) as { error: string }).error, "caller_jurisdiction_mismatch");
  assert.equal(calls.stub, 0);
});

test("each credential routes only to its own binding and jurisdiction", async () => {
  const { env, seen } = recordingEnv(CALLERS_AB);
  const rb = await worker.fetch(req("/v1/state", { authorization: `Bearer ${TOKEN_B}` }), env);
  assert.equal(rb.status, 200);
  assert.deepEqual(seen.names, [B]);
  assert.deepEqual(seen.jurisdictions, ["eu"]);
  // B's credential cannot be widened to A by a header, either.
  const widened = await worker.fetch(
    req("/v1/state", { authorization: `Bearer ${TOKEN_B}`, "x-binding-id": A }),
    env,
  );
  assert.equal(widened.status, 403);
  assert.deepEqual(seen.names, [B], "no further namespace access happened");
});

test("the ID key is forwarded only from the caller record; a client-supplied key is dropped", async () => {
  const { env, seen } = recordingEnv(CALLERS_AB);
  const forgedKey = "22".repeat(32);
  await worker.fetch(
    req("/v1/state", { authorization: `Bearer ${TOKEN_A}`, "x-iroh-id": forgedKey }),
    env,
  );
  assert.equal(seen.requests[0].headers.get(HDR_IROH), IROH_A, "the record's key, not the client's");
  // Caller B's record names no key: a client-supplied key stays dropped.
  await worker.fetch(
    req("/v1/state", { authorization: `Bearer ${TOKEN_B}`, "x-iroh-id": forgedKey }),
    env,
  );
  assert.equal(seen.requests[1].headers.get(HDR_IROH), null);
  assert.equal(seen.requests[1].headers.get("x-iroh-id"), null);
});

test("the caller credential itself is never forwarded to the object", async () => {
  const { env, seen } = recordingEnv(CALLERS_AB);
  await worker.fetch(req("/v1/state", { authorization: `Bearer ${TOKEN_A}` }), env);
  assert.equal(seen.requests[0].headers.get("authorization"), null);
});
