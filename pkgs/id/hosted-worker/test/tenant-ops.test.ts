// E2 of #9083 (issue #12797): the TenantOpsDO provisioning state
// machine.
//
// Oracle: the hosted-tenancy design record
// (doc/2026-11-17T16-55-00Z_design_hosted-tenancy-architecture)
// section 4 (the lifecycle: transitions, who may do each step,
// intent-first creation, two-phase deletion) and section 11
// item 2 ("tests prove concurrent sign-ups create one database").
//
// Harness style follows test/bridge-do.test.ts: the DO class is
// constructed directly over a fake state whose SQL storage is a
// real node:sqlite database — here with the actual tenancy
// migrations applied (the same files test/tenancy-schema.test.ts
// pins), so the machine is exercised against the real E1 schema,
// never a copy of it. The Turso provider is a fake implementing
// the idempotency contract the design requires of it.
import { test } from "node:test";
import assert from "node:assert/strict";
import { DatabaseSync } from "node:sqlite";
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  HDR_ACTOR_ID,
  HDR_ACTOR_ROLE,
  HDR_TRIGGER,
  MAX_CREATE_ATTEMPTS,
  ProviderError,
  TenantOpsDO,
  provisioningRequestId,
  type TenantDatabaseHandle,
  type TenantDatabaseProvider,
} from "../src/tenant-ops.ts";

const here = dirname(fileURLToPath(import.meta.url));
const migrationsDir = join(here, "..", "tenancy", "migrations");

const BINDING = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER_BINDING = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const ACCOUNT = "acct-1";
const T0 = "2026-11-24T00:00:00.000Z";

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

function fakeTenancyDb(db: DatabaseSync) {
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

class FakeProvider implements TenantDatabaseProvider {
  byRequest = new Map<string, TenantDatabaseHandle>();
  createCalls = 0;
  dropCalls: string[] = [];
  createDelayMs = 0;
  /** Test script for createDatabase; return null to fall through
   * to the default idempotent create. */
  onCreate: ((intent: { requestId: string }) => Promise<TenantDatabaseHandle | null>) | null = null;
  private seq = 0;

  async createDatabase(intent: { requestId: string }): Promise<TenantDatabaseHandle> {
    this.createCalls++;
    if (this.createDelayMs > 0) await new Promise((r) => setTimeout(r, this.createDelayMs));
    if (this.onCreate !== null) {
      const scripted = await this.onCreate(intent);
      if (scripted !== null) return scripted;
    }
    return this.register(intent.requestId);
  }

  register(requestId: string): TenantDatabaseHandle {
    const existing = this.byRequest.get(requestId);
    if (existing) return existing;
    this.seq++;
    const handle = { dbId: `db-${this.seq}`, dbUrl: `https://db-${this.seq}.turso.io` };
    this.byRequest.set(requestId, handle);
    return handle;
  }

  async findDatabase(requestId: string): Promise<TenantDatabaseHandle | null> {
    return this.byRequest.get(requestId) ?? null;
  }

  async dropDatabase(dbId: string): Promise<void> {
    this.dropCalls.push(dbId);
    for (const [requestId, handle] of this.byRequest) {
      if (handle.dbId === dbId) this.byRequest.delete(requestId);
    }
  }
}

function makeOps(db: DatabaseSync, env: Record<string, unknown> = {}, binding: string = BINDING) {
  const state = {
    id: { name: binding },
    storage: { sql: fakeSql(db) },
    blockConcurrencyWhile: async <T>(fn: () => Promise<T>): Promise<T> => fn(),
  };
  return new TenantOpsDO(state as never, { TENANCY_DB: fakeTenancyDb(db), ...env } as never);
}

function seedBinding(db: DatabaseSync, status = "active", binding: string = BINDING) {
  db.prepare(
    "INSERT OR IGNORE INTO account (account_id, sub, email_contact, created_at, state, tier) VALUES (?, ?, NULL, ?, 'active', 'standard')",
  ).run(ACCOUNT, "provider-subject-1", T0);
  db.prepare(
    "INSERT INTO binding (binding_id, account_id, iroh_id, status, bound_at, superseded_by) VALUES (?, ?, 'iroh-public-key', ?, ?, NULL)",
  ).run(binding, ACCOUNT, status, T0);
}

interface Actor {
  role: string;
  id: string;
  trigger?: string;
}
const OWNER: Actor = { role: "owner", id: BINDING };
const PLATFORM: Actor = { role: "platform", id: "tenant-ops" };
const OPERATOR: Actor = { role: "operator", id: "operator-1" };
const ABUSE: Actor = { role: "trigger", id: "abuse", trigger: "abuse" };
const CEILING: Actor = { role: "trigger", id: "ceiling", trigger: "ceiling" };

async function call(
  ops: TenantOpsDO,
  method: string,
  path: string,
  actor: Actor | null,
  body?: Record<string, unknown>,
): Promise<{ status: number; body: Record<string, unknown> }> {
  const headers = new Headers();
  if (actor !== null) {
    headers.set(HDR_ACTOR_ROLE, actor.role);
    headers.set(HDR_ACTOR_ID, actor.id);
    if (actor.trigger !== undefined) headers.set(HDR_TRIGGER, actor.trigger);
  }
  const res = await ops.fetch(
    new Request("https://do" + path, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
    }),
  );
  return { status: res.status, body: (await res.json()) as Record<string, unknown> };
}

