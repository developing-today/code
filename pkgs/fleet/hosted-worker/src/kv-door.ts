// The fleet-kv door (#13316; design doc/2026-11-23T12-45-00Z_design_fleet_kv
// §4, §9, D011, D014). The central plane's only way in: a host's
// `fleet kv` client posts the same Hrana v2 pipeline body it would send
// a libSQL server to
//
//   POST /v1/kv/v2/pipeline     (so FLEET_KV_URL = https://<door>/v1/kv)
//
// and the door forwards it to the shared Turso primary under the
// Worker's own credential, after checking it statement by statement.
// The caller authenticates with an RP/agent JWT or a service token
// scoped to the `kv` door (src/auth.ts); a per-user Turso data-plane
// token is refused before this module runs (data_plane_credential_refused).
//
// The door never forwards SQL it did not get from the client crate:
// src/kv-catalog.json is generated from pkgs/fleet/crates/fleet-kv/src/
// catalog.rs (a Rust test keeps the copy equal), and every statement's
// text must match a catalogue entry exactly. The rest of the checks
// come from the entry's argument roles:
//
//   shape    exactly one batch then close: BEGIN IMMEDIATE, each
//            statement on the success of the one before, COMMIT, and
//            ROLLBACK unless the commit ran — the client's all-or-
//            nothing transaction (kv_pipeline_malformed otherwise)
//   class    admin statements (namespace definitions, grants) need the
//            kv-admin role, or a service token scoped to the kv-admin
//            door (kv_admin_required)
//   owner /  a person or agent caller may only write and read as its
//   writer   own tenancy binding id (kv_identity_refused); a service
//            token is a host daemon acting for its hosts' users and is
//            trusted for these, inside its route scope
//   scope    the scope must be the namespace's shared scope, or one of
//            the caller's own private scopes (`private:<hex binding>:`)
//            — checked against kv_namespaces on the primary before the
//            batch runs (kv_scope_refused)
//
// The door brings the primary's schema up to date itself, from the
// catalogue's migrations, once per isolate; clients never send DDL.
// Unconfigured (no KV_LIBSQL_URL) → 503, like every missing dependency.

import type { AuthDeps, CallerContext } from "./auth.ts";
import { FailClosed } from "./errors.ts";
import catalogDocument from "./kv-catalog.json" with { type: "json" };

export type KvClass = "read" | "write" | "admin";

export interface KvCatalogStatement {
  name: string;
  class: KvClass;
  args: number;
  scope: number | null;
  registry: number | null;
  namespace: number | null;
  owner: number | null;
  writer: number | null;
  sql: string;
}

export interface KvCatalog {
  format: string;
  admin_role: string;
  admin_door: string;
  door_namespace_scope: string;
  statements: KvCatalogStatement[];
  migrations: {
    table: string;
    read: string;
    record: string;
    versions: { version: number; statements: string[] }[];
  };
}

export const KV_CATALOG = catalogDocument as KvCatalog;

/** A Hrana value as it travels in a pipeline body. */
export type KvValue =
  | { type: "null" }
  | { type: "integer"; value: string }
  | { type: "float"; value: number }
  | { type: "text"; value: string }
  | { type: "blob"; base64: string };

export interface KvStatement {
  sql: string;
  args: KvValue[];
}

/** The shared primary the door forwards to: one Hrana pipeline body
 * in, the primary's HTTP status and parsed JSON out. */
export interface KvPrimary {
  pipeline(body: unknown): Promise<{ status: number; body: unknown }>;
}

/** Hrana over HTTP to the Turso primary: `libsql://` maps to https,
 * the scheme Turso issues; http(s) passes through (a local sqld). */
export class HttpKvPrimary implements KvPrimary {
  private readonly baseUrl: string;
  constructor(
    url: string,
    private readonly authToken: string | null,
  ) {
    this.baseUrl = url.startsWith("libsql://") ? `https://${url.slice("libsql://".length)}` : url;
  }

  async pipeline(body: unknown): Promise<{ status: number; body: unknown }> {
    const res = await fetch(`${this.baseUrl.replace(/\/+$/, "")}/v2/pipeline`, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        ...(this.authToken ? { authorization: `Bearer ${this.authToken}` } : {}),
      },
      body: JSON.stringify(body),
    });
    let parsed: unknown = null;
    try {
      parsed = await res.json();
    } catch {
      parsed = null;
    }
    return { status: res.status, body: parsed };
  }
}

const BY_SQL = new Map(KV_CATALOG.statements.map((s) => [s.sql, s]));

