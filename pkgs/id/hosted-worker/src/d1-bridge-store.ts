// The hosted bridge store's event log on a per-tenant D1 database
// (#15538 slice 3; the 2026-11-26 amendment to the #15482 record:
// "Tenant D1 access is binding-scoped, not token-scoped").
//
// This is the first D1 slice, not the whole store. It appends one
// signed event at a time to the tenant's own D1 binding and reads
// back the event bytes and the author's chain head. The table shapes
// are the `bridge_events` and `bridge_chains` rows of BRIDGE_SCHEMA,
// so a later slice can move the rest of the ingest (forks, the
// waiting pool, proof headers) onto the same tables.
//
// What it deliberately does not do yet:
// - Forks and out-of-order arrivals are refused here with a named
//   reason, never stored or resolved. The full decision procedure in
//   bridge-store.ts `decide` stays the oracle for those cases.
// - No rehoming of existing UserDO rows, no dual-write, no registry
//   flip, and no production binding (#15538 scope).
//
// Tenant isolation is the binding itself: the caller passes the one
// D1 binding that the authoritative tenant record names. This module
// takes no tenant id, database id, token, or connection string, so
// it has no way to address another tenant's database.
//
// D1's Worker API is asynchronous (prepare/bind/first/all/batch),
// unlike the Durable Object's synchronous `SqlStorage.exec`, which is
// why this is a separate adapter seam rather than a SqlStorageLike.

import { BRIDGE_SCHEMA } from "./bridge-store.ts";
import { decodeEvent, Malformed } from "./event.ts";

/** The subset of a D1 prepared statement this adapter uses. */
export interface D1PreparedStatementLike {
  bind(...values: unknown[]): D1PreparedStatementLike;
  first<T = Record<string, unknown>>(): Promise<T | null>;
  all<T = Record<string, unknown>>(): Promise<{ results?: T[] }>;
  run(): Promise<unknown>;
}

/** The subset of a D1 database binding this adapter uses. */
export interface D1DatabaseLike {
  prepare(query: string): D1PreparedStatementLike;
  batch(statements: D1PreparedStatementLike[]): Promise<unknown[]>;
}

/** The two BRIDGE_SCHEMA tables (and the id index) this slice owns:
 * the same DDL text, so the D1 rows and the DO rows are one shape. */
export const D1_BRIDGE_SCHEMA: string[] = BRIDGE_SCHEMA.filter((statement) =>
  /\b(bridge_events|bridge_events_by_id|bridge_chains)\b/.test(statement.split("(")[0]),
);

export type D1AppendOutcome =
  | { kind: "accepted"; id: string; author: string; seq: number }
  | { kind: "duplicate"; id: string }
  | {
      kind: "refused";
      reason: "malformed" | "not_next";
      message: string;
    };

export interface D1ChainHead {
  author: string;
  next_seq: number;
  head: string | null;
}

export interface D1StoredEvent {
  author: string;
  seq: number;
  id: string;
  bytes: Uint8Array;
}

function blobBytes(value: unknown): Uint8Array {
  if (value instanceof Uint8Array) return value;
  if (value instanceof ArrayBuffer) return new Uint8Array(value);
  if (ArrayBuffer.isView(value)) {
    return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
  }
  // Local Wrangler D1 returns a BLOB column as an array of numbers;
  // accept it only when every element is a byte.
  if (Array.isArray(value) && value.every((b) => Number.isInteger(b) && b >= 0 && b <= 0xff)) {
    return Uint8Array.from(value as number[]);
  }
  throw new Error("D1 returned a non-BLOB event value");
}

/** D1 binds BLOBs from an ArrayBuffer; copy so the bound value is
 * exactly the event's bytes and never a larger backing buffer. */
function toArrayBuffer(bytes: Uint8Array): ArrayBuffer {
  const copy = new Uint8Array(bytes.byteLength);
  copy.set(bytes);
  return copy.buffer;
}

