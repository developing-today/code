// TenantOpsDO — the provisioning state machine for one tenant
// (component E2 of #9083, issue #12797).
//
// Design record: doc/2026-11-17T16-55-00Z_design_hosted-tenancy-architecture
// section 3 (the DO inventory: one TenantOpsDO per tenant, keyed by
// binding_id, created lazily; "the provisioning state machine (§4):
// serialised transitions, idempotent external effects,
// retry/compensation") and section 4 (the provisioning lifecycle this
// module implements, step for step). The registry row it drives is
// the E1 baseline (tenancy/migrations/0001_tenancy_baseline.sql,
// #12796): `tenant_db`, one row per tenant, tenant_id = binding_id.
//
//   requested ──► db_creating ──► db_ready ──► active
//       │              │               │          │
//       │              └──► failed ◄───┘          ├──► suspended ──► active
//       └──► failed (validation)                  ├──► export_only ──► deleting
//                                                 └──► deleting ──► deleted
//
// (failed is also reachable from db_creating on a permanent provider
// failure; the diagram's failed edges are drawn from requested and
// db_ready's predecessor states — §4 step 4: creation is "retried
// with the same request id until the row is db_ready or failed".)
//
// The rules that make it defensible, and where they live below:
//
//   - The row is the record; the object is the lock. Every route runs
//     inside a per-object serialisation gate, so two concurrent
//     sign-ups for one binding cannot interleave a transition, and
//     the provider seam below is only ever called from inside the
//     gate.
//   - Creation is intent-first. The deterministic request id
//     (provisioningRequestId) is a pure function of the tenant id,
//     and the intent (request id, jurisdiction, tenant_id) is
//     persisted on the row (state db_creating) BEFORE the provider
//     call, so a retry after a crash adopts the existing database
//     instead of creating a second one.
//   - Only the actors §4 names may perform each step. The actor
//     arrives on internal headers set by the tenancy Worker after
//     authentication — the same trust posture as the front door's
//     routing context (never trusted from a client) — and every
//     transition checks the role (and, for owner, that the actor IS
//     this object's binding) before it touches the row.
//   - Deletion is two-phase (§4 step 10): credential revocation
//     first (at deleting), then — after the cooling-off window —
//     the drop, then the tombstone. A half-deleted tenant whose
//     token still works is the failure this ordering prevents; a
//     tenant stuck in deleting has no working credentials, which is
//     the safe half-state.
//   - Suspension must not look like loss: the state route answers
//     in every state, including suspended and export_only.
//
// Seams, stated plainly:
//   - The tenancy SQL handle is the object's SQLite storage with
//     the tenancy migrations applied (the scaffold's local model of
//     the central tenancy DB). The E6 store adapter (#12799's line)
//     is where a Turso-over-HTTP handle plugs in; the machine does
//     not care which side of the seam it is on.
//   - The Turso management API is the TenantDatabaseProvider
//     interface. The platform credential that a production provider
//     would hold never leaves the Worker and is referenced by name
//     only; with no provider configured, provisioning and database
//     drops fail closed rather than pretending.
//   - Credential ISSUANCE (§4 step 6) is E4, not this slice. This
//     slice revokes credential metadata on deletion because §4
//     step 10 names revocation as deletion's first phase; it mints
//     nothing.

import { FailClosed, parseBindingId, parseJurisdiction, type Jurisdiction } from "./routing.ts";
import { TenancyRegistry } from "./tenancy-registry.ts";

export const TENANT_STATES = [
  "requested",
  "db_creating",
  "db_ready",
  "active",
  "suspended",
  "export_only",
  "deleting",
  "deleted",
  "failed",
] as const;
export type TenantState = (typeof TENANT_STATES)[number];

export const PARTICIPANT_KINDS = ["user_hosted", "user_native", "self_hosted_node", "public"] as const;
export type ParticipantKind = (typeof PARTICIPANT_KINDS)[number];

/** Internal headers, set by the tenancy Worker after it has
 * authenticated the caller — the ops analogue of user-do.ts's
 * routing context. Never trusted from a client. */
export const HDR_ACTOR_ROLE = "x-tenancy-actor-role";
export const HDR_ACTOR_ID = "x-tenancy-actor-id";
export const HDR_TRIGGER = "x-tenancy-trigger";