const post = (ops: TenantOpsDO, path: string, actor: Actor | null, body?: Record<string, unknown>) =>
  call(ops, "POST", path, actor, body ?? {});

function tenantRow(db: DatabaseSync, binding: string = BINDING) {
  return db.prepare("SELECT * FROM tenant_db WHERE tenant_id = ?").get(binding) as
    | Record<string, unknown>
    | undefined;
}

/** Drive a tenant to `active` the §4 way; returns the provider. */
async function provisionedToActive(db: DatabaseSync, ops: TenantOpsDO) {
  await post(ops, "/v1/tenant/request", OWNER, { jurisdiction: "us", at: T0 });
  await post(ops, "/v1/tenant/provision", PLATFORM);
  await post(ops, "/v1/tenant/bootstrap-observed", PLATFORM, { schemaVersion: 3 });
  const res = await post(ops, "/v1/tenant/activate", PLATFORM);
  assert.equal(res.body.state, "active");
}

test("the §4 lifecycle, end to end, with two-phase deletion", async () => {
  const db = freshTenancyDb();
  seedBinding(db);
  const provider = new FakeProvider();
  const ops = makeOps(db, { TENANT_DB_PROVIDER: provider, TENANT_DELETION_COOLING_OFF_MS: "60000" });

  let res = await post(ops, "/v1/tenant/request", OWNER, { jurisdiction: "eu", at: T0 });
  assert.equal(res.status, 200);
  assert.equal(res.body.state, "requested");
  assert.equal(res.body.jurisdiction, "eu");
  assert.equal(res.body.turso_db_id, null);

  res = await post(ops, "/v1/tenant/provision", PLATFORM);
  assert.equal(res.body.state, "db_ready");
  assert.equal(res.body.turso_db_id, "db-1");
  assert.equal(provider.createCalls, 1);

  res = await post(ops, "/v1/tenant/bootstrap-observed", PLATFORM, { schemaVersion: 3 });
  assert.equal(res.body.schema_version, 3);
  res = await post(ops, "/v1/tenant/activate", PLATFORM);
  assert.equal(res.body.state, "active");

  res = await post(ops, "/v1/tenant/suspend", OPERATOR);
  assert.equal(res.body.state, "suspended");
  // Suspension must not look like loss: the state still answers.
  res = await call(ops, "GET", "/v1/tenant/state", OWNER);
  assert.equal(res.status, 200);
  assert.equal(res.body.state, "suspended");

  res = await post(ops, "/v1/tenant/resume", OPERATOR);
  assert.equal(res.body.state, "active");
  res = await post(ops, "/v1/tenant/export-only", OWNER);
  assert.equal(res.body.state, "export_only");

  // A live credential (metadata only, as E1/E4 keep it) to prove
  // deletion's phase 1 revokes before anything is dropped.
  db.prepare(
    "INSERT INTO credential (credential_id, tenant_id, audience, scope, issued_at, expires_at, revoked_at, issuer, jti) VALUES ('cred-1', ?, 'db-1', 'readwrite', ?, ?, NULL, 'tenant-ops', 'jti-1')",
  ).run(BINDING, T0, "2027-01-01T00:00:00.000Z");

  res = await post(ops, "/v1/tenant/delete", OWNER, { at: "2026-11-25T00:00:00.000Z" });
  assert.equal(res.body.state, "deleting");
  const cred = db.prepare("SELECT revoked_at FROM credential WHERE credential_id = 'cred-1'").get() as {
    revoked_at: string;
  };
  assert.equal(cred.revoked_at, "2026-11-25T00:00:00.000Z", "revocation is phase 1, at initiation");
  assert.deepEqual(provider.dropCalls, [], "nothing is dropped inside the cooling-off window");

  res = await post(ops, "/v1/tenant/delete", PLATFORM, { at: "2026-11-25T00:02:00.000Z" });
  assert.equal(res.body.state, "deleted");
  assert.equal(res.body.deleted_at, "2026-11-25T00:02:00.000Z");
  assert.deepEqual(provider.dropCalls, ["db-1"]);
  // The tombstone: the row is retained, marked deleted.
  const row = tenantRow(db);
  assert.equal(row?.state, "deleted");
  assert.equal(row?.deleted_at, "2026-11-25T00:02:00.000Z");

  const events = (
    db.prepare("SELECT event FROM ops_journal ORDER BY seq").all() as { event: string }[]
  ).map((r) => r.event);
  assert.deepEqual(events, [
    "requested",
    "db_creating",
    "db_ready",
    "schema_bootstrap_observed",
    "active",
    "suspended",
    "active",
    "export_only",
    "deleting",
    "deleted",
  ]);
});