export class D1BridgeStore {
  private readonly db: D1DatabaseLike;
  private schemaReady: Promise<void> | undefined;

  constructor(db: D1DatabaseLike) {
    this.db = db;
  }

  /** Creates the slice's tables in this tenant's database if absent. */
  ensureSchema(): Promise<void> {
    if (!this.schemaReady) {
      this.schemaReady = this.db
        .batch(D1_BRIDGE_SCHEMA.map((statement) => this.db.prepare(statement)))
        .then(() => undefined);
      this.schemaReady.catch(() => {
        this.schemaReady = undefined;
      });
    }
    return this.schemaReady;
  }

  async chainOf(author: string): Promise<D1ChainHead> {
    await this.ensureSchema();
    const row = await this.db
      .prepare("SELECT next_seq, head FROM bridge_chains WHERE author = ?")
      .bind(author)
      .first<{ next_seq: number; head: string | null }>();
    if (!row) return { author, next_seq: 0, head: null };
    return { author, next_seq: Number(row.next_seq), head: row.head ?? null };
  }

  async storedAt(author: string, seq: number): Promise<D1StoredEvent | null> {
    await this.ensureSchema();
    const row = await this.db
      .prepare("SELECT id, bytes FROM bridge_events WHERE author = ? AND seq = ?")
      .bind(author, seq)
      .first<{ id: string; bytes: unknown }>();
    if (!row) return null;
    return { author, seq, id: row.id, bytes: blobBytes(row.bytes) };
  }

  async eventById(id: string): Promise<D1StoredEvent | null> {
    await this.ensureSchema();
    const row = await this.db
      .prepare("SELECT author, seq, bytes FROM bridge_events WHERE id = ?")
      .bind(id)
      .first<{ author: string; seq: number; bytes: unknown }>();
    if (!row) return null;
    return { author: row.author, seq: Number(row.seq), id, bytes: blobBytes(row.bytes) };
  }

  /** Verifies and appends one signed event. Accepted only as the
   * author's next contiguous event: `seq` equals the chain's
   * `next_seq` and the backlink names the current head. Anything
   * else is refused, never stored, so a fork is not silently
   * resolved by this slice. The event row and the head move in one
   * D1 batch (a single transaction). */
  async append(bytes: Uint8Array): Promise<D1AppendOutcome> {
    let event;
    try {
      event = await decodeEvent(bytes);
    } catch (error) {
      if (error instanceof Malformed) {
        return { kind: "refused", reason: "malformed", message: error.message };
      }
      throw error;
    }
    if (await this.eventById(event.id)) return { kind: "duplicate", id: event.id };

    const chain = await this.chainOf(event.authorHex);
    if (event.seq !== chain.next_seq || event.backlinkHex !== chain.head) {
      return {
        kind: "refused",
        reason: "not_next",
        message:
          `expected sequence ${chain.next_seq} after head ${chain.head ?? "none"}; ` +
          "forks and out-of-order events go through the full bridge ingest",
      };
    }

    await this.db.batch([
      this.db
        .prepare("INSERT INTO bridge_events (author, seq, id, bytes) VALUES (?, ?, ?, ?)")
        .bind(event.authorHex, event.seq, event.id, toArrayBuffer(event.bytes)),
      // The WHERE guard makes a concurrent writer that moved the head
      // first lose: the event INSERT above then conflicts on
      // (author, seq), failing the whole batch.
      this.db
        .prepare(
          `INSERT INTO bridge_chains (author, next_seq, head, frozen) VALUES (?, ?, ?, 0)
           ON CONFLICT(author) DO UPDATE SET next_seq = excluded.next_seq, head = excluded.head
           WHERE bridge_chains.next_seq = ?`,
        )
        .bind(event.authorHex, event.seq + 1, event.id, chain.next_seq),
    ]);
    return { kind: "accepted", id: event.id, author: event.authorHex, seq: event.seq };
  }
}