/** §4's actors. owner = the bound binding (the account holder);
 * operator = the operator role; platform = TenantOpsDO itself,
 * holding the platform management credential; trigger = an
 * automated trigger §4 names (abuse / ceiling). */
export type ActorRole = "owner" | "operator" | "platform" | "trigger";
export interface OpsActor {
  role: ActorRole;
  /** owner: the binding_id. operator: the operator's id.
   * platform: "tenant-ops". trigger: the trigger name. */
  id: string;
}

export interface TenantDatabaseHandle {
  dbId: string;
  dbUrl: string;
}

/** A provider failure. `retryable` distinguishes a transient
 * provider fault (retry with the same request id) from a permanent
 * refusal (the tenant fails, with compensation). Errors that are
 * not ProviderErrors are treated as retryable: an unknown fault
 * must not fail a tenant permanently on a guess. */
export class ProviderError extends Error {
  readonly retryable: boolean;
  constructor(message: string, retryable: boolean) {
    super(message);
    this.name = "ProviderError";
    this.retryable = retryable;
  }
}

/** The platform's database provider (the Turso management API in
 * production). Every operation is idempotent, because the state
 * machine retries: createDatabase under a request id that already
 * has a database returns THAT database; dropDatabase of a missing
 * database is a no-op success. */
export interface TenantDatabaseProvider {
  createDatabase(intent: {
    requestId: string;
    tenantId: string;
    jurisdiction: Jurisdiction;
  }): Promise<TenantDatabaseHandle>;
  /** The database created under an intent, if one exists. This is
   * the adoption lookup: after a crash or a lost response, the
   * machine finds the database its intent already made. */
  findDatabase(requestId: string): Promise<TenantDatabaseHandle | null>;
  dropDatabase(dbId: string): Promise<void>;
}

/** The deterministic provisioning request id for a tenant. Pure in
 * the tenant id, so the intent needs no storage of its own: any
 * retry, in any process, recomputes the same id and the provider
 * call under it adopts rather than duplicates (§4: "retried with
 * the same request id"). */
export function provisioningRequestId(tenantId: string): string {
  return `tenant-db:${tenantId}`;
}

/** Bounded in-call create attempts. A retryable failure that
 * outlasts them leaves the row db_creating — never failed on a
 * transient fault — and the next provision call continues with the
 * same request id. */
export const MAX_CREATE_ATTEMPTS = 3;

/** Default cooling-off window before a deletion completes (§4
 * step 10 names the window, not its length; the deployment may
 * override it with TENANT_DELETION_COOLING_OFF_MS). */
export const DEFAULT_COOLING_OFF_MS = 24 * 60 * 60 * 1000;

interface SqlStorage {
  exec(query: string, ...params: unknown[]): { toArray(): Record<string, unknown>[] };
}
interface DOState {
  id: { name?: string; jurisdiction?: string };
  storage: { sql: SqlStorage };
  blockConcurrencyWhile<T>(fn: () => Promise<T>): Promise<T>;
}
interface Env {
  TENANT_DB_PROVIDER?: TenantDatabaseProvider;
  TENANT_DELETION_COOLING_OFF_MS?: string;
}

interface TenantRow {
  tenant_id: string;
  participant_kind: string;
  turso_db_id: string | null;
  turso_db_url: string | null;
  jurisdiction: string;
  backend: string;
  state: TenantState;
  schema_version: number;
  created_at: string;
  deleted_at: string | null;
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function rowView(row: TenantRow): Record<string, unknown> {
  return {
    tenant_id: row.tenant_id,
    state: row.state,
    participant_kind: row.participant_kind,
    jurisdiction: row.jurisdiction,
    backend: row.backend,
    turso_db_id: row.turso_db_id,
    turso_db_url: row.turso_db_url,
    schema_version: row.schema_version,
    created_at: row.created_at,
    deleted_at: row.deleted_at,
  };
}

export class TenantOpsDO {
  private state: DOState;
  private env: Env;
  private ready: Promise<void>;
  private gate: Promise<void> = Promise.resolve();
  /** The tenant this object serves: its name is the binding_id
   * (§3: keyed by binding_id). Null when the name is not a minted
   * binding id — every route then fails closed. */
  private tenantId: string | null;
  private schemaPresent = false;