test("tenant requests require an active account in the tenancy registry", async () => {
  // Oracle: the authoritative account.state and binding.status rows in
  // the real tenancy migration jointly decide whether provisioning may start.
  const db = freshTenancyDb();
  seedBinding(db);
  db.prepare("UPDATE account SET state = 'suspended' WHERE account_id = ?").run(ACCOUNT);
  const ops = makeOps(db);

  const res = await post(ops, "/v1/tenant/request", OWNER, { jurisdiction: "us", at: T0 });

  assert.equal(res.status, 200);
  assert.equal(res.body.state, "failed");
  const events = db.prepare("SELECT detail FROM ops_journal WHERE event = 'failed'").all() as { detail: string }[];
  assert.equal(JSON.parse(events[0].detail).reason, "account_not_active");
});

test("tenant operations read binding authority from the shared tenancy database", async () => {
  const tenantStorage = freshTenancyDb();
  const centralTenancy = freshTenancyDb();
  seedBinding(centralTenancy);
  const ops = makeOps(tenantStorage, { TENANCY_DB: fakeTenancyDb(centralTenancy) });

  const res = await post(ops, "/v1/tenant/request", OWNER, { jurisdiction: "us", at: T0 });

  assert.equal(res.status, 200);
  assert.equal(res.body.state, "requested");
  assert.equal(tenantRow(tenantStorage)?.state, "requested");
  assert.equal(
    (centralTenancy.prepare("SELECT binding_id FROM binding WHERE binding_id = ?").get(BINDING) as { binding_id: string })
      .binding_id,
    BINDING,
  );
});

test("concurrent sign-ups create exactly one database (§11 item 2)", async () => {
  const db = freshTenancyDb();
  seedBinding(db);
  const provider = new FakeProvider();
  provider.createDelayMs = 25; // widen the race window: without serialisation, both drivers reach the provider
  const ops = makeOps(db, { TENANT_DB_PROVIDER: provider });

  // Two sign-ups race: duplicate requests, then duplicate
  // provisioning drives, all in flight at once.
  const [r1, r2] = await Promise.all([
    post(ops, "/v1/tenant/request", OWNER, { jurisdiction: "us", at: T0 }),
    post(ops, "/v1/tenant/request", OWNER, { jurisdiction: "us", at: T0 }),
  ]);
  assert.equal(r1.body.state, "requested");
  assert.equal(r2.body.state, "requested");
  const [p1, p2] = await Promise.all([
    post(ops, "/v1/tenant/provision", PLATFORM),
    post(ops, "/v1/tenant/provision", PLATFORM),
  ]);
  assert.equal(p1.body.state, "db_ready");
  assert.equal(p2.body.state, "db_ready");
  assert.equal(p1.body.turso_db_id, p2.body.turso_db_id);
  assert.equal(provider.createCalls, 1, "one binding, one database, however the sign-ups race");
  const count = db.prepare("SELECT COUNT(*) AS n FROM tenant_db").get() as { n: number };
  assert.equal(count.n, 1);

  // A restarted object (a second instance over the same tenancy
  // DB) re-driving provisioning still creates nothing.
  const ops2 = makeOps(db, { TENANT_DB_PROVIDER: provider });
  const p3 = await post(ops2, "/v1/tenant/provision", PLATFORM);
  assert.equal(p3.body.state, "db_ready");
  assert.equal(provider.createCalls, 1);
});

