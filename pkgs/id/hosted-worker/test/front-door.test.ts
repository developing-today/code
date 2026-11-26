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
import type { TenancyDatabase } from "../src/tenancy-registry.ts";

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

function registryDb(
  resolve: (bindingId: string) => Record<string, unknown> | null = (bindingId) => ({
    binding_id: bindingId,
    binding_status: "active",
    account_state: "active",
  }),
): TenancyDatabase {
  return {
    prepare(_query: string) {
      return {
        bind(bindingId: string) {
          return {
            async first<T extends Record<string, unknown>>(): Promise<T | null> {
              return resolve(bindingId) as T | null;
            },
          };
        },
      };
    },
  };
}

function sqliteTenancyDb(db: DatabaseSync): TenancyDatabase {
  return {
    prepare(query: string) {
      let params: unknown[] = [];
      return {
        bind(...values: Array<string | number | null>) {
          params = values;
          return this;
        },
        async first<T extends Record<string, unknown>>(): Promise<T | null> {
          return (db.prepare(query).get(...(params as never[])) as T | undefined) ?? null;
        },
      };
    },
  };
}

function migrationProjection(record: Record<string, unknown>): HostedDirectoryDatabase {
  return {
    prepare(query: string) {
      return {
        bind(...values: Array<string | number | null>) {
          const bindingId = values[0];
          return {
            async first<T extends Record<string, unknown>>(): Promise<T | null> {
              if (query.includes("legacy_hosted_accounts")) return null;
              return { ...record, binding_id: bindingId } as T;
            },
            async all<T extends Record<string, unknown>>(): Promise<T[]> {
              return [];
            },
          };
        },
      };
    },
  };
}