  constructor(state: DOState, env: Env) {
    this.state = state;
    this.env = env;
    try {
      this.tenantId = parseBindingId(state.id.name ?? null);
    } catch {
      this.tenantId = null;
    }
    // The object's own journal. The tenancy tables are NOT created
    // here: they belong to the tenancy migrations, applied to this
    // storage by the migration runner (in tests, by the harness),
    // and a missing tenancy schema fails closed below rather than
    // being silently improvised.
    this.ready = state.blockConcurrencyWhile(async () => {
      const sql = this.state.storage.sql;
      sql.exec(
        "CREATE TABLE IF NOT EXISTS ops_meta (name TEXT PRIMARY KEY, value TEXT NOT NULL)",
      );
      sql.exec(
        "CREATE TABLE IF NOT EXISTS ops_journal (seq INTEGER PRIMARY KEY AUTOINCREMENT, at TEXT NOT NULL, event TEXT NOT NULL, from_state TEXT, to_state TEXT, actor_role TEXT NOT NULL, actor_id TEXT NOT NULL, detail TEXT)",
      );
      const found = sql
        .exec("SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'tenant_db'")
        .toArray();
      this.schemaPresent = found.length > 0;
    });
  }

  // -- plumbing ----------------------------------------------------

  private sql(): SqlStorage {
    return this.state.storage.sql;
  }

  /** The serialisation gate (§3/§4: "the object is the lock").
   * Every route runs inside it, so transitions for one tenant can
   * never interleave — including across the provider call, which is
   * exactly where a second sign-up would otherwise slip in. */
  private async serialized<T>(fn: () => Promise<T>): Promise<T> {
    let release!: () => void;
    const next = new Promise<void>((resolve) => {
      release = resolve;
    });
    const prev = this.gate;
    this.gate = next;
    await prev;
    try {
      return await fn();
    } finally {
      release();
    }
  }

  private meta(name: string): string | null {
    const rows = this.sql().exec("SELECT value FROM ops_meta WHERE name = ?", name).toArray();
    return rows.length > 0 ? String(rows[0].value) : null;
  }

  private setMeta(name: string, value: string): void {
    this.sql().exec(
      "INSERT INTO ops_meta (name, value) VALUES (?, ?) ON CONFLICT(name) DO UPDATE SET value = excluded.value",
      name,
      value,
    );
  }

  private journal(
    at: string,
    event: string,
    from: TenantState | null,
    to: TenantState | null,
    actor: OpsActor,
    detail?: Record<string, unknown>,
  ): void {
    this.sql().exec(
      "INSERT INTO ops_journal (at, event, from_state, to_state, actor_role, actor_id, detail) VALUES (?, ?, ?, ?, ?, ?, ?)",
      at,
      event,
      from,
      to,
      actor.role,
      actor.id,
      detail === undefined ? null : JSON.stringify(detail),
    );
  }

  private row(): TenantRow | null {
    const rows = this.sql()
      .exec("SELECT * FROM tenant_db WHERE tenant_id = ?", this.tenantId)
      .toArray();
    return rows.length > 0 ? (rows[0] as unknown as TenantRow) : null;
  }

  private setState(row: TenantRow, to: TenantState, extra: Record<string, unknown> = {}): TenantRow {
    const sets = ["state = ?"];
    const params: unknown[] = [to];
    for (const [column, value] of Object.entries(extra)) {
      sets.push(`${column} = ?`);
      params.push(value);
    }
    params.push(row.tenant_id);
    this.sql().exec(`UPDATE tenant_db SET ${sets.join(", ")} WHERE tenant_id = ?`, ...params);
    return { ...row, ...extra, state: to };
  }

  private actor(req: Request): OpsActor {
    const role = req.headers.get(HDR_ACTOR_ROLE);
    const id = req.headers.get(HDR_ACTOR_ID);
    if (role === null || id === null || id === "") {
      throw new FailClosed(401, "missing_actor", "no authenticated actor on this request; failing closed");
    }
    if (role === "owner" || role === "operator" || role === "platform") {
      return { role, id };
    }
    if (role === "trigger") {
      const trigger = req.headers.get(HDR_TRIGGER);
      if (trigger !== "abuse" && trigger !== "ceiling") {
        throw new FailClosed(401, "invalid_actor", "a trigger actor must name a trigger §4 names (abuse | ceiling)");
      }
      return { role, id: trigger };
    }
    throw new FailClosed(401, "invalid_actor", `unknown actor role ${JSON.stringify(role)}; failing closed`);
  }