test("intent-first: a crash after creation adopts the existing database", async () => {
  const db = freshTenancyDb();
  seedBinding(db);
  const provider = new FakeProvider();
  // The intent already made a database; the row still says
  // db_creating (the crash took the response with it).
  const handle = provider.register(provisioningRequestId(BINDING));
  db.prepare(
    "INSERT INTO tenant_db (tenant_id, participant_kind, turso_db_id, turso_db_url, jurisdiction, backend, state, schema_version, created_at, deleted_at) VALUES (?, 'user_hosted', NULL, NULL, 'us', 'turso', 'db_creating', 0, ?, NULL)",
  ).run(BINDING, T0);
  const ops = makeOps(db, { TENANT_DB_PROVIDER: provider });
  const res = await post(ops, "/v1/tenant/provision", PLATFORM);
  assert.equal(res.body.state, "db_ready");
  assert.equal(res.body.turso_db_id, handle.dbId);
  assert.equal(provider.createCalls, 0, "adoption, not a second creation");
});

test("a create whose response is lost is adopted, not duplicated", async () => {
  const db = freshTenancyDb();
  seedBinding(db);
  const provider = new FakeProvider();
  provider.onCreate = async (intent) => {
    provider.register(intent.requestId); // the database exists…
    throw new Error("connection reset"); // …but the response never arrived
  };
  const ops = makeOps(db, { TENANT_DB_PROVIDER: provider });
  await post(ops, "/v1/tenant/request", OWNER, { jurisdiction: "us", at: T0 });
  const res = await post(ops, "/v1/tenant/provision", PLATFORM);
  assert.equal(res.body.state, "db_ready");
  assert.equal(provider.createCalls, 1);
});

test("retryable failures retry with the same request id, then succeed", async () => {
  const db = freshTenancyDb();
  seedBinding(db);
  const provider = new FakeProvider();
  const requestIds: string[] = [];
  let failuresLeft = 2;
  provider.onCreate = async (intent) => {
    requestIds.push(intent.requestId);
    if (failuresLeft > 0) {
      failuresLeft--;
      throw new ProviderError("provider busy", true);
    }
    return null;
  };
  const ops = makeOps(db, { TENANT_DB_PROVIDER: provider });
  await post(ops, "/v1/tenant/request", OWNER, { jurisdiction: "us", at: T0 });
  const res = await post(ops, "/v1/tenant/provision", PLATFORM);
  assert.equal(res.body.state, "db_ready");
  assert.equal(provider.createCalls, 3);
  assert.ok(requestIds.every((id) => id === provisioningRequestId(BINDING)));
});

test("retryable failures that outlast the attempts leave db_creating, never failed", async () => {
  const db = freshTenancyDb();
  seedBinding(db);
  const provider = new FakeProvider();
  provider.onCreate = async () => {
    throw new ProviderError("provider busy", true);
  };
  const ops = makeOps(db, { TENANT_DB_PROVIDER: provider });
  await post(ops, "/v1/tenant/request", OWNER, { jurisdiction: "us", at: T0 });
  const res = await post(ops, "/v1/tenant/provision", PLATFORM);
  assert.equal(res.status, 202);
  assert.equal(res.body.state, "db_creating");
  assert.equal(provider.createCalls, MAX_CREATE_ATTEMPTS);
  // The provider heals; the same tenant completes under the same
  // request id, no fresh sign-up needed.
  provider.onCreate = null;
  const res2 = await post(ops, "/v1/tenant/provision", PLATFORM);
  assert.equal(res2.body.state, "db_ready");
});

