// #11337: the front door (worker.ts default export) must bound body
// memory before the DO's ceiling check can run. These tests go
// through the default export, not UserDO alone, with a namespace fake
// that routes to the real UserDO over in-memory SQLite and R2.
import { test } from "node:test";
import assert from "node:assert/strict";
import { DatabaseSync } from "node:sqlite";
import { readFileSync } from "node:fs";
import worker from "../src/worker.ts";
import { UserDO } from "../src/user-do.ts";
import { PersistentHostedAccountDirectory, type HostedDirectoryDatabase } from "../src/persistent-account-directory.ts";

const A = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

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

function makeEnv(vars: Record<string, string> = {}) {
  const calls = { stub: 0, jurisdiction: [] as string[], idFromName: [] as string[], get: 0 };
  const userDO = new UserDO(
    {
      id: { name: A },
      storage: { sql: fakeSql() },
      blockConcurrencyWhile: async <T>(fn: () => Promise<T>): Promise<T> => fn(),
    } as never,
    { BLOBS: fakeBucket(), ...vars } as never,
  );
  const ns = {
    jurisdiction(j: string) {
      calls.jurisdiction.push(j);
      return ns;
    },
    idFromName(name: string) {
      calls.idFromName.push(name);
      return { name };
    },
    get() {
      calls.get++;
      return {
        fetch: (r: Request) => {
          calls.stub++;
          return userDO.fetch(r);
        },
      };
    },
  };
  return { env: { USERS: ns, ...vars }, calls };
}

function front(path: string, init: RequestInit = {}): Request {
  const headers = new Headers(init.headers);
  headers.set("x-binding-id", A);
  headers.set("x-jurisdiction", "us");
  return new Request("https://hosted.example" + path, { ...init, headers, duplex: "half" } as RequestInit);
}

/** A body stream that records how many bytes were pulled from it. */
function countingBody(chunks: Uint8Array[]) {
  const seen = { bytes: 0 };
  let i = 0;
  const body = new ReadableStream<Uint8Array>(
    {
      pull(c) {
        if (i >= chunks.length) return c.close();
        const chunk = chunks[i++];
        seen.bytes += chunk.byteLength;
        c.enqueue(chunk);
      },
    },
    { highWaterMark: 0 },
  );
  return { body, seen };
}

test("a declared oversized body is refused at the front door without being read", async () => {
  const { env, calls } = makeEnv({ USER_CEILING_BYTES: "10" });
  const { body, seen } = countingBody([new Uint8Array(64)]);
  const r = await worker.fetch(front("/v1/blobs", { method: "PUT", body, headers: { "content-length": "64" } }), env);
  assert.equal(r.status, 413);
  assert.equal(((await r.json()) as { error: string }).error, "body_too_large");
  assert.equal(seen.bytes, 0, "the body was never read");
  assert.equal(calls.stub, 0, "the DO was never reached");
});

test("a chunked oversized body stops being read at the limit plus one chunk", async () => {
  const { env } = makeEnv({ USER_CEILING_BYTES: "10" });
  const chunk = new Uint8Array(4);
  const { body, seen } = countingBody(Array.from({ length: 1000 }, () => chunk));
  const r = await worker.fetch(front("/v1/blobs", { method: "PUT", body }), env);
  assert.equal(r.status, 413);
  assert.ok(seen.bytes <= 10 + 2 * chunk.byteLength, `read ${seen.bytes} bytes of 4000`);

  const kv = countingBody(Array.from({ length: 1000 }, () => chunk));
  const r2 = await worker.fetch(front("/v1/kv/big", { method: "PUT", body: kv.body }), env);
  assert.equal(r2.status, 413);
  assert.ok(kv.seen.bytes <= 10 + 2 * chunk.byteLength, `read ${kv.seen.bytes} bytes of 4000`);
});

test("normal kv and blob PUTs still succeed end to end through the front door", async () => {
  const { env } = makeEnv({ USER_CEILING_BYTES: "100" });
  const kv = await worker.fetch(front("/v1/kv/note", { method: "PUT", body: "hello" }), env);
  assert.equal(kv.status, 200);
  const read = await worker.fetch(front("/v1/kv/note"), env);
  assert.equal(((await read.json()) as { value: string }).value, "hello");
  const blob = await worker.fetch(
    front("/v1/blobs", { method: "PUT", body: new TextEncoder().encode("blobbytes!") }),
    env,
  );
  assert.equal(blob.status, 200);
  const usage = (await (await worker.fetch(front("/v1/usage"), env)).json()) as { storage_bytes: number };
  assert.equal(usage.storage_bytes, 15);
});

test("with no ceiling, normal writes pass and the hard body bound still applies to declared length", async () => {
  const { env, calls } = makeEnv();
  assert.equal((await worker.fetch(front("/v1/kv/k", { method: "PUT", body: "x" }), env)).status, 200);
  const r = await worker.fetch(
    front("/v1/kv/k", { method: "PUT", body: "x", headers: { "content-length": String(200 * 1024 * 1024) } }),
    env,
  );
  assert.equal(r.status, 413);
  assert.equal(calls.stub, 1, "only the normal write reached the DO");
});