function malformed(message: string): never {
  throw new FailClosed(400, "kv_pipeline_malformed", message);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Structural equality, key order ignored: the client's JSON library
 * may sort object keys or keep insertion order. */
function sameJson(a: unknown, b: unknown): boolean {
  if (Array.isArray(a) || Array.isArray(b)) {
    return Array.isArray(a) && Array.isArray(b) && a.length === b.length && a.every((v, i) => sameJson(v, b[i]));
  }
  if (isRecord(a) && isRecord(b)) {
    const keys = Object.keys(a);
    return keys.length === Object.keys(b).length && keys.every((k) => k in b && sameJson(a[k], b[k]));
  }
  return a === b;
}

function checkValue(value: unknown): KvValue {
  if (!isRecord(value)) malformed("an argument is not a Hrana value");
  switch (value.type) {
    case "null":
      return { type: "null" };
    case "integer":
      if (typeof value.value !== "string" || !/^-?\d+$/.test(value.value))
        malformed("an integer argument is malformed");
      return { type: "integer", value: value.value };
    case "float":
      if (typeof value.value !== "number") malformed("a float argument is malformed");
      return { type: "float", value: value.value };
    case "text":
      if (typeof value.value !== "string") malformed("a text argument is malformed");
      return { type: "text", value: value.value };
    case "blob":
      if (typeof value.base64 !== "string") malformed("a blob argument is malformed");
      return { type: "blob", base64: value.base64 };
    default:
      return malformed("an argument has an unknown Hrana type");
  }
}

/** The statements of one client transaction, read out of its pipeline
 * body. Anything but the client's exact shape is refused. */
export function parseKvPipeline(body: unknown): KvStatement[] {
  if (!isRecord(body) || !Array.isArray(body.requests) || body.requests.length !== 2) {
    malformed("expected one batch request and one close");
  }
  const [batch, close] = body.requests as unknown[];
  if (!isRecord(close) || close.type !== "close" || Object.keys(close).length !== 1) {
    malformed("the second request must be close");
  }
  if (!isRecord(batch) || batch.type !== "batch" || !isRecord(batch.batch) || !Array.isArray(batch.batch.steps)) {
    malformed("the first request must be a batch");
  }
  const steps = batch.batch.steps as unknown[];
  const count = steps.length - 3;
  if (count < 1) malformed("a batch carries at least one statement");
  const stmtOf = (step: unknown, index: number): Record<string, unknown> => {
    if (!isRecord(step) || !isRecord(step.stmt)) malformed(`step ${index} has no statement`);
    return step.stmt;
  };
  const begin = steps[0];
  if (!isRecord(begin) || "condition" in begin || !sameJson(stmtOf(begin, 0), { sql: "BEGIN IMMEDIATE" })) {
    malformed("the batch must open with BEGIN IMMEDIATE");
  }
  const statements: KvStatement[] = [];
  for (let index = 1; index <= count; index++) {
    const step = steps[index] as Record<string, unknown>;
    if (!sameJson(step?.condition, { type: "ok", step: index - 1 })) {
      malformed(`step ${index} must run only after step ${index - 1} succeeded`);
    }
    const stmt = stmtOf(step, index);
    if (typeof stmt.sql !== "string" || !Array.isArray(stmt.args)) malformed(`step ${index} is malformed`);
    for (const key of Object.keys(stmt)) {
      if (key !== "sql" && key !== "args") malformed(`step ${index} carries ${key}`);
    }
    statements.push({ sql: stmt.sql, args: stmt.args.map(checkValue) });
  }
  const commit = steps[count + 1] as Record<string, unknown>;
  if (
    !sameJson(commit?.condition, { type: "ok", step: count }) ||
    !sameJson(stmtOf(commit, count + 1), { sql: "COMMIT" })
  ) {
    malformed("the batch must commit only after its last statement");
  }
  const rollback = steps[count + 2] as Record<string, unknown>;
  if (
    !sameJson(rollback?.condition, { type: "not", cond: { type: "ok", step: count + 1 } }) ||
    !sameJson(stmtOf(rollback, count + 2), { sql: "ROLLBACK" })
  ) {
    malformed("the batch must roll back unless it committed");
  }
  return statements;
}

/** The body the door forwards: rebuilt from the checked statements,
 * so nothing the checks did not read reaches the primary. */
export function kvPipelineBody(statements: KvStatement[]): unknown {
  const steps: unknown[] = [{ stmt: { sql: "BEGIN IMMEDIATE" } }];
  statements.forEach((s, i) => steps.push({ condition: { type: "ok", step: i }, stmt: { sql: s.sql, args: s.args } }));
  const n = statements.length;
  steps.push({ condition: { type: "ok", step: n }, stmt: { sql: "COMMIT" } });
  steps.push({ condition: { type: "not", cond: { type: "ok", step: n + 1 } }, stmt: { sql: "ROLLBACK" } });
  return { requests: [{ type: "batch", batch: { steps } }, { type: "close" }] };
}

/** The prefix of every private scope of `bindingId`
 * (central::private_scope_prefix in the client crate). */
export function privateScopePrefix(bindingId: string): string {
  const hex = Array.from(new TextEncoder().encode(bindingId), (b) => b.toString(16).padStart(2, "0")).join("");
  return `private:${hex}:`;
}

function isAdmin(ctx: CallerContext): boolean {
  if (ctx.kind === "service") return (ctx.serviceRoutes ?? []).includes(KV_CATALOG.admin_door);
  return ctx.roles.includes(KV_CATALOG.admin_role);
}

function textArg(stmt: KvStatement, entry: KvCatalogStatement, index: number, role: string): string {
  const value = stmt.args[index - 1];
  if (value?.type !== "text") {
    throw new FailClosed(400, "kv_pipeline_malformed", `${entry.name}: the ${role} argument must be text`);
  }
  return value.value;
}

/** A scoped address one statement touches: checked against the
 * namespace's definition before anything runs. */
interface ScopedAddress {
  scope: string;
  registry: string;
  namespace: string;
}

/** Checks every statement against the catalogue and the caller, and
 * returns the scoped addresses the namespace check must confirm. */
export function authorizeKvStatements(statements: KvStatement[], ctx: CallerContext): ScopedAddress[] {
  const addresses = new Map<string, ScopedAddress>();
  const trusted = ctx.kind === "service";
  for (const stmt of statements) {
    const entry = BY_SQL.get(stmt.sql);
    if (!entry) {
      throw new FailClosed(403, "kv_statement_not_allowed", "a statement is not in the fleet-kv catalogue");
    }
    if (stmt.args.length !== entry.args) {
      malformed(`${entry.name} takes ${entry.args} arguments, got ${stmt.args.length}`);
    }
    if (entry.class === "admin" && !isAdmin(ctx)) {
      throw new FailClosed(403, "kv_admin_required", `${entry.name} needs the ${KV_CATALOG.admin_role} role`);
    }
    for (const role of ["owner", "writer"] as const) {
      const index = entry[role];
      if (index === null) continue;
      const value = textArg(stmt, entry, index, role);
      if (!trusted && value !== ctx.bindingId) {
        throw new FailClosed(403, "kv_identity_refused", `${entry.name}: the ${role} is not the caller`);
      }
    }
    if (entry.scope !== null && entry.registry !== null && entry.namespace !== null) {
      const scope = textArg(stmt, entry, entry.scope, "scope");
      const address = {
        scope,
        registry: textArg(stmt, entry, entry.registry, "registry"),
        namespace: textArg(stmt, entry, entry.namespace, "namespace"),
      };
      if (scope.startsWith("private:") && !trusted && !scope.startsWith(privateScopePrefix(ctx.bindingId))) {
        throw new FailClosed(403, "kv_scope_refused", `${entry.name}: the private scope is not the caller's`);
      }
      addresses.set(JSON.stringify([address.scope, address.registry, address.namespace]), address);
    }
  }
  return [...addresses.values()];
}

function okResults(body: unknown): unknown[] {
  if (!isRecord(body) || !Array.isArray(body.results)) {
    throw new FailClosed(502, "kv_primary_failed", "the primary answered without results");
  }
  return body.results;
}

function rowsOf(result: unknown): unknown[][] {
  if (!isRecord(result) || result.type !== "ok" || !isRecord(result.response) || !isRecord(result.response.result)) {
    const message = isRecord(result) && isRecord(result.error) ? String(result.error.message ?? "") : "";
    throw new FailClosed(502, "kv_primary_failed", `the primary refused a door statement ${message}`.trim());
  }
  const rows = result.response.result.rows;
  return Array.isArray(rows) ? (rows as unknown[][]) : [];
}

function cellText(cell: unknown): string | null {
  return isRecord(cell) && cell.type === "text" && typeof cell.value === "string" ? cell.value : null;
}

/** Confirms each address's scope against its namespace's definition:
 * the namespace's shared scope, or a private scope of a private
 * namespace. One pipeline, read-only. */
export async function checkKvScopes(primary: KvPrimary, addresses: ScopedAddress[]): Promise<void> {
  if (addresses.length === 0) return;
  const requests: unknown[] = addresses.map((a) => ({
    type: "execute",
    stmt: {
      sql: KV_CATALOG.door_namespace_scope,
      args: [
        { type: "text", value: a.registry },
        { type: "text", value: a.namespace },
      ],
    },
  }));
  requests.push({ type: "close" });
  const res = await primary.pipeline({ requests });
  if (res.status < 200 || res.status >= 300) {
    throw new FailClosed(502, "kv_primary_failed", `the primary answered HTTP ${res.status}`);
  }
  const results = okResults(res.body);
  addresses.forEach((address, index) => {
    const row = rowsOf(results[index])[0];
    if (!row) {
      throw new FailClosed(403, "kv_scope_refused", `namespace ${address.namespace} is not defined in its registry`);
    }
    const kind = cellText(row[0]);
    const shared = cellText(row[1]);
    const ok =
      kind === "shared" ? address.scope === shared : kind === "private" && address.scope.startsWith("private:");
    if (!ok) {
      throw new FailClosed(403, "kv_scope_refused", `the scope is not namespace ${address.namespace}'s`);
    }
  });
}

/** One transaction of literal statements, the door's own. */
function transactionBody(statements: { sql: string; args?: KvValue[] }[]): unknown {
  return kvPipelineBody(statements.map((s) => ({ sql: s.sql, args: s.args ?? [] })));
}

async function runDoorTransaction(
  primary: KvPrimary,
  statements: { sql: string; args?: KvValue[] }[],
): Promise<unknown[][][]> {
  const res = await primary.pipeline(transactionBody(statements));
  if (res.status < 200 || res.status >= 300) {
    throw new FailClosed(502, "kv_primary_failed", `the primary answered HTTP ${res.status}`);
  }
  const first = okResults(res.body)[0];
  if (!isRecord(first) || first.type !== "ok" || !isRecord(first.response) || !isRecord(first.response.result)) {
    throw new FailClosed(502, "kv_primary_failed", "the primary refused the door's migration batch");
  }
  const stepResults = first.response.result.step_results;
  const stepErrors = first.response.result.step_errors;
  if (Array.isArray(stepErrors) && stepErrors.some((e) => e !== null)) {
    throw new FailClosed(502, "kv_primary_failed", "a migration statement failed on the primary");
  }
  if (!Array.isArray(stepResults)) return [];
  return stepResults
    .slice(1, statements.length + 1)
    .map((r) => (isRecord(r) && Array.isArray(r.rows) ? (r.rows as unknown[][]) : []));
}

/** Applies every catalogued migration the primary has not recorded,
 * each in its own transaction with its bookkeeping row — the same
 * steps as central::migrate in the client crate. */
export async function migrateKvPrimary(primary: KvPrimary, now: () => number = Date.now): Promise<number[]> {
  const { migrations } = KV_CATALOG;
  const [, appliedRows] = await runDoorTransaction(primary, [{ sql: migrations.table }, { sql: migrations.read }]);
  const applied = new Set(
    (appliedRows ?? [])
      .map((row) => (isRecord(row[0]) && row[0].type === "integer" ? Number(row[0].value) : null))
      .filter((v): v is number => v !== null),
  );
  const ran: number[] = [];
  for (const { version, statements } of migrations.versions) {
    if (applied.has(version)) continue;
    await runDoorTransaction(primary, [
      ...statements.map((sql) => ({ sql })),
      {
        sql: migrations.record,
        args: [
          { type: "integer", value: String(version) },
          { type: "integer", value: String(Math.trunc(now())) },
        ],
      },
    ]);
    ran.push(version);
  }
  return ran;
}

const migrated = new WeakMap<KvPrimary, Promise<number[]>>();

/** Migrates `primary` once per isolate; a failed attempt is forgotten
 * so the next request tries again. */
export function ensureKvMigrated(primary: KvPrimary): Promise<number[]> {
  let pending = migrated.get(primary);
  if (!pending) {
    pending = migrateKvPrimary(primary);
    migrated.set(primary, pending);
    pending.catch(() => migrated.delete(primary));
  }
  return pending;
}

/** The kv door's handler: check, migrate, confirm scopes, forward. */
export async function handleKvPipeline(req: Request, ctx: CallerContext, deps: AuthDeps): Promise<Response> {
  const primary = deps.kvPrimary;
  if (!primary) {
    throw new FailClosed(503, "kv_primary_not_configured", "this deployment has no fleet-kv primary configured");
  }
  let body: unknown;
  try {
    body = await req.json();
  } catch {
    malformed("the body is not JSON");
  }
  const statements = parseKvPipeline(body);
  const addresses = authorizeKvStatements(statements, ctx);
  await ensureKvMigrated(primary);
  await checkKvScopes(primary, addresses);
  const res = await primary.pipeline(kvPipelineBody(statements));
  if (res.status < 200 || res.status >= 300 || res.body === null) {
    throw new FailClosed(502, "kv_primary_failed", `the primary answered HTTP ${res.status}`);
  }
  return new Response(JSON.stringify(res.body), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}