  /** The §4 role check. `owner` must be THIS tenant's binding —
   * an owner of some other binding is not an actor here at all. */
  private requireActor(actor: OpsActor, allowed: ActorRole[], step: string): void {
    if (!allowed.includes(actor.role)) {
      throw new FailClosed(
        403,
        "actor_not_permitted",
        `${step}: §4 does not name the ${actor.role} actor for this step; failing closed`,
      );
    }
    if (actor.role === "owner" && actor.id !== this.tenantId) {
      throw new FailClosed(
        403,
        "actor_not_permitted",
        `${step}: the owner actor is not this tenant's binding; failing closed`,
      );
    }
  }

  private requireRow(step: string): TenantRow {
    const row = this.row();
    if (row === null) {
      throw new FailClosed(404, "tenant_not_requested", `${step}: no tenant row exists for this binding`);
    }
    return row;
  }

  private requireState(row: TenantRow, states: TenantState[], step: string): void {
    if (!states.includes(row.state)) {
      throw new FailClosed(
        409,
        "invalid_transition",
        `${step}: tenant is ${row.state}; §4 allows this step only from ${states.join(" | ")}`,
      );
    }
  }

  private provider(step: string): TenantDatabaseProvider {
    const provider = this.env.TENANT_DB_PROVIDER;
    if (!provider) {
      throw new FailClosed(
        503,
        "provider_unavailable",
        `${step}: no database provider is configured; the platform credential never leaves the Worker and none is bound here — failing closed`,
      );
    }
    return provider;
  }

  private coolingOffMs(): number {
    const raw = this.env.TENANT_DELETION_COOLING_OFF_MS;
    if (raw === undefined || raw === "") return DEFAULT_COOLING_OFF_MS;
    const n = Number(raw);
    if (!Number.isInteger(n) || n < 0) {
      throw new FailClosed(
        500,
        "cooling_off_misconfigured",
        "TENANT_DELETION_COOLING_OFF_MS is not a non-negative integer; deletion fails closed",
      );
    }
    return n;
  }

  private async body(req: Request): Promise<Record<string, unknown>> {
    const text = await req.text();
    if (text === "") return {};
    try {
      const parsed: unknown = JSON.parse(text);
      if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
        throw new FailClosed(400, "invalid_body", "request body must be a JSON object");
      }
      return parsed as Record<string, unknown>;
    } catch (e) {
      if (e instanceof FailClosed) throw e;
      throw new FailClosed(400, "invalid_body", "request body is not valid JSON");
    }
  }

  /** Caller-supplied internal clock (the tenancy Worker's), so
   * time-bearing steps are deterministic under test; defaults to
   * the object's own clock. */
  private at(body: Record<string, unknown>): string {
    const raw = body.at;
    if (raw === undefined) return new Date().toISOString();
    if (typeof raw !== "string" || Number.isNaN(Date.parse(raw))) {
      throw new FailClosed(400, "invalid_body", "at must be an ISO-8601 timestamp string");
    }
    return raw;
  }

  // -- §4 steps ------------------------------------------------------