// Oracle for #3399: only an existing stable row matching the upstream
// binding/jurisdiction may reach ANY namespace method. Requests are reads
// of the directory: the complete before/after table snapshots must agree.
function directoryFixture() {
  const db = new DatabaseSync(":memory:");
  for (const name of ["0001_hosted_accounts.sql", "0002_legacy_hosted_accounts.sql"]) {
    db.exec(readFileSync(new URL(`../migrations/${name}`, import.meta.url), "utf8"));
  }
  const adapter: HostedDirectoryDatabase = {
    prepare(query) {
      let values: Array<string | number | null> = [];
      return {
        bind(...bound) { values = bound; return this; },
        async first<T extends Record<string, unknown>>() {
          return (db.prepare(query).get(...values) as T | undefined) ?? null;
        },
        async all<T extends Record<string, unknown>>() {
          return db.prepare(query).all(...values) as T[];
        },
      };
    },
  };
  const snapshot = () => ["hosted_accounts", "legacy_hosted_accounts"].map(
    (table) => db.prepare(`SELECT * FROM ${table} ORDER BY subject_key`).all(),
  );
  return { db, adapter, directory: new PersistentHostedAccountDirectory(adapter), snapshot };
}

test("directory guard forwards a matching validated binding through the restricted namespace", async () => {
  const { db, adapter, directory, snapshot } = directoryFixture();
  try {
    await directory.signup({ subjectKey: "subject", bindingId: A, jurisdiction: "us" });
    const before = snapshot();
    const { env, calls } = makeEnv();
    const response = await worker.fetch(front("/v1/state"), { ...env, HOSTED_DIRECTORY: adapter });
    assert.equal(response.status, 200);
    assert.deepEqual(calls, { stub: 1, jurisdiction: ["us"], idFromName: [A], get: 1 });
    assert.deepEqual(snapshot(), before, "routing never mutates directory or inventory");
  } finally { db.close(); }
});

for (const scenario of [
  { name: "missing row", status: 403, error: "directory_account_missing" },
  { name: "legacy-only row", status: 403, error: "legacy_subject_requires_migration" },
  { name: "legacy subject conflict", status: 503, error: "invalid_directory_record" },
  { name: "legacy binding conflict", status: 503, error: "invalid_directory_record" },
  { name: "jurisdiction mismatch", status: 403, error: "directory_jurisdiction_mismatch" },
  ...[
    ["pending jurisdiction", "jurisdiction_pending", "eu"],
    ["migration id", "migration_id", "migration-1"],
    ["migration phase", "phase", "moving"],
    ["routing epoch", "routing_epoch", 1],
    ["corrupt subject", "subject_key", "   "],
    ["corrupt jurisdiction", "jurisdiction", "unknown"],
  ].map(([name, column, value]) => ({ name, column, value, status: 503, error: "invalid_directory_record" })),
  { name: "database failure", status: 503, error: "directory_unavailable" },
]) {
  test(`directory guard refuses ${scenario.name} before namespace access`, async () => {
    const { db, adapter, directory, snapshot } = directoryFixture();
    try {
      if (scenario.name === "legacy-only row") {
        await directory.inventoryLegacy({ subjectKey: "legacy", bindingId: A });
      } else if (scenario.name !== "missing row" && scenario.name !== "database failure") {
        await directory.signup({ subjectKey: "subject", bindingId: A, jurisdiction: scenario.name === "jurisdiction mismatch" ? "eu" : "us" });
        if ("column" in scenario) {
          // Simulate corrupt storage outside the migration's CHECKs: the
          // parser remains a read-time boundary even if schema checks failed.
          db.exec("PRAGMA ignore_check_constraints = ON");
          db.prepare(`UPDATE hosted_accounts SET ${scenario.column} = ?`).run(scenario.value as string | number);
        }
        if (scenario.name === "legacy subject conflict" || scenario.name === "legacy binding conflict") {
          // Bypass the write API to model inconsistent persisted inventory.
          db.prepare("INSERT INTO legacy_hosted_accounts (subject_key, binding_id, classification) VALUES (?, ?, ?)").run(
            scenario.name === "legacy subject conflict" ? "subject" : "other-subject",
            scenario.name === "legacy binding conflict" ? A : "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "legacy-unrestricted",
          );
        }
      }
      const before = snapshot();
      if (scenario.name === "database failure") db.exec("DROP TABLE hosted_accounts");
      const { env, calls } = makeEnv();
      const response = await worker.fetch(front("/v1/state"), { ...env, HOSTED_DIRECTORY: adapter });
      assert.equal(response.status, scenario.status);
      assert.equal(((await response.json()) as { error: string }).error, scenario.error);
      assert.deepEqual(calls, { stub: 0, jurisdiction: [], idFromName: [], get: 0 });
      if (scenario.name !== "database failure") assert.deepEqual(snapshot(), before, "no account created or changed");
      else assert.equal(db.prepare("SELECT COUNT(*) AS n FROM legacy_hosted_accounts").get()?.n, 0);
    } finally { db.close(); }
  });
}

test("a client cannot reach migration control: the front door strips the principal header (#3404)", async () => {
  const { env } = makeEnv();
  const r = await worker.fetch(
    front("/v1/migration/freeze", {
      method: "POST",
      headers: { "x-hosted-migration-principal": "1", "content-type": "application/json" },
      body: JSON.stringify({ migration_id: "m-1", routing_epoch: 1 }),
    }),
    env,
  );
  assert.equal(r.status, 403);
  assert.equal(((await r.json()) as { error: string }).error, "migration_principal_required");
  const write = await worker.fetch(front("/v1/kv/note", { method: "PUT", body: "still open" }), env);
  assert.equal(write.status, 200, "nothing was frozen");
});
