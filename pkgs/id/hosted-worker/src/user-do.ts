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
import { issueNonce, verifyProof, type AuthContext } from "./bridge-auth.ts";
import {
  BRIDGE_SCHEMA,
  BridgeStore,
  DEFAULT_WAITING_LIMITS,
  type WaitingLimits,
} from "./bridge-store.ts";
import {
  BridgeSessionServer,
  decodeBatch,
  decodeEntries,
  encodeBatch,
  DEFAULT_BRIDGE_LIMITS,
  type FrameTransport,
} from "./bridge-session.ts";
import type { IngestOutcome } from "./bridge-store.ts";

export const SCHEMA_VERSION = 3;

export const HDR_BINDING = "x-hosted-binding-id";
export const HDR_JURISDICTION = "x-hosted-jurisdiction";
export const HDR_ROUTED = "x-hosted-routed";
/** The bound ID key (the binding record's current Iroh root key,
 * hex), resolved by the authenticated internal caller from the
 * #3808 registry. Consumed only by the bridge routes (#3818): the
 * bridge authenticates callers by proof against this key, and a
 * request whose routing context names no valid key fails closed. */
export const HDR_IROH = "x-hosted-iroh-id";

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
  /** Bridge (#3818): waiting-pool bounds for the signed-object
   * bridge, as string vars. Absent = the native defaults
   * (bridge-store.ts). A malformed value fails bridge writes
   * closed (see bridgeLimits). */
  BRIDGE_WAITING_PER_AUTHOR?: string;
  BRIDGE_WAITING_TOTAL?: string;
  BRIDGE_WAITING_MAX_AGE_MS?: string;
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

/** Hard per-request body bound for every mutating route (#11337),
 * matching the platform's default request size. It only bounds
 * memory; the storage ceiling stays the authority on what fits. */
export const MAX_BODY_BYTES = 100 * 1024 * 1024;

/** The most body bytes any mutating route will read for this
 * deployment: the storage ceiling when one is configured (no single
 * write can exceed the whole ceiling), capped at MAX_BODY_BYTES.
 * Shared by the front door and the DO so the two cannot drift. A
 * malformed ceiling falls back to MAX_BODY_BYTES here; the DO's own
 * ceiling check still fails those writes closed. */
export function requestBodyLimit(rawCeiling: string | undefined): number {
  if (rawCeiling === undefined || rawCeiling === "") return MAX_BODY_BYTES;
  const n = Number(rawCeiling);
  if (!Number.isInteger(n) || n < 0) return MAX_BODY_BYTES;
  return Math.min(n, MAX_BODY_BYTES);
}

export function bodyTooLarge(limit: number): FailClosed {
  return new FailClosed(413, "body_too_large", `request body is larger than the ${limit}-byte limit`);
}

/** Reads a request body, giving up as soon as it exceeds `limit`
 * bytes so an oversized body is never buffered whole. */
async function readBytesUpTo(req: Request, limit: number): Promise<Uint8Array<ArrayBuffer> | null> {
  if (!req.body) return new Uint8Array(0);
  const reader = req.body.getReader();
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
  return bytes;
}