test("a permanent provider refusal fails the tenant and leaves no database", async () => {
  const db = freshTenancyDb();
  seedBinding(db);
  const provider = new FakeProvider();
  provider.onCreate = async () => {
    throw new ProviderError("jurisdiction not offered", false);
  };
  const ops = makeOps(db, { TENANT_DB_PROVIDER: provider });
  await post(ops, "/v1/tenant/request", OWNER, { jurisdiction: "us", at: T0 });
  const res = await post(ops, "/v1/tenant/provision", PLATFORM);
  assert.equal(res.body.state, "failed");
  assert.equal(await provider.findDatabase(provisioningRequestId(BINDING)), null);
  assert.equal(tenantRow(db)?.turso_db_id, null);
});

test("validation: no database before a binding exists (§4)", async () => {
  const db = freshTenancyDb();
  const provider = new FakeProvider();
  const ops = makeOps(db, { TENANT_DB_PROVIDER: provider });
  // No binding row at all: the request is recorded and fails
  // validation on the row — the diagram's requested ──► failed edge.
  let res = await post(ops, "/v1/tenant/request", OWNER, { jurisdiction: "us", at: T0 });
  assert.equal(res.body.state, "failed");
  res = await post(ops, "/v1/tenant/provision", PLATFORM);
  assert.equal(res.status, 409);
  assert.equal(provider.createCalls, 0);

  // A superseded binding is not an active binding either.
  const db2 = freshTenancyDb();
  seedBinding(db2, "superseded");
  const ops2 = makeOps(db2, { TENANT_DB_PROVIDER: provider });
  res = await post(ops2, "/v1/tenant/request", OWNER, { jurisdiction: "us", at: T0 });
  assert.equal(res.body.state, "failed");
  assert.equal(provider.createCalls, 0);

  // A jurisdiction outside the closed vocabulary refuses before
  // any row exists.
  const db3 = freshTenancyDb();
  seedBinding(db3);
  const ops3 = makeOps(db3, { TENANT_DB_PROVIDER: provider });
  res = await post(ops3, "/v1/tenant/request", OWNER, { jurisdiction: "apac", at: T0 });
  assert.equal(res.status, 403);
  assert.equal(tenantRow(db3), undefined);
});

test("activation waits for observed schema bootstrap (§4 steps 5+7)", async () => {
  const db = freshTenancyDb();
  seedBinding(db);
  const ops = makeOps(db, { TENANT_DB_PROVIDER: new FakeProvider() });
  await post(ops, "/v1/tenant/request", OWNER, { jurisdiction: "us", at: T0 });
  await post(ops, "/v1/tenant/provision", PLATFORM);
  const early = await post(ops, "/v1/tenant/activate", PLATFORM);
  assert.equal(early.status, 409);
  assert.equal(early.body.error, "bootstrap_not_observed");
  assert.equal(tenantRow(db)?.state, "db_ready");
  await post(ops, "/v1/tenant/bootstrap-observed", PLATFORM, {});
  const res = await post(ops, "/v1/tenant/activate", PLATFORM);
  assert.equal(res.body.state, "active");
});