  /** Steps 1–3 (signup + binding ceremony are upstream; this is
   * the tenant request they enable): record the row `requested`
   * with its intent, then validate. A tenant whose binding is
   * missing or not active fails validation ON the row (§4's
   * requested ──► failed edge) — and no database is ever created
   * before a binding exists. Idempotent: a repeat request returns
   * the existing row, whatever state it has reached. */
  private async request(req: Request): Promise<Response> {
    const actor = this.actor(req);
    this.requireActor(actor, ["owner"], "tenant request");
    const body = await this.body(req);
    const at = this.at(body);
    const existing = this.row();
    if (existing !== null) return json(rowView(existing));

    const jurisdiction = parseJurisdiction(
      typeof body.jurisdiction === "string" ? body.jurisdiction : null,
    );
    const participantKind = body.participantKind ?? "user_hosted";
    if (!(PARTICIPANT_KINDS as readonly unknown[]).includes(participantKind)) {
      throw new FailClosed(400, "invalid_body", `participantKind must be one of ${PARTICIPANT_KINDS.join(" | ")}`);
    }
    const backend = body.backend ?? "turso";
    if (backend !== "turso" && backend !== "redb" && backend !== "k8s_tiered") {
      throw new FailClosed(400, "invalid_body", "backend must be one of turso | redb | k8s_tiered");
    }
    // Intent first: the row, carrying the deterministic request
    // id's inputs (tenant, jurisdiction), exists before any
    // provider call can happen.
    this.sql().exec(
      "INSERT INTO tenant_db (tenant_id, participant_kind, turso_db_id, turso_db_url, jurisdiction, backend, state, schema_version, created_at, deleted_at) VALUES (?, ?, NULL, NULL, ?, ?, 'requested', 0, ?, NULL)",
      this.tenantId,
      participantKind,
      jurisdiction,
      backend,
      at,
    );
    let row = this.requireRow("tenant request");
    this.journal(at, "requested", null, "requested", actor, {
      request_id: provisioningRequestId(row.tenant_id),
      jurisdiction,
    });
    const reason = new TenancyRegistry(this.sql()).provisioningBlockReason(this.tenantId);
    if (reason !== null) {
      row = this.setState(row, "failed");
      this.journal(at, "failed", "requested", "failed", actor, { reason });
    }
    return json(rowView(row));
  }

  /** Step 4: database creation. TenantOpsDO only, holding the
   * platform credential (via the provider seam). Intent-first and
   * idempotent: adopt before creating, retry retryable failures
   * with the same request id, and on a permanent failure check
   * once more for a database the intent already made before
   * failing the tenant — adoption is the compensation that leaves
   * no orphan and no duplicate. */
  private async provision(req: Request): Promise<Response> {
    const actor = this.actor(req);
    this.requireActor(actor, ["platform"], "database creation");
    const at = this.at(await this.body(req));
    let row = this.requireRow("database creation");
    if (row.state === "db_ready" || row.state === "active" || row.state === "suspended" || row.state === "export_only") {
      return json(rowView(row)); // already provisioned: idempotent no-op
    }
    this.requireState(row, ["requested", "db_creating"], "database creation");
    const provider = this.provider("database creation");
    if (row.state === "requested") {
      row = this.setState(row, "db_creating");
      this.journal(at, "db_creating", "requested", "db_creating", actor, {
        request_id: provisioningRequestId(row.tenant_id),
      });
    }
    const requestId = provisioningRequestId(row.tenant_id);
    const intent = { requestId, tenantId: row.tenant_id, jurisdiction: row.jurisdiction as Jurisdiction };

    const adopted = await provider.findDatabase(requestId);
    if (adopted !== null) {
      row = this.setState(row, "db_ready", { turso_db_id: adopted.dbId, turso_db_url: adopted.dbUrl });
      this.journal(at, "db_ready", "db_creating", "db_ready", actor, { adopted: true });
      return json(rowView(row));
    }
    for (let attempt = 1; attempt <= MAX_CREATE_ATTEMPTS; attempt++) {
      try {
        const handle = await provider.createDatabase(intent);
        row = this.setState(row, "db_ready", { turso_db_id: handle.dbId, turso_db_url: handle.dbUrl });
        this.journal(at, "db_ready", "db_creating", "db_ready", actor, { attempt });
        return json(rowView(row));
      } catch (e) {
        const retryable = !(e instanceof ProviderError) || e.retryable;
        // The create may have landed despite the error (a lost
        // response is the normal shape of this): look before
        // deciding anything.
        const landed = await provider.findDatabase(requestId);
        if (landed !== null) {
          row = this.setState(row, "db_ready", { turso_db_id: landed.dbId, turso_db_url: landed.dbUrl });
          this.journal(at, "db_ready", "db_creating", "db_ready", actor, { adopted: true, attempt });
          return json(rowView(row));
        }
        if (!retryable) {
          row = this.setState(row, "failed");
          this.journal(at, "failed", "db_creating", "failed", actor, {
            reason: "provider_refused",
            attempt,
          });
          return json(rowView(row));
        }
        this.journal(at, "create_attempt_failed", "db_creating", "db_creating", actor, {
          attempt,
          retryable: true,
        });
      }
    }
    // Retryable failures outlasted the in-call attempts: stay
    // db_creating. The next provision call resumes with the same
    // request id — the tenant is never failed on a transient fault.
    return json({ ...rowView(row), provisioning: "in_progress" }, 202);
  }