async function readTextUpTo(req: Request, limit: number): Promise<string | null> {
  const bytes = await readBytesUpTo(req, limit);
  return bytes === null ? null : new TextDecoder().decode(bytes);
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
      // The signed-object bridge (#3818): the chain-store mirror
      // (events, chains, fork proofs, waiting pool) in this same
      // per-user database. IF NOT EXISTS throughout, so an object
      // created before the bridge gains the tables on its next
      // bootstrap with its data untouched.
      for (const statement of BRIDGE_SCHEMA) sql.exec(statement);
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
    // Bridge event bytes are this user's storage like any other:
    // they count toward the same ceiling.
    const bridgeBytes = this.bridgeStore().storedBytes();
    return {
      requests: Number(this.meta("requests") ?? "0"),
      kv_count: Number(kv.n),
      kv_bytes: kvBytes,
      blob_count: Number(blobs.n),
      blob_bytes: blobBytes,
      bridge_bytes: bridgeBytes,
      storage_bytes: kvBytes + blobBytes + bridgeBytes,
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

  /** The bridge store over this object's database, with the
   * configured waiting-pool bounds. */
  private bridgeStore(): BridgeStore {
    return new BridgeStore(this.state.storage.sql, this.bridgeLimits());
  }

  /** Bridge waiting-pool bounds from the environment; a malformed
   * value fails closed, like the storage ceiling. */
  private bridgeLimits(): WaitingLimits {
    const parse = (raw: string | undefined, fallback: number, name: string): number => {
      if (raw === undefined || raw === "") return fallback;
      const n = Number(raw);
      if (!Number.isInteger(n) || n < 0) {
        throw new FailClosed(
          500,
          "bridge_limits_misconfigured",
          `${name} is not a non-negative integer; bridge writes fail closed`,
        );
      }
      return n;
    };
    return {
      perAuthor: parse(this.env.BRIDGE_WAITING_PER_AUTHOR, DEFAULT_WAITING_LIMITS.perAuthor, "BRIDGE_WAITING_PER_AUTHOR"),
      total: parse(this.env.BRIDGE_WAITING_TOTAL, DEFAULT_WAITING_LIMITS.total, "BRIDGE_WAITING_TOTAL"),
      maxAgeMs: parse(this.env.BRIDGE_WAITING_MAX_AGE_MS, DEFAULT_WAITING_LIMITS.maxAgeMs, "BRIDGE_WAITING_MAX_AGE_MS"),
    };
  }

  /** The bound ID key from the routing context. Bridge routes fail
   * closed without one: there is no key to authenticate against. */
  private requireIroh(req: Request): string {
    const raw = req.headers.get(HDR_IROH);
    if (raw === null || !/^[0-9a-f]{64}$/.test(raw)) {
      throw new FailClosed(
        503,
        "id_key_unavailable",
        "the binding record names no ID key for this user; bridge routes fail closed",
      );
    }
    return raw;
  }

  /** Verifies an HTTPS bridge leg's ID-key proof (bridge-auth.ts):
   * the nonce and signature travel as headers, the body is the
   * exact request body. Returns the store to act on. */
  private async bridgeProof(
    req: Request,
    bindingId: string,
    irohHex: string,
    context: AuthContext,
    body: Uint8Array,
  ): Promise<BridgeStore> {
    const nonce = req.headers.get("x-id-nonce");
    const signature = req.headers.get("x-id-proof");
    if (!nonce || !signature) {
      throw new FailClosed(401, "bridge_proof_required", "bridge calls carry an x-id-nonce and x-id-proof header");
    }
    const store = this.bridgeStore();
    const ok = await verifyProof(store, {
      bindingId,
      irohHex,
      context,
      body,
      nonce,
      signatureHex: signature,
      nowMs: Date.now(),
    });
    if (!ok) {
      throw new FailClosed(403, "bridge_proof_invalid", "the ID-key proof did not verify");
    }
    return store;
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
      // ----- the signed-object bridge (#3818) -----
      // Every bridge route additionally requires the bound ID key
      // in the routing context and, for the HTTPS legs, a fresh
      // ID-key proof per call (bridge-auth.ts). The WebSocket leg
      // proves itself in-band (CHALLENGE/AUTH) instead.
      if (url.pathname === "/v1/bridge/challenge" && req.method === "GET") {
        this.requireIroh(req);
        const { nonce, expiresAt } = issueNonce(this.bridgeStore(), Date.now());
        return json({ nonce, expires_at: new Date(expiresAt).toISOString() });
      }
      if (url.pathname === "/v1/bridge/heads" && req.method === "GET") {
        const irohHex = this.requireIroh(req);
        const store = await this.bridgeProof(req, bindingId, irohHex, "heads", new Uint8Array(0));
        return json({ heads: store.heads() });
      }
      if (url.pathname === "/v1/bridge/push" && req.method === "POST") {
        const irohHex = this.requireIroh(req);
        const body = await readBytesUpTo(req, DEFAULT_BRIDGE_LIMITS.maxBatchBytes);
        if (body === null) {
          throw new FailClosed(413, "batch_too_large", `push body is larger than the ${DEFAULT_BRIDGE_LIMITS.maxBatchBytes}-byte batch bound`);
        }
        const store = await this.bridgeProof(req, bindingId, irohHex, "push", body);
        const rawEvents = decodeBatch(body, DEFAULT_BRIDGE_LIMITS);
        const entries = await decodeEntries(rawEvents);
        // The ceiling is checked before anything is written, over
        // the bytes this batch would add (verified events the store
        // does not already hold in any form).
        let newBytes = 0;
        for (const entry of entries) {
          if ("malformed" in entry) continue;
          if (!store.heldEvent(entry.id)) newBytes += entry.bytes.length;
        }
        this.assertWritable(newBytes);
        const outcomes = store.ingestBatch(entries, Date.now());
        return json({ outcomes: outcomes.map(serializeOutcome), heads: store.heads() });
      }
      if (url.pathname === "/v1/bridge/pull" && req.method === "POST") {
        const irohHex = this.requireIroh(req);
        const text = await readTextUpTo(req, 64 * 1024);
        if (text === null) throw bodyTooLarge(64 * 1024);
        const store = await this.bridgeProof(
          req,
          bindingId,
          irohHex,
          "pull",
          new TextEncoder().encode(text),
        );
        let positions = new Map<string, number>();
        let maxEvents = DEFAULT_BRIDGE_LIMITS.maxBatchEvents;
        try {
          const parsed = JSON.parse(text) as {
            positions?: { author: string; next_seq: number }[];
            max_events?: number;
          };
          for (const p of parsed.positions ?? []) positions.set(p.author, p.next_seq);
          if (parsed.max_events !== undefined) {
            maxEvents = Math.max(1, Math.min(512, Math.floor(parsed.max_events)));
          }
        } catch {
          throw new FailClosed(400, "invalid_pull", "pull body must be JSON positions");
        }
        // The sync set beyond the caller's per-author frontiers, in
        // key order, bounded in count and bytes. Positions are the
        // resumption mechanism: a caller that lost state pulls from
        // zero and replays into its empty store.
        const entries = store
          .syncEntries()
          .filter((e) => e.seq >= (positions.get(e.author) ?? 0))
          .sort((a, b) => (a.author < b.author ? -1 : a.author > b.author ? 1 : a.seq - b.seq));
        const batch: Uint8Array[] = [];
        let batchBytes = 0;
        let more = false;
        for (const entry of entries) {
          const held = store.heldEvent(entry.id);
          if (!held) continue;
          if (batch.length >= maxEvents || batchBytes + held.bytes.length > DEFAULT_BRIDGE_LIMITS.maxBatchBytes) {
            more = true;
            break;
          }
          batch.push(held.bytes);
          batchBytes += held.bytes.length;
        }
        return new Response(encodeBatch(batch), {
          status: 200,
          headers: {
            "content-type": "application/octet-stream",
            "x-bridge-more": more ? "1" : "0",
            "x-bridge-heads": JSON.stringify(store.heads()),
          },
        });
      }
      if (url.pathname === "/v1/bridge/ws" && req.method === "GET") {
        const irohHex = this.requireIroh(req);
        if ((req.headers.get("upgrade") ?? "").toLowerCase() !== "websocket") {
          throw new FailClosed(400, "websocket_required", "the bridge session leg requires a WebSocket upgrade");
        }
        const pair = new WebSocketPair();
        const client = pair[0];
        const server = pair[1];
        server.accept();
        new BridgeSessionServer({
          store: this.bridgeStore(),
          bindingId,
          irohHex,
          transport: new WsTransport(server),
          guardBatch: (newBytes) => {
            try {
              this.assertWritable(newBytes);
              return true;
            } catch {
              return false;
            }
          },
        });
        return new Response(null, { status: 101, webSocket: client } as unknown as ResponseInit);
      }
      const blobHashMatch = url.pathname.match(/^\/v1\/blobs\/([0-9a-f]{64})$/);
      if (url.pathname === "/v1/blobs" && req.method === "PUT") {
        // Bounded read (#11337): a blob larger than the whole
        // ceiling can never fit, so the ceiling bounds the read too.
        const limit = requestBodyLimit(this.env.USER_CEILING_BYTES);
        const tooLarge = () => (this.ceilingBytes() === null ? bodyTooLarge(limit) : ceilingExceeded(limit));
        const declared = req.headers.get("content-length");
        if (declared !== null && Number(declared) > limit) throw tooLarge();
        const bytes = await readBytesUpTo(req, limit);
        if (bytes === null) throw tooLarge();
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
          const limit = requestBodyLimit(this.env.USER_CEILING_BYTES);
          const declared = req.headers.get("content-length");
          if (declared !== null && Number(declared) > limit) throw bodyTooLarge(limit);
          value = await readTextUpTo(req, limit);
          if (value === null) throw bodyTooLarge(limit);
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

/** An ingest outcome as the push leg reports it: the same record
 * the native store returns, JSON-shaped. */
function serializeOutcome(outcome: IngestOutcome): Record<string, unknown> {
  switch (outcome.kind) {
    case "accepted":
      return { kind: "accepted", id: outcome.id, released: outcome.released };
    case "duplicate":
      return { kind: "duplicate", id: outcome.id };
    case "fork":
      return { kind: "fork", author: outcome.author, seq: outcome.seq };
    case "waiting":
      return { kind: "waiting", id: outcome.id };
    case "refused":
      return { kind: "refused", reason: outcome.reason, message: outcome.message };
  }
}

/** The minimal shape of a workerd WebSocket the session transport
 * needs (the package carries no workers-types dependency). */
declare const WebSocketPair: new () => [WsSocketLike, WsSocketLike];

interface WsSocketLike {
  accept(): void;
  send(data: Uint8Array): void;
  close(): void;
  addEventListener(type: string, cb: (event: { data?: unknown }) => void): void;
}

/** Adapts a workerd server WebSocket to the bridge session's
 * message transport. Binary messages only — the bridge has no text
 * frames, and anything else is ignored rather than guessed at. */
class WsTransport implements FrameTransport {
  constructor(private socket: WsSocketLike) {}

  send(data: Uint8Array): void {
    this.socket.send(data);
  }

  onMessage(cb: (data: Uint8Array) => void): void {
    this.socket.addEventListener("message", (event) => {
      if (event.data instanceof ArrayBuffer) cb(new Uint8Array(event.data));
      else if (event.data instanceof Uint8Array) cb(event.data);
    });
  }

  onClose(cb: () => void): void {
    this.socket.addEventListener("close", () => cb());
  }

  close(): void {
    this.socket.close();
  }
}
