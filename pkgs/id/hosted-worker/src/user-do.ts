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

import { FailClosed, blobsBindingFor, parseBindingId, parseJurisdiction, type Jurisdiction, type R2BucketLike } from "./routing.ts";

export const SCHEMA_VERSION = 2;

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
interface Env {
  BLOBS?: R2BucketLike;
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

export class UserDO {
  private state: DOState;
  private ready: Promise<void>;

  private env: Env;

  constructor(state: DOState, env: Env) {
    this.state = state;
    this.env = env;
    // Versioned, idempotent schema bootstrap before serving.
    this.ready = state.blockConcurrencyWhile(async () => {
      const sql = this.state.storage.sql;
      sql.exec("CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)");
      sql.exec("CREATE TABLE IF NOT EXISTS kv (key TEXT PRIMARY KEY, value TEXT NOT NULL)");
      // Slice A (#9954): blob references. Blob BYTES live only in
      // R2, content-addressed by sha256; this table is the sole
      // access path — a hash with no row here is invisible to this
      // user, even if the bytes exist in the bucket for someone
      // else.
      sql.exec(
        "CREATE TABLE IF NOT EXISTS blobs (hash TEXT PRIMARY KEY, size INTEGER NOT NULL, owner TEXT NOT NULL, created_at TEXT NOT NULL)",
      );
      sql.exec(
        "INSERT INTO meta (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        "schema_version",
        String(SCHEMA_VERSION),
      );
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
      const blobHashMatch = url.pathname.match(/^\/v1\/blobs\/([0-9a-f]{64})$/);
      if (url.pathname === "/v1/blobs" && req.method === "PUT") {
        const bytes = new Uint8Array(await req.arrayBuffer());
        const digest = await crypto.subtle.digest("SHA-256", bytes);
        const hash = [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
        const bucket = blobsBindingFor(this.env, jurisdiction as Jurisdiction);
        // Content-addressed dedup: identical bytes are stored
        // once, whoever uploads them; the reference row below is
        // what grants THIS user access.
        const existing = await bucket.head(hash);
        if (!existing) await bucket.put(hash, bytes);
        this.state.storage.sql.exec(
          "INSERT INTO blobs (hash, size, owner, created_at) VALUES (?, ?, ?, ?) ON CONFLICT(hash) DO NOTHING",
          hash,
          bytes.byteLength,
          bindingId,
          new Date().toISOString(),
        );
        return json({ hash, size: bytes.byteLength, stored: true, deduped: !!existing });
      }
      if (blobHashMatch && req.method === "GET") {
        const hash = blobHashMatch[1];
        const rows = this.state.storage.sql
          .exec("SELECT size FROM blobs WHERE hash = ?", hash)
          .toArray();
        if (rows.length === 0) return json({ error: "not_found" }, 404);
        const bucket = blobsBindingFor(this.env, jurisdiction as Jurisdiction);
        const obj = await bucket.get(hash);
        if (!obj) return json({ error: "not_found" }, 404);
        return new Response(obj.body, {
          status: 200,
          headers: {
            "content-type": "application/octet-stream",
            "x-blob-hash": hash,
            "x-blob-size": String(rows[0].size),
          },
        });
      }
      if (blobHashMatch && req.method === "DELETE") {
        const hash = blobHashMatch[1];
        const rows = this.state.storage.sql
          .exec("SELECT hash FROM blobs WHERE hash = ?", hash)
          .toArray();
        if (rows.length === 0) return json({ error: "not_found" }, 404);
        this.state.storage.sql.exec("DELETE FROM blobs WHERE hash = ?", hash);
        // The reference is gone, so this user can no longer reach
        // the bytes. The R2 object itself is deliberately NOT
        // deleted here: identical content may be referenced by
        // other users' objects, whose references this DO cannot
        // see, and deleting shared bytes would break their reads.
        // Byte reclamation is the GC work (#9935 family), never a
        // side effect of one user's delete.
        return json({ hash, deleted: true });
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