test("only the actors §4 names can perform each step", async () => {
  const db = freshTenancyDb();
  seedBinding(db);
  const provider = new FakeProvider();
  const ops = makeOps(db, { TENANT_DB_PROVIDER: provider, TENANT_DELETION_COOLING_OFF_MS: "0" });

  // Each step at its §4 doorstep: the forbidden actors try first
  // (the tenant must not move), then the named actor performs it.
  const steps: {
    path: string;
    doorstep: string | null;
    forbidden: Actor[];
    legit: Actor;
    body?: Record<string, unknown>;
    lands: string;
  }[] = [
    { path: "/v1/tenant/request", doorstep: null, forbidden: [OPERATOR, PLATFORM, ABUSE], legit: OWNER, body: { jurisdiction: "us", at: T0 }, lands: "requested" },
    { path: "/v1/tenant/provision", doorstep: "requested", forbidden: [OWNER, OPERATOR, ABUSE], legit: PLATFORM, lands: "db_ready" },
    { path: "/v1/tenant/bootstrap-observed", doorstep: "db_ready", forbidden: [OWNER, OPERATOR], legit: PLATFORM, body: {}, lands: "db_ready" },
    { path: "/v1/tenant/activate", doorstep: "db_ready", forbidden: [OWNER, OPERATOR, CEILING], legit: PLATFORM, lands: "active" },
    { path: "/v1/tenant/suspend", doorstep: "active", forbidden: [OWNER, PLATFORM], legit: OPERATOR, lands: "suspended" },
    { path: "/v1/tenant/resume", doorstep: "suspended", forbidden: [OWNER, PLATFORM, ABUSE], legit: OPERATOR, lands: "active" },
    { path: "/v1/tenant/export-only", doorstep: "active", forbidden: [PLATFORM, ABUSE], legit: OWNER, lands: "export_only" },
    { path: "/v1/tenant/delete", doorstep: "export_only", forbidden: [OPERATOR, PLATFORM, CEILING], legit: OWNER, body: { at: T0 }, lands: "deleting" },
  ];
  for (const step of steps) {
    assert.equal(tenantRow(db)?.state ?? null, step.doorstep, `doorstep for ${step.path}`);
    for (const actor of step.forbidden) {
      const res = await post(ops, step.path, actor, { at: T0, jurisdiction: "us" });
      assert.equal(res.status, 403, `${step.path} by ${actor.role}/${actor.id} must refuse`);
      assert.equal(res.body.error, "actor_not_permitted");
      assert.equal(tenantRow(db)?.state ?? null, step.doorstep, `${step.path} by ${actor.role} must not move the tenant`);
    }
    const ok = await post(ops, step.path, step.legit, step.body ?? {});
    assert.equal(ok.status, 200, `${step.path} by its §4 actor`);
    assert.equal(ok.body.state, step.lands);
  }
  // Deletion completion by the holder, window already elapsed (0):
  let res = await post(ops, "/v1/tenant/delete", OWNER, { at: T0 });
  assert.equal(res.body.state, "deleted");

  // An owner of a DIFFERENT binding is not this tenant's owner.
  const db2 = freshTenancyDb();
  seedBinding(db2);
  const ops2 = makeOps(db2, { TENANT_DB_PROVIDER: new FakeProvider() });
  res = await post(ops2, "/v1/tenant/request", { role: "owner", id: OTHER_BINDING }, { jurisdiction: "us" });
  assert.equal(res.status, 403);
  assert.equal(tenantRow(db2), undefined);

  // No actor at all fails closed.
  res = await post(ops2, "/v1/tenant/request", null, { jurisdiction: "us" });
  assert.equal(res.status, 401);
});

test("the named trigger actors work where §4 names them — and only there", async () => {
  const db = freshTenancyDb();
  seedBinding(db);
  const ops = makeOps(db, { TENANT_DB_PROVIDER: new FakeProvider() });
  await provisionedToActive(db, ops);
  let res = await post(ops, "/v1/tenant/suspend", ABUSE);
  assert.equal(res.body.state, "suspended", "abuse may suspend");
  await post(ops, "/v1/tenant/resume", OPERATOR);
  res = await post(ops, "/v1/tenant/export-only", CEILING);
  assert.equal(res.body.state, "export_only", "the ceiling trigger may force export-only");
});

test("deletion refuses before export-only and inside the cooling-off window", async () => {
  const db = freshTenancyDb();
  seedBinding(db);
  const provider = new FakeProvider();
  const ops = makeOps(db, { TENANT_DB_PROVIDER: provider, TENANT_DELETION_COOLING_OFF_MS: "60000" });
  await provisionedToActive(db, ops);
  // From active, deletion is not a §4 transition at all.
  let res = await post(ops, "/v1/tenant/delete", OWNER, { at: T0 });
  assert.equal(res.status, 409);
  assert.equal(res.body.error, "invalid_transition");
  await post(ops, "/v1/tenant/export-only", OWNER);
  res = await post(ops, "/v1/tenant/delete", OWNER, { at: "2026-11-25T00:00:00.000Z" });
  assert.equal(res.body.state, "deleting");
  res = await post(ops, "/v1/tenant/delete", OWNER, { at: "2026-11-25T00:00:30.000Z" });
  assert.equal(res.status, 409);
  assert.equal(res.body.error, "cooling_off");
  assert.deepEqual(provider.dropCalls, [], "the drop waits for the window");
  assert.equal(tenantRow(db)?.state, "deleting");
});