  /** Step 5's signal: the TenantDO has bootstrapped the tenant
   * schema (its own idempotent bootstrap, in user-do.ts). The ops
   * object only records the observation — activation (§4 step 7)
   * is refused until it exists. */
  private async bootstrapObserved(req: Request): Promise<Response> {
    const actor = this.actor(req);
    this.requireActor(actor, ["platform"], "schema bootstrap observation");
    const body = await this.body(req);
    const at = this.at(body);
    const row = this.requireRow("schema bootstrap observation");
    this.requireState(row, ["db_ready", "active"], "schema bootstrap observation");
    this.setMeta("schema_bootstrap_observed", "1");
    if (body.schemaVersion !== undefined) {
      if (!Number.isInteger(body.schemaVersion) || (body.schemaVersion as number) < 0) {
        throw new FailClosed(400, "invalid_body", "schemaVersion must be a non-negative integer");
      }
      this.setState(row, row.state, { schema_version: body.schemaVersion });
    }
    this.journal(at, "schema_bootstrap_observed", row.state, row.state, actor);
    return json(rowView(this.requireRow("schema bootstrap observation")));
  }

  /** Step 7: activation, once schema bootstrap has been observed. */
  private async activate(req: Request): Promise<Response> {
    const actor = this.actor(req);
    this.requireActor(actor, ["platform"], "activation");
    const at = this.at(await this.body(req));
    let row = this.requireRow("activation");
    if (row.state === "active") return json(rowView(row));
    this.requireState(row, ["db_ready"], "activation");
    if (this.meta("schema_bootstrap_observed") !== "1") {
      throw new FailClosed(
        409,
        "bootstrap_not_observed",
        "activation: §4 step 7 requires schema bootstrap to have been observed first",
      );
    }
    row = this.setState(row, "active");
    this.journal(at, "active", "db_ready", "active", actor);
    return json(rowView(row));
  }

  /** Step 8: suspension — operator role, or an abuse/ceiling
   * trigger. Reversible (resume below); data is untouched and
   * existing tokens expire on their own TTL (E4's surface). */
  private async suspend(req: Request): Promise<Response> {
    const actor = this.actor(req);
    this.requireActor(actor, ["operator", "trigger"], "suspension");
    const at = this.at(await this.body(req));
    let row = this.requireRow("suspension");
    if (row.state === "suspended") return json(rowView(row));
    this.requireState(row, ["active"], "suspension");
    row = this.setState(row, "suspended");
    this.journal(at, "suspended", "active", "suspended", actor);
    return json(rowView(row));
  }

  /** Suspension reversed: suspended ──► active (operator only —
   * the role that may suspend is the role that may lift it). */
  private async resume(req: Request): Promise<Response> {
    const actor = this.actor(req);
    this.requireActor(actor, ["operator"], "resume");
    const at = this.at(await this.body(req));
    let row = this.requireRow("resume");
    if (row.state === "active") return json(rowView(row));
    this.requireState(row, ["suspended"], "resume");
    row = this.setState(row, "active");
    this.journal(at, "active", "suspended", "active", actor);
    return json(rowView(row));
  }

  /** Step 9: export-only — a user request (owner), an operator
   * forcing it, or the ceiling trigger. Reads and export stay;
   * writes refuse (the rule #3811 states, enforced on the data
   * plane). Abuse may suspend, but §4 does not name it here. */
  private async exportOnly(req: Request): Promise<Response> {
    const actor = this.actor(req);
    this.requireActor(actor, ["owner", "operator", "trigger"], "export-only");
    if (actor.role === "trigger" && actor.id !== "ceiling") {
      throw new FailClosed(
        403,
        "actor_not_permitted",
        "export-only: §4 names the ceiling trigger here, not abuse; failing closed",
      );
    }
    const at = this.at(await this.body(req));
    let row = this.requireRow("export-only");
    if (row.state === "export_only") return json(rowView(row));
    this.requireState(row, ["active", "suspended"], "export-only");
    const from = row.state;
    row = this.setState(row, "export_only");
    this.journal(at, "export_only", from, "export_only", actor);
    return json(rowView(row));
  }

