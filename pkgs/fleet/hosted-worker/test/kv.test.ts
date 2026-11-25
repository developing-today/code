// Kv door tests (#13316). Oracle: design doc/2026-11-23T12-45-00Z_design_fleet_kv
// §9 / D011 and #13316 acceptance 5 — central access reuses the hosted
// RP/agent JWT or route-scoped service token path only, a per-user
// Turso data-plane credential is refused with its typed rule, and the
// door forwards nothing outside the fleet-kv statement catalogue. The
// primary here is node:sqlite behind a small Hrana interpreter, so the
// catalogue's statements and migrations run against real SQLite.

import assert from "node:assert/strict";
import { DatabaseSync } from "node:sqlite";
import { test } from "node:test";
import { sha256Hex } from "../src/auth.ts";
import { configFromEnv } from "../src/config.ts";
import { KV_CATALOG, kvPipelineBody, privateScopePrefix, type KvPrimary, type KvValue } from "../src/kv-door.ts";
import { dispatch } from "../src/router.ts";
import {
  AUDIENCE,
  BINDING_A,
  BINDING_B,
  ISSUER,
  SERVICE_TOKEN,
  WEBHOOK_SECRET,
  bearerReq,
  makeHarness,
} from "./helpers.ts";

const h = await makeHarness();
const REGISTRY = "fleet-kv-central";
const KV_SERVICE_TOKEN = "svc-token-kv-door-0123456789abcdef0123";
const KV_ADMIN_SERVICE_TOKEN = "svc-token-kv-admin-0123456789abcdef01";

// --- a Hrana v2 pipeline over node:sqlite ---------------------------

type Cell = KvValue;

function toSqlite(value: KvValue): unknown {
  switch (value.type) {
    case "null":
      return null;
    case "integer":
      return BigInt(value.value);
    case "float":
      return value.value;
    case "text":
      return value.value;
    case "blob":
      return Uint8Array.from(atob(value.base64), (c) => c.charCodeAt(0));
  }
}

function fromSqlite(value: unknown): Cell {
  if (value === null || value === undefined) return { type: "null" };
  if (typeof value === "bigint") return { type: "integer", value: value.toString() };
  if (typeof value === "number") {
    return Number.isInteger(value) ? { type: "integer", value: String(value) } : { type: "float", value };
  }
  if (typeof value === "string") return { type: "text", value };
  const bytes = value as Uint8Array;
  let bin = "";
  for (const b of bytes) bin += String.fromCharCode(b);
  return { type: "blob", base64: btoa(bin) };
}

class SqlitePrimary implements KvPrimary {
  readonly db = new DatabaseSync(":memory:");
  readonly bodies: unknown[] = [];

  private run(stmt: { sql: string; args?: KvValue[] }): { cols: { name: string }[]; rows: Cell[][] } {
    const sql = stmt.sql.trim();
    if (/^(BEGIN|COMMIT|ROLLBACK)\b/i.test(sql)) {
      this.db.exec(sql);
      return { cols: [], rows: [] };
    }
    const prepared = this.db.prepare(sql);
    const records = prepared.all(...((stmt.args ?? []).map(toSqlite) as never[])) as Record<string, unknown>[];
    const names = records[0] ? Object.keys(records[0]) : [];
    return { cols: names.map((name) => ({ name })), rows: records.map((r) => Object.values(r).map(fromSqlite)) };
  }

  async pipeline(body: unknown): Promise<{ status: number; body: unknown }> {
    this.bodies.push(body);
    const results: unknown[] = [];
    for (const request of (body as { requests: Record<string, unknown>[] }).requests) {
      if (request.type === "close") {
        results.push({ type: "ok", response: { type: "close" } });
      } else if (request.type === "execute") {
        try {
          const result = this.run(request.stmt as { sql: string; args: KvValue[] });
          results.push({ type: "ok", response: { type: "execute", result } });
        } catch (e) {
          results.push({ type: "error", error: { message: String(e) } });
        }
      } else if (request.type === "batch") {
        const steps = (request.batch as { steps: Record<string, unknown>[] }).steps;
        const ok: boolean[] = [];
        const stepResults: unknown[] = [];
        const stepErrors: unknown[] = [];
        const holds = (cond: Record<string, unknown> | undefined): boolean => {
          if (!cond) return true;
          if (cond.type === "ok") return ok[cond.step as number] === true;
          if (cond.type === "not") return !holds(cond.cond as Record<string, unknown>);
          throw new Error("unsupported condition");
        };
        for (const step of steps) {
          if (!holds(step.condition as Record<string, unknown> | undefined)) {
            ok.push(false);
            stepResults.push(null);
            stepErrors.push(null);
            continue;
          }
          try {
            stepResults.push(this.run(step.stmt as { sql: string; args?: KvValue[] }));
            stepErrors.push(null);
            ok.push(true);
          } catch (e) {
            stepResults.push(null);
            stepErrors.push({ message: String(e) });
            ok.push(false);
          }
        }
        results.push({
          type: "ok",
          response: { type: "batch", result: { step_results: stepResults, step_errors: stepErrors } },
        });
      } else {
        results.push({ type: "error", error: { message: "unsupported request" } });
      }
    }
    return { status: 200, body: { baton: null, base_url: null, results } };
  }

