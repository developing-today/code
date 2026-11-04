// Per-user Durable Object for the hosted data plane (#5199).
//
// The object is named by binding_id (see routing.ts). Its SQLite schema
// is versioned and bootstrapped idempotently before serving; the first
// routed request binds the object immutably to (binding_id,
// jurisdiction) and every later request must match that identity.
//
// Guard (design SS "every identity-touching entry point runs a common
// guard"): before any read or write, fetch() verifies
//   1. the internal routing context is present (binding id +
//      jurisdiction, set by the routing module or, in tests, the
//      test harness — never trusted from a client, the front door
//      strips and re-sets these headers),
//   2. the runtime-reported object jurisdiction, when the platform
//      reports one (state.id.jurisdiction), equals the routed
//      jurisdiction — a mismatch fails closed; when the platform
//      reports none (local workerd reports none and does not implement
//      restrictions at all) the persisted identity in (3) is the
//      fail-closed anchor, and production residency still requires the
//      real-account canary gated on #2285,
//   3. once bootstrapped, the persisted (binding_id, jurisdiction)
//      equals the routed identity — a mismatch fails closed.
// Any guard failure returns before side effects.

import { FailClosed, parseBindingId, parseJurisdiction } from "./routing.ts";

export const SCHEMA_VERSION = 1;

export const HDR_BINDING = "x-hosted-binding-id";
export const HDR_JURISDICTION = "x-hosted-jurisdiction";
export const HDR_ROUTED = "x-hosted-routed";

interface SqlStorage {
  exec(query: string, ...params: unknown[]): { toArray(): Record<string, unknown>[] };
}
interface DOState {
  id: { name?: string; jurisdiction?: string };
  storage: { sql: SqlStorage };
  blockConcurrencyWhile<T>(fn: () => Promise<T>): Promise<T>;
}
interface Env {}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

export class UserDO {
  private state: DOState;
  private ready: Promise<void>;

  constructor(state: DOState, _env: Env) {
    this.state = state;
    // Versioned, idempotent schema bootstrap before serving.
    this.ready = state.blockConcurrencyWhile(async () => {
      const sql = this.state.storage.sql;
      sql.exec("CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)");
      sql.exec("CREATE TABLE IF NOT EXISTS kv (key TEXT PRIMARY KEY, value TEXT NOT NULL)");
      sql.exec("INSERT OR IGNORE INTO meta (key, value) VALUES (?, ?)", "schema_version", String(SCHEMA_VERSION));
    });
  }

  private meta(key: string): string | null {
    const rows = this.state.storage.sql.exec("SELECT value FROM meta WHERE key = ?", key).toArray();
    return rows.length === 0 ? null : String(rows[0].value);
  }

  private guard(req: Request): { bindingId: string; jurisdiction: string } {
    if (req.headers.get(HDR_ROUTED) !== "restricted") {
      throw new FailClosed(403, "unrouted_request", "request did not arrive via the jurisdiction routing module");
    }
    const bindingId = parseBindingId(req.headers.get(HDR_BINDING));
    const jurisdiction = parseJurisdiction(req.headers.get(HDR_JURISDICTION));
    const runtimeJurisdiction = this.state.id.jurisdiction;
    if (typeof runtimeJurisdiction === "string" && runtimeJurisdiction !== jurisdiction) {
      throw new FailClosed(
        409,
        "jurisdiction_mismatch",
        "object jurisdiction does not match routed jurisdiction; failing closed",
      );
    }
    const boundBinding = this.meta("binding_id");
    const boundJurisdiction = this.meta("jurisdiction");
    if (boundBinding === null) {
      // First routed request: bind identity immutably. Concurrent first
      // requests are serialized by the DO input gate, and both meta
      // rows are written together before any data route runs.
      this.state.storage.sql.exec(
        "INSERT INTO meta (key, value) VALUES (?, ?), (?, ?)",
        "binding_id",
        bindingId,
        "jurisdiction",
        jurisdiction,
      );
    } else if (boundBinding !== bindingId || boundJurisdiction !== jurisdiction) {
      throw new FailClosed(
        409,
        "identity_mismatch",
        "object is bound to a different binding/jurisdiction; failing closed",
      );
    }
    return { bindingId, jurisdiction };
  }

  async fetch(req: Request): Promise<Response> {
    await this.ready;
    try {
      const { bindingId, jurisdiction } = this.guard(req);
      const url = new URL(req.url);
      if (url.pathname === "/v1/state" && req.method === "GET") {
        return json({
          binding_id: bindingId,
          jurisdiction,
          schema_version: Number(this.meta("schema_version")),
          object_name: this.state.id.name ?? null,
        });
      }
      const kvMatch = url.pathname.match(/^\/v1\/kv\/([A-Za-z0-9._-]{1,128})$/);
      if (kvMatch && req.method === "PUT") {
        const value = await req.text();
        this.state.storage.sql.exec(
          "INSERT INTO kv (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
          kvMatch[1],
          value,
        );
        return json({ key: kvMatch[1], stored: true });
      }
      if (kvMatch && req.method === "GET") {
        const rows = this.state.storage.sql.exec("SELECT value FROM kv WHERE key = ?", kvMatch[1]).toArray();
        if (rows.length === 0) return json({ error: "not_found" }, 404);
        return json({ key: kvMatch[1], value: String(rows[0].value) });
      }
      return json({ error: "not_found" }, 404);
    } catch (e) {
      if (e instanceof FailClosed) return json({ error: e.code, message: e.message }, e.status);
      throw e;
    }
  }
}
