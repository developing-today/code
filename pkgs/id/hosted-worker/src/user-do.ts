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

import {
  FailClosed,
  blobsBindingFor,
  parseBindingId,
  parseJurisdiction,
  type Jurisdiction,
  type R2BucketLike,
} from "./routing.ts";

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
  /** Slice B (#9955): per-user storage ceiling in bytes, as a
   * string var. Absent/empty = no ceiling. A malformed value
   * fails writes closed (see ceilingBytes). */
  USER_CEILING_BYTES?: string;
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function ceilingExceeded(headroom: number): FailClosed {
  return new FailClosed(
    413,
    "ceiling_exceeded",
    `storage ceiling exceeded: value is larger than the ${headroom} bytes of headroom; reads and export remain available`,
  );
}

/** Reads a request body as text, giving up as soon as it exceeds
 * `limit` bytes so an oversized body is never buffered whole. */
async function readTextUpTo(req: Request, limit: number): Promise<string | null> {
  if (!req.body) return "";
  const reader = req.body.getReader();
  const decoder = new TextDecoder();
  const chunks: Uint8Array[] = [];
  let total = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    total += value.byteLength;
    if (total > limit) {
      await reader.cancel();
      return null;
    }
    chunks.push(value);
  }
  const bytes = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return decoder.decode(bytes);
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

  /** The configured storage ceiling in bytes; null = no ceiling.
   * A malformed value is a configuration error: writes fail
   * closed rather than silently running unceilinged. */
  private ceilingBytes(): number | null {
    const raw = this.env.USER_CEILING_BYTES;
    if (raw === undefined || raw === "") return null;
    const n = Number(raw);
    if (!Number.isInteger(n) || n < 0) {
      throw new FailClosed(
        500,
        "ceiling_misconfigured",
        "USER_CEILING_BYTES is not a non-negative integer; writes fail closed",
      );
    }
    return n;
  }

  /** Usage accounting, derived from the tables on every read so
   * the numbers can never drift from the data they describe.
   * blob_bytes counts bytes referenced by THIS user (a deduped
   * blob referenced by two users counts toward both — the
   * reference is what each user's storage pays for). */
  private usage() {
    const sql = this.state.storage.sql;
    // LENGTH on TEXT counts characters; casting to BLOB counts the
    // UTF-8 bytes the ceiling is defined in (#11349).
    const kv = sql
      .exec("SELECT COUNT(*) AS n, COALESCE(SUM(LENGTH(CAST(value AS BLOB))), 0) AS bytes FROM kv")
      .toArray()[0];
    const blobs = sql.exec("SELECT COUNT(*) AS n, COALESCE(SUM(size), 0) AS bytes FROM blobs").toArray()[0];
    const kvBytes = Number(kv.bytes);
    const blobBytes = Number(blobs.bytes);
    return {
      requests: Number(this.meta("requests") ?? "0"),
      kv_count: Number(kv.n),
      kv_bytes: kvBytes,
      blob_count: Number(blobs.n),
      blob_bytes: blobBytes,
      storage_bytes: kvBytes + blobBytes,
    };
  }

  /** Fail a write closed when it would push storage over the
   * ceiling. Reads, export, and deletes never call this: at cap
   * the object is read-only + exportable, and deletes are how a
   * user recovers headroom. */
  private assertWritable(deltaBytes: number) {
    const ceiling = this.ceilingBytes();
    if (ceiling === null) return;
    const after = this.usage().storage_bytes + deltaBytes;
    if (after > ceiling) {
      throw new FailClosed(
        413,
        "ceiling_exceeded",
        `storage ceiling exceeded: ${after} bytes would exceed the ${ceiling}-byte ceiling; reads and export remain available`,
      );
    }
  }

  /** The largest value (bytes) a KV PUT over `priorBytes` of existing
   * data can carry and still pass assertWritable; null = no ceiling. */
  private kvHeadroom(priorBytes: number): number | null {
    const ceiling = this.ceilingBytes();
    if (ceiling === null) return null;
    return ceiling - this.usage().storage_bytes + priorBytes;
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
      this.state.storage.sql.exec(
        "INSERT INTO meta (key, value) VALUES ('requests', '1') ON CONFLICT(key) DO UPDATE SET value = CAST(value AS INTEGER) + 1",
      );
      const url = new URL(req.url);
      if (url.pathname === "/v1/usage" && req.method === "GET") {
        const usage = this.usage();
        const ceiling = this.ceilingBytes();
        return json({
          binding_id: bindingId,
          jurisdiction,
          ...usage,
          ceiling_bytes: ceiling,
          at_cap: ceiling !== null && usage.storage_bytes >= ceiling,
        });
      }
      if (url.pathname === "/v1/export" && req.method === "GET") {
        // Export stays available at cap by design (#3811): it is
        // a read, and it is how a capped user gets their data out.
        const kvRows = this.state.storage.sql.exec("SELECT key, value FROM kv ORDER BY key").toArray();
        const blobRows = this.state.storage.sql
          .exec("SELECT hash, size, created_at FROM blobs ORDER BY hash")
          .toArray();
        return json({
          binding_id: bindingId,
          jurisdiction,
          usage: this.usage(),
          kv: Object.fromEntries(kvRows.map((r) => [String(r.key), String(r.value)])),
          blobs: blobRows.map((r) => ({
            hash: String(r.hash),
            size: Number(r.size),
            created_at: String(r.created_at),
          })),
        });
      }
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
        const priorRef = this.state.storage.sql.exec("SELECT hash FROM blobs WHERE hash = ?", hash).toArray();
        this.assertWritable(priorRef.length === 0 ? bytes.byteLength : 0);
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
        const rows = this.state.storage.sql.exec("SELECT size FROM blobs WHERE hash = ?", hash).toArray();
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
        const rows = this.state.storage.sql.exec("SELECT hash FROM blobs WHERE hash = ?", hash).toArray();
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
        const prior = this.state.storage.sql
          .exec("SELECT LENGTH(CAST(value AS BLOB)) AS n FROM kv WHERE key = ?", kvMatch[1])
          .toArray();
        const priorBytes = prior.length === 0 ? 0 : Number(prior[0].n);
        const headroom = this.kvHeadroom(priorBytes);
        let value: string | null;
        if (headroom === null) {
          value = await req.text();
        } else {
          const declared = req.headers.get("content-length");
          if (declared !== null && Number(declared) > headroom) throw ceilingExceeded(headroom);
          value = await readTextUpTo(req, headroom);
          if (value === null) throw ceilingExceeded(headroom);
        }
        this.assertWritable(new TextEncoder().encode(value).byteLength - priorBytes);
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
      if (kvMatch && req.method === "DELETE") {
        // Deletes only shrink usage, so they stay available at
        // cap — they are how a capped user recovers headroom.
        const rows = this.state.storage.sql.exec("SELECT key FROM kv WHERE key = ?", kvMatch[1]).toArray();
        if (rows.length === 0) return json({ error: "not_found" }, 404);
        this.state.storage.sql.exec("DELETE FROM kv WHERE key = ?", kvMatch[1]);
        return json({ key: kvMatch[1], deleted: true });
      }
      return json({ error: "not_found" }, 404);
    } catch (e) {
      if (e instanceof FailClosed) return json({ error: e.code, message: e.message }, e.status);
      throw e;
    }
  }
}
