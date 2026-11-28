// The D1-backed hosted event-store adapter (#15538 slice 3).
//
// One tenant database per tenant: the caller passes the tenant's own
// D1 binding in, and the adapter never names another tenant's
// database. Isolation is structural — there is no tenant selector,
// no cross-tenant query, and no credential to leak or replay. The
// only bearer credential in the system is the platform's
// account-level API token held by provisioning, never on this path.
//
// This slice covers the append/read/head seam only: append one signed
// event's exact bytes under (author, seq, id), read the event back,
// and read the author's chain head. Fork, waiting, blob and export
// behavior stays in `bridge-store.ts` on the Durable Object path.
//
// Oracle: the #15538 acceptance criterion — a D1 adapter appends and
// reads an event plus chain head through an explicit per-tenant D1
// binding, using D1's asynchronous binding API, with no Turso
// credential or broad tenant-path token.

// The small async statement surface this adapter uses. It mirrors
// the D1 binding API (`prepare`/`bind`/`run`/`all`/`first`), so a
// real Worker passes `env.TENANT_DB` straight in and tests pass an
// in-memory fake.
export interface D1BoundStatement {
  bind(...values: Array<string | number | null | Uint8Array | ArrayBuffer>): D1BoundStatement;
  run(): Promise<unknown>;
  all<T extends Record<string, unknown>>(): Promise<{ results: T[] }>;
  first<T extends Record<string, unknown>>(): Promise<T | null>;
}

export interface D1Binding {
  prepare(query: string): D1BoundStatement;
  batch<T = unknown>(statements: D1BoundStatement[]): Promise<T[]>;
}

export interface D1StoredEvent {
  author: string;
  seq: number;
  id: string;
  bytes: Uint8Array;
}

export interface D1ChainHead {
  next_seq: number;
  head: string | null;
}

function blobToBytes(value: unknown): Uint8Array {
  if (value instanceof Uint8Array) return value;
  if (value instanceof ArrayBuffer) return new Uint8Array(value);
  if (Array.isArray(value)) return Uint8Array.from(value as number[]);
  throw new Error("D1 event bytes did not return bytes");
}

export const D1_BRIDGE_SCHEMA: readonly string[] = [
  `CREATE TABLE IF NOT EXISTS bridge_events (
     author TEXT NOT NULL, seq INTEGER NOT NULL, id TEXT NOT NULL,
     bytes BLOB NOT NULL,
     PRIMARY KEY (author, seq))`,
  `CREATE UNIQUE INDEX IF NOT EXISTS bridge_events_by_id ON bridge_events(id)`,
  `CREATE TABLE IF NOT EXISTS bridge_chains (
     author TEXT PRIMARY KEY, next_seq INTEGER NOT NULL,
     head TEXT)`,
];

export class D1BridgeStore {
  constructor(private readonly db: D1Binding) {
    if (!db || typeof db.prepare !== "function") {
      throw new Error("D1BridgeStore requires a per-tenant D1 binding");
    }
  }

  async ensureSchema(): Promise<void> {
    for (const statement of D1_BRIDGE_SCHEMA) {
      await this.db.prepare(statement).run();
    }
  }

  /** Append one signed event's exact bytes. The sequence must extend
   * the author's stored prefix; anything else is a verification
   * refusal, never a silent overwrite. */
  async appendEvent(event: D1StoredEvent): Promise<D1ChainHead> {
    if (!event.author || !event.id || !Number.isSafeInteger(event.seq) || event.seq < 0) {
      throw new Error("malformed event: author, seq and id are required");
    }
    if (!(event.bytes instanceof Uint8Array) || event.bytes.length === 0) {
      throw new Error("malformed event: bytes are required");
    }
    const head = await this.chainHead(event.author);
    if (event.seq !== head.next_seq) {
      throw new Error(`sequence_gap: author ${event.author} is at ${head.next_seq}, append names ${event.seq}`);
    }
    const next: D1ChainHead = { next_seq: event.seq + 1, head: event.id };
    await this.db.batch([
      this.db
        .prepare("INSERT INTO bridge_events (author, seq, id, bytes) VALUES (?, ?, ?, ?)")
        .bind(event.author, event.seq, event.id, event.bytes),
      this.db
        .prepare("INSERT OR REPLACE INTO bridge_chains (author, next_seq, head) VALUES (?, ?, ?)")
        .bind(event.author, next.next_seq, next.head),
    ]);
    return next;
  }

  async storedEvent(author: string, seq: number): Promise<D1StoredEvent | null> {
    const row = await this.db
      .prepare("SELECT id, bytes FROM bridge_events WHERE author = ? AND seq = ?")
      .bind(author, seq)
      .first<{ id: unknown; bytes: unknown }>();
    if (row === null) return null;
    return { author, seq, id: String(row.id), bytes: blobToBytes(row.bytes) };
  }

  async chainHead(author: string): Promise<D1ChainHead> {
    const row = await this.db
      .prepare("SELECT next_seq, head FROM bridge_chains WHERE author = ?")
      .bind(author)
      .first<{ next_seq: unknown; head: unknown }>();
    if (row === null) return { next_seq: 0, head: null };
    return { next_seq: Number(row.next_seq), head: (row.head as string | null) ?? null };
  }
}