  migrationBatches(): number {
    return this.bodies.filter((b) => JSON.stringify(b).includes(KV_CATALOG.migrations.record)).length;
  }
}

// --- callers ----------------------------------------------------------

async function depsWith(primary?: KvPrimary) {
  const fixture = JSON.stringify({
    subjects: [
      { issuer: ISSUER, subject: "user-sub-1", binding_id: BINDING_A, active: true },
      { issuer: ISSUER, subject: "user-sub-2", binding_id: BINDING_B, active: true },
    ],
    service_tokens: [
      {
        sha256: await sha256Hex(KV_SERVICE_TOKEN),
        binding_id: BINDING_A,
        routes: ["kv"],
        active: true,
        label: "kv-host",
      },
      {
        sha256: await sha256Hex(KV_ADMIN_SERVICE_TOKEN),
        binding_id: BINDING_A,
        routes: ["kv", "kv-admin"],
        active: true,
        label: "kv-admin",
      },
    ],
  });
  const config = configFromEnv({
    RP_ISSUER: ISSUER,
    RP_AUDIENCE: AUDIENCE,
    TENANCY_FIXTURE_JSON: fixture,
    GITHUB_WEBHOOK_SECRET: WEBHOOK_SECRET,
  });
  return { ...h.deps, config, ...(primary ? { kvPrimary: primary } : {}) };
}

const userA = await h.sign(h.rpPayload({ sub: "user-sub-1" }));
const adminA = await h.sign(h.rpPayload({ sub: "user-sub-1", roles: ["owner", "kv-admin"] }));

const text = (value: string): KvValue => ({ type: "text", value });
const int = (value: number): KvValue => ({ type: "integer", value: String(value) });
const blob = (value: string): KvValue => ({ type: "blob", base64: btoa(value) });

function sqlOf(name: string): string {
  const entry = KV_CATALOG.statements.find((s) => s.name === name);
  assert.ok(entry, `catalogue has ${name}`);
  return entry.sql;
}

function argsFor(name: string, all: KvValue[]): KvValue[] {
  const entry = KV_CATALOG.statements.find((s) => s.name === name);
  assert.ok(entry);
  return all.slice(0, entry.args);
}

function defineNamespace(namespace: string, scope: string | null) {
  const args = [
    text(REGISTRY),
    text(namespace),
    text(scope === null ? "private" : "shared"),
    scope === null ? ({ type: "null" } as KvValue) : text(scope),
    { type: "null" } as KvValue,
    int(0),
    int(262144),
    int(1),
    int(Date.now()),
  ];
  return [
    { sql: sqlOf("namespace.define"), args },
    { sql: sqlOf("namespace.epoch"), args: args.slice(0, 2) },
  ];
}

let outbox = 0;
function cas(scope: string, namespace: string, key: string, expected: number, value: string, writer: string) {
  outbox += 1;
  const all = [
    text(scope),
    text(REGISTRY),
    text(namespace),
    text(key),
    int(expected),
    text("live"),
    blob(value),
    text(writer),
    text("host-1"),
    text(`outbox-${outbox}`),
    text(`fingerprint-${outbox}`),
    text(`attempt-${outbox}`),
    int(Date.now()),
  ];
  return ["cas.receipt_insert", "cas.conflict_insert", "cas.version_insert", "cas.head_upsert", "cas.receipt_read"].map(
    (name) => ({ sql: sqlOf(name), args: argsFor(name, all) }),
  );
}

function headRead(scope: string, namespace: string, key: string) {
  return [{ sql: sqlOf("head.read"), args: [text(scope), text(REGISTRY), text(namespace), text(key)] }];
}