test("deletion refuses when the recorded request time is missing or unreadable (#15028)", async () => {
  for (const corrupt of ["not-a-date", null]) {
    const db = freshTenancyDb();
    seedBinding(db);
    const provider = new FakeProvider();
    const ops = makeOps(db, { TENANT_DB_PROVIDER: provider, TENANT_DELETION_COOLING_OFF_MS: "60000" });
    await provisionedToActive(db, ops);
    await post(ops, "/v1/tenant/export-only", OWNER);
    let res = await post(ops, "/v1/tenant/delete", OWNER, { at: "2026-11-25T00:00:00.000Z" });
    assert.equal(res.body.state, "deleting");
    if (corrupt === null) {
      db.exec("DELETE FROM ops_meta WHERE name = 'deletion_requested_at'");
    } else {
      db.prepare("UPDATE ops_meta SET value = ? WHERE name = 'deletion_requested_at'").run(corrupt);
    }
    // Well past the window by the clock, but the start cannot be proven.
    res = await post(ops, "/v1/tenant/delete", PLATFORM, { at: "2026-11-26T00:00:00.000Z" });
    assert.equal(res.status, 409, `refuses with ${String(corrupt)} as the request time`);
    assert.equal(res.body.error, "cooling_off_unproven");
    assert.deepEqual(provider.dropCalls, [], "nothing is dropped without a provable window");
    assert.equal(tenantRow(db)?.state, "deleting");
  }
});

test("deletion completes once a valid request time is past the window", async () => {
  const db = freshTenancyDb();
  seedBinding(db);
  const provider = new FakeProvider();
  const ops = makeOps(db, { TENANT_DB_PROVIDER: provider, TENANT_DELETION_COOLING_OFF_MS: "60000" });
  await provisionedToActive(db, ops);
  await post(ops, "/v1/tenant/export-only", OWNER);
  await post(ops, "/v1/tenant/delete", OWNER, { at: "2026-11-25T00:00:00.000Z" });
  let res = await post(ops, "/v1/tenant/delete", PLATFORM, { at: "2026-11-25T00:00:59.999Z" });
  assert.equal(res.body.error, "cooling_off", "one millisecond short still refuses");
  res = await post(ops, "/v1/tenant/delete", PLATFORM, { at: "2026-11-25T00:01:00.000Z" });
  assert.equal(res.body.state, "deleted");
  assert.equal(provider.dropCalls.length, 1);
});

test("transitions from the wrong state refuse, and repeats are idempotent", async () => {
  const db = freshTenancyDb();
  seedBinding(db);
  const ops = makeOps(db, { TENANT_DB_PROVIDER: new FakeProvider() });
  await post(ops, "/v1/tenant/request", OWNER, { jurisdiction: "us", at: T0 });
  for (const path of ["/v1/tenant/suspend", "/v1/tenant/resume", "/v1/tenant/export-only", "/v1/tenant/activate"]) {
    const actor = path === "/v1/tenant/activate" ? PLATFORM : OPERATOR;
    const res = await post(ops, path, actor, {});
    assert.equal(res.status, 409, `${path} from requested`);
    assert.equal(res.body.error, "invalid_transition");
  }
  await provisionedToActive(db, ops);
  // Repeats of completed steps answer with the row, not an error.
  let res = await post(ops, "/v1/tenant/request", OWNER, { jurisdiction: "us" });
  assert.equal(res.body.state, "active");
  res = await post(ops, "/v1/tenant/provision", PLATFORM);
  assert.equal(res.body.state, "active");
  res = await post(ops, "/v1/tenant/suspend", OPERATOR);
  assert.equal(res.body.state, "suspended");
  res = await post(ops, "/v1/tenant/suspend", OPERATOR);
  assert.equal(res.body.state, "suspended", "suspend is idempotent");
});

test("no provider configured: provisioning fails closed, nothing is created", async () => {
  const db = freshTenancyDb();
  seedBinding(db);
  const ops = makeOps(db, {});
  await post(ops, "/v1/tenant/request", OWNER, { jurisdiction: "us", at: T0 });
  const res = await post(ops, "/v1/tenant/provision", PLATFORM);
  assert.equal(res.status, 503);
  assert.equal(res.body.error, "provider_unavailable");
  assert.equal(tenantRow(db)?.state, "requested", "the row does not advance on a guess");
});