  /** Step 10: deletion, two-phase. Initiation (the account
   * holder, from export_only) revokes every credential FIRST and
   * opens the cooling-off window. Completion — the same route,
   * once the window has elapsed — drops the database and writes
   * the tombstone; the tenancy rows are retained. A failed drop
   * leaves the tenant in deleting with its credentials already
   * revoked: the safe half-state. */
  private async delete(req: Request): Promise<Response> {
    const actor = this.actor(req);
    const body = await this.body(req);
    const at = this.at(body);
    let row = this.requireRow("deletion");
    if (row.state === "deleted") return json(rowView(row));
    if (row.state !== "deleting") {
      this.requireActor(actor, ["owner"], "deletion");
      this.requireState(row, ["export_only"], "deletion");
      // Phase 1: revocation first, before anything irreversible.
      this.sql().exec(
        "UPDATE credential SET revoked_at = ? WHERE tenant_id = ? AND revoked_at IS NULL",
        at,
        row.tenant_id,
      );
      this.setMeta("deletion_requested_at", at);
      row = this.setState(row, "deleting");
      this.journal(at, "deleting", "export_only", "deleting", actor, { phase: "revoked" });
      return json(rowView(row));
    }
    // Completion: the holder who opened the window, or the
    // platform driving its own sweep — nobody else.
    this.requireActor(actor, ["owner", "platform"], "deletion completion");
    const requestedAt = this.meta("deletion_requested_at");
    const windowMs = this.coolingOffMs();
    // A missing or unreadable start time cannot prove the window
    // elapsed, so it refuses: NaN would otherwise compare false and
    // let the drop through at once.
    const elapsedMs = requestedAt === null ? Number.NaN : Date.parse(at) - Date.parse(requestedAt);
    if (!Number.isFinite(elapsedMs)) {
      throw new FailClosed(
        409,
        "cooling_off_unproven",
        "deletion: the recorded deletion request time is missing or unreadable, so the cooling-off window (§4 step 10) cannot be shown to have elapsed; the tenant stays in deleting with its credentials revoked",
      );
    }
    if (elapsedMs < windowMs) {
      throw new FailClosed(
        409,
        "cooling_off",
        "deletion: the cooling-off window (§4 step 10) has not elapsed; the tenant stays in deleting with its credentials revoked",
      );
    }
    // Phase 2: the drop. Only the platform credential may drop a
    // database, and it acts through the provider seam.
    if (row.turso_db_id !== null) {
      const provider = this.provider("deletion");
      await provider.dropDatabase(row.turso_db_id);
    }
    row = this.setState(row, "deleted", { deleted_at: at });
    this.journal(at, "deleted", "deleting", "deleted", actor, { phase: "dropped" });
    return json(rowView(row));
  }

  /** The row, readable in every state — a suspended or
   * export-only tenant's state must never look like loss (§4). */
  private async getState(req: Request): Promise<Response> {
    const actor = this.actor(req);
    this.requireActor(actor, ["owner", "operator", "platform"], "state read");
    return json(rowView(this.requireRow("state read")));
  }

  async fetch(req: Request): Promise<Response> {
    await this.ready;
    return this.serialized(async () => {
      try {
        if (this.tenantId === null) {
          throw new FailClosed(401, "invalid_binding", "this object's name is not a minted binding id; failing closed");
        }
        if (!this.schemaPresent) {
          throw new FailClosed(
            503,
            "tenancy_schema_missing",
            "the tenancy migrations have not been applied to this object's storage; failing closed",
          );
        }
        const path = new URL(req.url).pathname;
        const route = `${req.method} ${path}`;
        switch (route) {
          case "POST /v1/tenant/request":
            return await this.request(req);
          case "POST /v1/tenant/provision":
            return await this.provision(req);
          case "POST /v1/tenant/bootstrap-observed":
            return await this.bootstrapObserved(req);
          case "POST /v1/tenant/activate":
            return await this.activate(req);
          case "POST /v1/tenant/suspend":
            return await this.suspend(req);
          case "POST /v1/tenant/resume":
            return await this.resume(req);
          case "POST /v1/tenant/export-only":
            return await this.exportOnly(req);
          case "POST /v1/tenant/delete":
            return await this.delete(req);
          case "GET /v1/tenant/state":
            return await this.getState(req);
          default:
            return json({ error: "not_found" }, 404);
        }
      } catch (e) {
        if (e instanceof FailClosed) return json({ error: e.code, message: e.message }, e.status);
        throw e;
      }
    });
  }
}