function directoryFixture() {
  const db = new DatabaseSync(":memory:");
  for (const name of ["0001_hosted_accounts.sql", "0002_legacy_hosted_accounts.sql", "0003_migration_leases.sql"]) {
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

function makeEnv(vars: Record<string, string> = {}, tenancyDb = registryDb()) {
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
  return { env: { USERS: ns, TENANCY_DB: tenancyDb, ...vars }, calls };
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

test("the front door consults the central registry and refuses a missing binding", async () => {
  let lookups = 0;
  const { env, calls } = makeEnv({}, registryDb(() => { lookups++; return null; }));
  const response = await worker.fetch(front("/v1/state"), env);

  assert.equal(response.status, 403);
  assert.equal(((await response.json()) as { error: string }).error, "registry_binding_missing");
  assert.equal(lookups, 1);
  assert.equal(calls.stub, 0, "the namespace is never reached for an unregistered binding");
});

test("the front door resolves account and binding state from the central tenancy schema", async () => {
  const db = new DatabaseSync(":memory:");
  db.exec(readFileSync(new URL("../tenancy/migrations/0001_tenancy_baseline.sql", import.meta.url), "utf8"));
  db.prepare(
    "INSERT INTO account (account_id, sub, email_contact, created_at, state, tier) VALUES ('acct-1', 'subject-1', NULL, '2026-11-26T00:00:00Z', 'active', 'standard')",
  ).run();
  db.prepare(
    "INSERT INTO binding (binding_id, account_id, iroh_id, status, bound_at, superseded_by) VALUES (?, 'acct-1', 'iroh-public-key', 'active', '2026-11-26T00:00:00Z', NULL)",
  ).run(A);
  const { env, calls } = makeEnv({}, sqliteTenancyDb(db));

  try {
    const response = await worker.fetch(front("/v1/state"), env);
    assert.equal(response.status, 200);
    assert.equal(calls.stub, 1);
  } finally {
    db.close();
  }
});

test("the front door refuses inactive central accounts and bindings before namespace access", async () => {
  for (const scenario of [
    { binding_status: "active", account_state: "suspended", error: "registry_account_inactive" },
    { binding_status: "superseded", account_state: "active", error: "registry_binding_inactive" },
  ]) {
    const { env, calls } = makeEnv({}, registryDb((bindingId) => ({ binding_id: bindingId, ...scenario })));
    const response = await worker.fetch(front("/v1/state"), env);
    assert.equal(response.status, 403);
    assert.equal(((await response.json()) as { error: string }).error, scenario.error);
    assert.equal(calls.stub, 0);
  }
});

test("the front door fails closed when the central registry binding is absent", async () => {
  const { env, calls } = makeEnv();
  const response = await worker.fetch(front("/v1/state"), { ...env, TENANCY_DB: undefined });
  assert.equal(response.status, 503);
  assert.equal(((await response.json()) as { error: string }).error, "tenancy_registry_unavailable");
  assert.equal(calls.stub, 0);
});

test("the front door fails closed when the central registry query fails", async () => {
  const { env, calls } = makeEnv({}, registryDb(() => { throw new Error("database offline"); }));
  const response = await worker.fetch(front("/v1/state"), env);
  assert.equal(response.status, 503);
  assert.equal(((await response.json()) as { error: string }).error, "tenancy_registry_unavailable");
  assert.equal(calls.stub, 0);
});

test("a central binding remains routable without a hosted-account projection", async () => {
  const { db, adapter, snapshot } = directoryFixture();
  try {
    const before = snapshot();
    const { env, calls } = makeEnv();
    const response = await worker.fetch(front("/v1/state"), { ...env, HOSTED_DIRECTORY: adapter } as never);
    assert.equal(response.status, 200);
    assert.deepEqual(calls, { stub: 1, jurisdiction: ["us"], idFromName: [A], get: 1 });
    assert.deepEqual(snapshot(), before, "the projection is read-only");
  } finally {
    db.close();
  }
});

for (const scenario of [
  { name: "legacy-only binding", status: 403, error: "legacy_subject_requires_migration" },
  { name: "legacy subject conflict", status: 503, error: "invalid_directory_record" },
  { name: "legacy binding conflict", status: 503, error: "invalid_directory_record" },
  { name: "jurisdiction mismatch", status: 403, error: "directory_jurisdiction_mismatch" },
  ...[
    ["migration fields", "migration_id", "migration-1"],
    ["migration phase", "phase", "moving"],
    ["routing epoch", "routing_epoch", -1],
    ["corrupt subject", "subject_key", "   "],
    ["corrupt jurisdiction", "jurisdiction", "unknown"],
  ].map(([name, column, value]) => ({ name, column, value, status: 503, error: "invalid_directory_record" })),
  { name: "database failure", status: 503, error: "directory_unavailable" },
]) {
  test(`migration projection refuses ${scenario.name} before namespace access`, async () => {
    const { db, adapter, directory, snapshot } = directoryFixture();
    try {
      if (scenario.name === "legacy-only binding") {
        await directory.inventoryLegacy({ subjectKey: "legacy", bindingId: A });
      } else if (scenario.name !== "database failure") {
        await directory.signup({ subjectKey: "subject", bindingId: A, jurisdiction: scenario.name === "jurisdiction mismatch" ? "eu" : "us" });
        if ("column" in scenario) {
          db.exec("PRAGMA ignore_check_constraints = ON");
          db.prepare(`UPDATE hosted_accounts SET ${scenario.column} = ?`).run(scenario.value as string | number);
        }
        if (scenario.name === "legacy subject conflict" || scenario.name === "legacy binding conflict") {
          db.prepare("INSERT INTO legacy_hosted_accounts (subject_key, binding_id, classification) VALUES (?, ?, ?)").run(
            scenario.name === "legacy subject conflict" ? "subject" : "other-subject",
            A,
            "legacy-unrestricted",
          );
        }
      }
      const before = scenario.name === "database failure" ? null : snapshot();
      if (scenario.name === "database failure") db.exec("DROP TABLE hosted_accounts");
      const { env, calls } = makeEnv();
      const response = await worker.fetch(front("/v1/state"), { ...env, HOSTED_DIRECTORY: adapter } as never);
      assert.equal(response.status, scenario.status);
      assert.equal(((await response.json()) as { error: string }).error, scenario.error);
      assert.equal(calls.stub, 0);
      if (before !== null) assert.deepEqual(snapshot(), before, "the projection is read-only");
    } finally {
      db.close();
    }
  });
}

test("the migration projection cannot authorize a binding missing from the central registry", async () => {
  const { env, calls } = makeEnv({}, registryDb(() => null));
  const response = await worker.fetch(front("/v1/state"), {
    ...env,
    HOSTED_DIRECTORY: migrationProjection({
      subject_key: "subject-1",
      binding_id: A,
      jurisdiction: "us",
      jurisdiction_pending: null,
      migration_id: null,
      phase: "stable",
      routing_epoch: 0,
    }),
  } as never);
  assert.equal(response.status, 403);
  assert.equal(((await response.json()) as { error: string }).error, "registry_binding_missing");
  assert.equal(calls.stub, 0);
});

test("the migration projection can pause a centrally registered binding during switch", async () => {
  const { env, calls } = makeEnv();
  const response = await worker.fetch(front("/v1/state"), {
    ...env,
    HOSTED_DIRECTORY: migrationProjection({
      subject_key: "subject-1",
      binding_id: A,
      jurisdiction: "us",
      jurisdiction_pending: "eu",
      migration_id: "migration-1",
      phase: "switched",
      routing_epoch: 2,
    }),
  } as never);
  assert.equal(response.status, 503);
  assert.equal(((await response.json()) as { error: string }).error, "migration_switching");
  assert.equal(calls.stub, 0);
});

test("the central registry stays authoritative while the migration projection controls switch routing", async () => {
  const { db, adapter, directory } = directoryFixture();
  try {
    let record = await directory.signup({ subjectKey: "subject-1", bindingId: A, jurisdiction: "us" });
    record = await directory.transition(record, { type: "begin", migrationId: "migration-1", targetJurisdiction: "eu" });
    record = await directory.transition(record, { type: "freeze-acknowledged", migrationId: "migration-1" });
    record = await directory.transition(record, { type: "snapshot-verified", migrationId: "migration-1" });

    let app = makeEnv({}, registryDb(() => ({ binding_id: A, binding_status: "active", account_state: "active" })));
    const source = await worker.fetch(front("/v1/state"), { ...app.env, HOSTED_DIRECTORY: adapter } as never);
    assert.equal(source.status, 200, "a verified migration continues to route to the source");

    record = await directory.transition(record, { type: "switch", migrationId: "migration-1" });
    app = makeEnv({}, registryDb(() => ({ binding_id: A, binding_status: "active", account_state: "active" })));
    const switching = await worker.fetch(front("/v1/state"), { ...app.env, HOSTED_DIRECTORY: adapter } as never);
    assert.equal(switching.status, 503);
    assert.equal(switching.headers.get("retry-after"), "30");
    assert.equal(((await switching.json()) as { error: string }).error, "migration_switching");
    assert.equal(app.calls.stub, 0, "the projection can pause routing but cannot authorize the binding");
  } finally {
    db.close();
  }
});

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
