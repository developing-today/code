// #15481: tenancy modules served through the Worker entry point.
//
// Oracle: the issue's acceptance criteria (tenancy modules served;
// correct-module routing with explicit unknown-route failure; tests
// through the Worker fetch handler) plus the hosted-tenancy design
// record (doc/2026-11-17T16-55-00Z_design_hosted-tenancy-architecture
// §§3–4: the tenancy Worker drives TenantOpsDO over internal calls
// with actor headers it mints after authentication — never trusted
// from a client).
//
// Harness style follows test/front-door.test.ts (through the worker
// default export with namespace fakes) and test/tenant-ops.test.ts
// (a real TenantOpsDO over node:sqlite with the real tenancy
// migrations applied, so the entry point is exercised against the
// real E1 schema).
import { test } from "node:test";
import assert from "node:assert/strict";
import { DatabaseSync } from "node:sqlite";
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import worker from "../src/worker.ts";
import { TenantOpsDO } from "../src/tenant-ops.ts";
import type { TenancyDatabase } from "../src/tenancy-registry.ts";

const here = dirname(fileURLToPath(import.meta.url));
const migrationsDir = join(here, "..", "tenancy", "migrations");

const A = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

function freshTenancyDb(): DatabaseSync {
  const db = new DatabaseSync(":memory:");
  db.exec("PRAGMA foreign_keys = ON");
  const files = readdirSync(migrationsDir)
    .filter((f) => f.endsWith(".sql"))
    .sort();
  for (const f of files) db.exec(readFileSync(join(migrationsDir, f), "utf8"));
  return db;
}

function fakeSql(db: DatabaseSync) {
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

/** A TENANT_OPS namespace fake backed by one real TenantOpsDO per
 * binding name, each over its own migrated in-memory SQLite store. */
function tenantOpsNs(tenancyDb: TenancyDatabase) {
  const calls = { get: 0, names: [] as string[] };
  const stores = new Map<string, DatabaseSync>();
  const opsFor = (name: string): TenantOpsDO => {
    let db = stores.get(name);
    if (!db) {
      db = freshTenancyDb();
      stores.set(name, db);
    }
    return new TenantOpsDO(
      {
        id: { name },
        storage: { sql: fakeSql(db) },
        blockConcurrencyWhile: async <T>(fn: () => Promise<T>): Promise<T> => fn(),
      } as never,
      { TENANCY_DB: tenancyDb } as never,
    );
  };
  const ns = {
    jurisdiction(_j: string) {
      return ns;
    },
    idFromName(name: string) {
      calls.names.push(name);
      return { name };
    },
    get(id: { name: string }) {
      calls.get++;
      const ops = opsFor(id.name);
      return { fetch: (r: Request) => ops.fetch(r) };
    },
  };
  return { ns, calls };
}

function makeEnv(tenancyDb: TenancyDatabase = registryDb()) {
  const usersCalls = { get: 0 };
  const users = {
    jurisdiction(_j: string) {
      return users;
    },
    idFromName(_name: string) {
      throw new Error("USERS must not be reached by a tenancy route");
    },
    get(_id: unknown): { fetch(_r: Request): Promise<Response> } {
      usersCalls.get++;
      throw new Error("USERS must not be reached by a tenancy route");
    },
  };
  const { ns, calls } = tenantOpsNs(tenancyDb);
  return { env: { USERS: users, TENANCY_DB: tenancyDb, TENANT_OPS: ns }, calls, usersCalls };
}

function front(path: string, init: RequestInit = {}): Request {
  const headers = new Headers(init.headers);
  headers.set("x-binding-id", A);
  headers.set("x-jurisdiction", "us");
  return new Request("https://hosted.example" + path, { ...init, headers, duplex: "half" } as RequestInit);
}

test("POST /v1/tenant/request is served through the Worker entry point", async () => {
  const { env, calls, usersCalls } = makeEnv();
  const r = await worker.fetch(
    front("/v1/tenant/request", { method: "POST", body: JSON.stringify({ jurisdiction: "us" }) }),
    env as never,
  );
  assert.equal(r.status, 200);
  const body = (await r.json()) as { tenant_id: string; state: string };
  assert.equal(body.tenant_id, A);
  assert.equal(body.state, "requested");
  assert.equal(calls.get, 1, "the request reached TenantOpsDO exactly once");
  assert.deepEqual(calls.names, [A], "the object is named by the authenticated binding");
  assert.equal(usersCalls.get, 0, "the data-plane object was not reached");
});

test("a client-supplied actor role is stripped; the entry point mints owner", async () => {
  const { env } = makeEnv();
  const r = await worker.fetch(
    front("/v1/tenant/request", {
      method: "POST",
      body: JSON.stringify({ jurisdiction: "us" }),
      headers: { "x-tenancy-actor-role": "platform", "x-tenancy-actor-id": "intruder" },
    }),
    env as never,
  );
  // Had the spoofed platform role reached the DO, the owner-only
  // route would refuse with 403 actor_not_permitted. A 200 proves
  // the entry point stripped it and minted owner from the
  // authenticated binding instead.
  assert.equal(r.status, 200);
  assert.equal(((await r.json()) as { state: string }).state, "requested");
});

test("unknown tenancy routes fail explicitly at the entry point", async () => {
  const { env, usersCalls } = makeEnv();
  const r = await worker.fetch(front("/v1/tenant/no-such-route"), env as never);
  assert.equal(r.status, 404);
  assert.equal(((await r.json()) as { error: string }).error, "not_found");
  assert.equal(usersCalls.get, 0, "the data-plane object was not reached");
});

test("tenancy routes stay behind the registry edge", async () => {
  const { env, calls } = makeEnv(registryDb(() => null));
  const r = await worker.fetch(
    front("/v1/tenant/request", { method: "POST", body: JSON.stringify({ jurisdiction: "us" }) }),
    env as never,
  );
  assert.equal(r.status, 403);
  assert.equal(((await r.json()) as { error: string }).error, "registry_binding_missing");
  assert.equal(calls.get, 0, "TenantOpsDO was never reached");
});