function post(token: string, body: unknown): Request {
  return bearerReq("/v1/kv/v2/pipeline", token, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
}

async function call(
  deps: Awaited<ReturnType<typeof depsWith>>,
  token: string,
  statements: { sql: string; args: KvValue[] }[],
) {
  const res = await dispatch(post(token, kvPipelineBody(statements)), deps);
  return { status: res.status, body: (await res.json()) as Record<string, unknown> };
}

function stepRows(body: Record<string, unknown>, step: number): KvValue[][] {
  const results = body.results as { response: { result: { step_results: ({ rows: KvValue[][] } | null)[] } } }[];
  return results[0].response.result.step_results[step]?.rows ?? [];
}

async function seeded() {
  const primary = new SqlitePrimary();
  const deps = await depsWith(primary);
  const defined = await call(deps, adminA, [
    ...defineNamespace("fleet", "fleet-shared"),
    ...defineNamespace("notes", null),
  ]);
  assert.equal(defined.status, 200, JSON.stringify(defined.body));
  return { primary, deps };
}

// --- the tests --------------------------------------------------------

test("a data-plane credential at the kv door is refused by its typed rule", async () => {
  const deps = await depsWith(new SqlitePrimary());
  const dataPlane = await h.sign({ a: "rw", p: { rw: { ns: ["fleet-kv"] } }, iat: Math.floor(Date.now() / 1000) });
  const res = await call(deps, dataPlane, headRead("fleet-shared", "fleet", "k"));
  assert.equal(res.status, 401);
  assert.equal(res.body.error, "data_plane_credential_refused");
});

test("the kv door needs an authenticated caller and a kv-scoped service token", async () => {
  const deps = await depsWith(new SqlitePrimary());
  const anonymous = await dispatch(
    new Request("https://fleet-hosted.test/v1/kv/v2/pipeline", { method: "POST", body: "{}" }),
    deps,
  );
  assert.equal(anonymous.status, 401);
  // The harness's service token is scoped to stamping and query only.
  const res = await call(h.deps as never, SERVICE_TOKEN, headRead("s", "fleet", "k"));
  assert.equal(res.status, 403);
  assert.equal(res.body.error, "service_token_route_scope");
});

test("with no primary configured the kv door fails closed", async () => {
  const deps = await depsWith();
  const res = await call(deps, userA, headRead("fleet-shared", "fleet", "k"));
  assert.equal(res.status, 503);
  assert.equal(res.body.error, "kv_primary_not_configured");
});

test("anything but the client's transaction shape is refused before the primary sees it", async () => {
  const primary = new SqlitePrimary();
  const deps = await depsWith(primary);
  const execute = await dispatch(
    post(userA, { requests: [{ type: "execute", stmt: { sql: sqlOf("head.read"), args: [] } }, { type: "close" }] }),
    deps,
  );
  assert.equal(execute.status, 400);
  assert.equal(((await execute.json()) as { error: string }).error, "kv_pipeline_malformed");

  const body = kvPipelineBody(headRead("fleet-shared", "fleet", "k")) as {
    requests: { batch: { steps: Record<string, unknown>[] } }[];
  };
  delete body.requests[0].batch.steps[1].condition;
  const unconditioned = await dispatch(post(userA, body), deps);
  assert.equal(unconditioned.status, 400);
  assert.equal(primary.bodies.length, 0, "nothing reached the primary");
});

test("the shape check ignores JSON key order, as a sorting client sends it", async () => {
  const { deps } = await seeded();
  const sortKeys = (value: unknown): unknown =>
    Array.isArray(value)
      ? value.map(sortKeys)
      : value && typeof value === "object"
        ? Object.fromEntries(
            Object.keys(value as object)
              .sort()
              .map((k) => [k, sortKeys((value as Record<string, unknown>)[k])]),
          )
        : value;
  const res = await dispatch(post(userA, sortKeys(kvPipelineBody(headRead("fleet-shared", "fleet", "k")))), deps);
  assert.equal(res.status, 200, await res.clone().text());
});

test("SQL outside the catalogue is refused", async () => {
  const primary = new SqlitePrimary();
  const deps = await depsWith(primary);
  const res = await call(deps, userA, [{ sql: "DROP TABLE kv_heads", args: [] }]);
  assert.equal(res.status, 403);
  assert.equal(res.body.error, "kv_statement_not_allowed");
  const extra = await call(deps, userA, [{ sql: sqlOf("head.read") + " ", args: [] }]);
  assert.equal(extra.body.error, "kv_statement_not_allowed", "texts match exactly, not loosely");
  assert.equal(primary.bodies.length, 0);
});

test("namespace definitions need the kv-admin role or the kv-admin door", async () => {
  const primary = new SqlitePrimary();
  const deps = await depsWith(primary);
  const plain = await call(deps, userA, defineNamespace("fleet", "fleet-shared"));
  assert.equal(plain.status, 403);
  assert.equal(plain.body.error, "kv_admin_required");
  const hostToken = await call(deps, KV_SERVICE_TOKEN, defineNamespace("fleet", "fleet-shared"));
  assert.equal(hostToken.body.error, "kv_admin_required");
  const admin = await call(deps, KV_ADMIN_SERVICE_TOKEN, defineNamespace("fleet", "fleet-shared"));
  assert.equal(admin.status, 200, JSON.stringify(admin.body));
  assert.deepEqual(stepRows(admin.body, 2), [[{ type: "integer", value: "1" }]]);
});

test("the door migrates the primary once, from the catalogue", async () => {
  const { primary, deps } = await seeded();
  const versions = primary.db.prepare("SELECT version FROM kv_central_migrations ORDER BY version").all() as {
    version: number;
  }[];
  assert.deepEqual(
    versions.map((r) => Number(r.version)),
    KV_CATALOG.migrations.versions.map((v) => v.version),
  );
  await call(deps, userA, headRead("fleet-shared", "fleet", "k"));
  await call(deps, userA, headRead("fleet-shared", "fleet", "k"));
  assert.equal(primary.migrationBatches(), KV_CATALOG.migrations.versions.length);
});

test("a caller writes and reads a shared namespace as itself", async () => {
  const { deps } = await seeded();
  const write = await call(deps, userA, cas("fleet-shared", "fleet", "build", 0, "green", BINDING_A));
  assert.equal(write.status, 200, JSON.stringify(write.body));
  const receipt = stepRows(write.body, 5)[0];
  assert.deepEqual(receipt[2], { type: "text", value: "committed" });
  const read = await call(deps, userA, headRead("fleet-shared", "fleet", "build"));
  const head = stepRows(read.body, 1)[0];
  assert.deepEqual(head[0], { type: "integer", value: "1" });
  assert.deepEqual(head[2], blob("green"));
});

test("a caller cannot write as someone else", async () => {
  const { primary, deps } = await seeded();
  const before = primary.bodies.length;
  const res = await call(deps, userA, cas("fleet-shared", "fleet", "build", 0, "forged", BINDING_B));
  assert.equal(res.status, 403);
  assert.equal(res.body.error, "kv_identity_refused");
  assert.equal(primary.bodies.length, before, "refused before the primary");
  // A host's kv service token acts for its hosts' users.
  const host = await call(deps, KV_SERVICE_TOKEN, cas("fleet-shared", "fleet", "build", 0, "swept", BINDING_B));
  assert.equal(host.status, 200, JSON.stringify(host.body));
});

test("a scope must be the namespace's shared scope or the caller's own private scope", async () => {
  const { deps } = await seeded();
  const wrongShared = await call(deps, userA, cas("not-fleet-shared", "fleet", "k", 0, "v", BINDING_A));
  assert.equal(wrongShared.status, 403);
  assert.equal(wrongShared.body.error, "kv_scope_refused");

  const mine = `${privateScopePrefix(BINDING_A)}0123abcd`;
  const theirs = `${privateScopePrefix(BINDING_B)}0123abcd`;
  const own = await call(deps, userA, cas(mine, "notes", "todo", 0, "milk", BINDING_A));
  assert.equal(own.status, 200, JSON.stringify(own.body));
  const other = await call(deps, userA, headRead(theirs, "notes", "todo"));
  assert.equal(other.status, 403);
  assert.equal(other.body.error, "kv_scope_refused");
  const privateOnShared = await call(deps, userA, cas(mine, "fleet", "k", 0, "v", BINDING_A));
  assert.equal(privateOnShared.body.error, "kv_scope_refused");
  const undefinedNamespace = await call(deps, userA, headRead(mine, "nowhere", "k"));
  assert.equal(undefinedNamespace.body.error, "kv_scope_refused");
});

test("an archive belongs to its owner", async () => {
  const { deps } = await seeded();
  const read = (owner: string) => [
    { sql: sqlOf("archive.checkpoint_read"), args: [text(owner), text("laptop"), text("ses_1")] },
  ];
  const own = await call(deps, userA, read(BINDING_A));
  assert.equal(own.status, 200, JSON.stringify(own.body));
  const other = await call(deps, userA, read(BINDING_B));
  assert.equal(other.status, 403);
  assert.equal(other.body.error, "kv_identity_refused");
});

test("the private scope prefix matches the client crate's", () => {
  // central::private_scope_prefix("drew") in pkgs/fleet/crates/fleet-kv.
  assert.equal(privateScopePrefix("drew"), "private:64726577:");
});

test("health reports whether the kv primary is configured, never its URL", async () => {
  const unset = await dispatch(new Request("https://fleet-hosted.test/health"), h.deps);
  assert.deepEqual(((await unset.json()) as { kv: unknown }).kv, { primary_configured: false });
  const config = configFromEnv({ KV_LIBSQL_URL: "libsql://kv.example.test", KV_LIBSQL_AUTH_TOKEN: "x" });
  const set = await dispatch(new Request("https://fleet-hosted.test/health"), { ...h.deps, config });
  const body = await set.text();
  assert.match(body, /"primary_configured":true/);
  assert.doesNotMatch(body, /kv\.example\.test/);
});
