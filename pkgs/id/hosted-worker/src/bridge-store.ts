// The hosted bridge store: pkgs/id's native ingest semantics
// (crates/id-events event_store::ingest + id-store waiting) mirrored
// onto the Durable Object's SQLite, the same mirror the #2276 proof
// of concept validated (tests/bridge_poc.rs `HostedStore`).
//
// The store is a chain store, not a snapshot: it keeps every stored
// event's original signed bytes, the fork proofs (two header byte
// strings per frozen point — the evidence, not a verdict), the proof
// headers' presence, and the waiting pool with its dependency edges.
// The applied prefix of an author's chain is derived from stored
// events and the lowest fork proof, so arrival order cannot change
// the result. Nothing here assigns order, re-signs, or vouches for
// content: it holds signed objects and proof-carrying evidence,
// exactly the bridge's contract from the #2046 investigation.
//
// The membership layer that voids stored bodies natively is not part
// of this store (it needs bodies and group resolution); presence —
// what dependency and fork decisions turn on — is unaffected.
//
// Oracle: the native decision procedure, transcribed step for step in
// `decide`; test/bridge-store.test.ts and the session acceptance
// tests compare this store's canonical shape against the semantics
// the Rust proof of concept pins.

import { bytesToHex } from "./ed25519.ts";
import { parseEvent, type SignedEvent } from "./event.ts";

export interface SqlStorageLike {
  exec(
    query: string,
    ...params: (string | number | bigint | boolean | null | ArrayBuffer | Uint8Array)[]
  ): { toArray(): Record<string, unknown>[] };
}

export const BRIDGE_SCHEMA: string[] = [
  `CREATE TABLE IF NOT EXISTS bridge_events (
     author TEXT NOT NULL, seq INTEGER NOT NULL, id TEXT NOT NULL,
     bytes BLOB NOT NULL,
     checkpoint_header BLOB, checkpoint_seq TEXT,
     PRIMARY KEY (author, seq))`,
  `CREATE UNIQUE INDEX IF NOT EXISTS bridge_events_by_id ON bridge_events(id)`,
  `CREATE TABLE IF NOT EXISTS bridge_chains (
     author TEXT PRIMARY KEY, next_seq INTEGER NOT NULL,
     head TEXT, frozen INTEGER NOT NULL)`,
  `CREATE TABLE IF NOT EXISTS bridge_forks (
     author TEXT NOT NULL, seq INTEGER NOT NULL,
     first_id TEXT NOT NULL, second_id TEXT NOT NULL,
     PRIMARY KEY (author, seq))`,
  `CREATE TABLE IF NOT EXISTS bridge_proof_ids (
     id TEXT PRIMARY KEY, author TEXT NOT NULL)`,
  `CREATE TABLE IF NOT EXISTS bridge_proof_headers (
     id TEXT PRIMARY KEY, bytes BLOB NOT NULL)`,
  `CREATE TABLE IF NOT EXISTS bridge_waiting (
     id TEXT PRIMARY KEY, author TEXT NOT NULL, seq INTEGER NOT NULL,
     bytes BLOB NOT NULL,
     arrived_at INTEGER NOT NULL, arrival INTEGER NOT NULL,
     deps TEXT NOT NULL, parked INTEGER NOT NULL,
     checkpoint_header BLOB, checkpoint_seq TEXT)`,
  `CREATE INDEX IF NOT EXISTS bridge_waiting_author_seq ON bridge_waiting(author, seq)`,
  `CREATE TABLE IF NOT EXISTS bridge_waiting_by_dep (
     dep TEXT NOT NULL, id TEXT NOT NULL, PRIMARY KEY (dep, id))`,
  `CREATE INDEX IF NOT EXISTS bridge_waiting_by_dep_id ON bridge_waiting_by_dep(id)`,
  `CREATE TABLE IF NOT EXISTS bridge_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)`,
];

export interface WaitingLimits {
  perAuthor: number;
  total: number;
  maxAgeMs: number;
}

/** crates/id-core waiting::WaitingLimits::default. */
export const DEFAULT_WAITING_LIMITS: WaitingLimits = {
  perAuthor: 256,
  total: 8192,
  maxAgeMs: 7 * 24 * 3600 * 1000,
};

export type IngestOutcome =
  | { kind: "accepted"; id: string; released: string[] }
  | { kind: "duplicate"; id: string }
  | { kind: "fork"; author: string; seq: number }
  | { kind: "waiting"; id: string }
  | { kind: "refused"; reason: "malformed" | "frozen" | "waiting_full"; message: string };

export interface ChainState {
  next_seq: number;
  head: string | null;
  frozen: boolean;
}

export interface HeldEvent {
  author: string;
  seq: number;
  id: string;
  bytes: Uint8Array;
}

type Decision =
  | { t: "duplicate" }
  | { t: "refuse"; reason: "frozen" | "malformed"; message: string }
  | { t: "fork"; first: { id: string; bytes: Uint8Array }; second: { id: string; bytes: Uint8Array } }
  | { t: "wait"; deps: string[] }
  | { t: "park" }
  | { t: "store" };

interface WaitingRow {
  id: string;
  author: string;
  seq: number;
  bytes: Uint8Array;
  arrived_at: number;
  arrival: number;
  deps: string[];
  parked: boolean;
  checkpoint_header: Uint8Array | null;
  checkpoint_seq: string | null;
}

function blobBytes(value: unknown): Uint8Array {
  if (value instanceof Uint8Array) return value;
  if (value instanceof ArrayBuffer) return new Uint8Array(value);
  if (Array.isArray(value)) return new Uint8Array(value as number[]);
  throw new Error("blob column did not return bytes");
}

function twoLowest(candidates: { id: string; bytes: Uint8Array }[]): {
  first: { id: string; bytes: Uint8Array };
  second: { id: string; bytes: Uint8Array };
} {
  const byId = new Map<string, { id: string; bytes: Uint8Array }>();
  for (const c of candidates) byId.set(c.id, c);
  const sorted = [...byId.values()].sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
  return { first: sorted[0], second: sorted[1] };
}

export class BridgeStore {
  constructor(
    private sql: SqlStorageLike,
    private limits: WaitingLimits = DEFAULT_WAITING_LIMITS,
  ) {}

  // ----- reads -----------------------------------------------------

  chainOf(author: string): ChainState {
    const rows = this.sql.exec(
      "SELECT next_seq, head, frozen FROM bridge_chains WHERE author = ?",
      author,
    ).toArray();
    if (rows.length === 0) return { next_seq: 0, head: null, frozen: false };
    return {
      next_seq: Number(rows[0].next_seq),
      head: (rows[0].head as string | null) ?? null,
      frozen: Number(rows[0].frozen) !== 0,
    };
  }

  /** Chain heads for the given authors (or every author the store
   * knows) — the HELLO announcement and the pull frontier source. */
  heads(authors?: string[]): { author: string; next_seq: number; head: string | null }[] {
    const list = authors ?? this.knownAuthors();
    return list.map((author) => {
      const chain = this.chainOf(author);
      return { author, next_seq: chain.next_seq, head: chain.head };
    });
  }

  knownAuthors(): string[] {
    const rows = this.sql.exec(
      `SELECT author FROM bridge_chains
       UNION SELECT author FROM bridge_events
       UNION SELECT author FROM bridge_waiting
       UNION SELECT author FROM bridge_proof_ids
       ORDER BY author`,
    ).toArray();
    return rows.map((r) => r.author as string);
  }

  isStored(id: string): boolean {
    return (
      this.sql.exec("SELECT 1 AS x FROM bridge_events WHERE id = ?", id).toArray().length > 0
    );
  }

  isProofHeader(author: string, id: string): boolean {
    return (
      this.sql.exec(
        "SELECT 1 AS x FROM bridge_proof_ids WHERE author = ? AND id = ?",
        author,
        id,
      ).toArray().length > 0
    );
  }

  /** present(native): stored, or a fork proof header. A waiting
   * event is not present. */
  isPresent(id: string): boolean {
    if (this.isStored(id)) return true;
    return (
      this.sql.exec("SELECT 1 AS x FROM bridge_proof_ids WHERE id = ?", id).toArray().length > 0
    );
  }

  storedAt(author: string, seq: number): { id: string; bytes: Uint8Array } | null {
    const rows = this.sql.exec(
      "SELECT id, bytes FROM bridge_events WHERE author = ? AND seq = ?",
      author,
      seq,
    ).toArray();
    if (rows.length === 0) return null;
    return { id: rows[0].id as string, bytes: blobBytes(rows[0].bytes) };
  }

  /** The sync set (stored events, fork proof headers, and waiting
   * events — the bridge's set, see the design note) as range keys
   * and as fetchable held events. */
  syncEntries(): { author: string; seq: number; id: string }[] {
    // A set of keys, not a list of rows: an event that is both
    // stored and a fork proof header (the applied-then-forked case)
    // is one key, as in the native sync set.
    const byId = new Map<string, { author: string; seq: number; id: string }>();
    const add = (author: string, seq: number, id: string) => {
      if (!byId.has(id)) byId.set(id, { author, seq, id });
    };
    for (const r of this.sql.exec("SELECT author, seq, id FROM bridge_events").toArray()) {
      add(r.author as string, Number(r.seq), r.id as string);
    }
    for (const r of this.sql.exec("SELECT author, seq, first_id, second_id FROM bridge_forks").toArray()) {
      add(r.author as string, Number(r.seq), r.first_id as string);
      add(r.author as string, Number(r.seq), r.second_id as string);
    }
    for (const r of this.sql.exec("SELECT author, seq, id FROM bridge_waiting").toArray()) {
      add(r.author as string, Number(r.seq), r.id as string);
    }
    return [...byId.values()];
  }

  heldEvent(id: string): HeldEvent | null {
    let rows = this.sql.exec(
      "SELECT author, seq, bytes FROM bridge_events WHERE id = ?",
      id,
    ).toArray();
    if (rows.length > 0) {
      return { author: rows[0].author as string, seq: Number(rows[0].seq), id, bytes: blobBytes(rows[0].bytes) };
    }
    rows = this.sql.exec("SELECT bytes FROM bridge_proof_headers WHERE id = ?", id).toArray();
    if (rows.length > 0) {
      const parsed = parseEvent(blobBytes(rows[0].bytes));
      return { author: parsed.authorHex, seq: parsed.seq, id, bytes: parsed.bytes };
    }
    rows = this.sql.exec(
      "SELECT author, seq, bytes FROM bridge_waiting WHERE id = ?",
      id,
    ).toArray();
    if (rows.length > 0) {
      return { author: rows[0].author as string, seq: Number(rows[0].seq), id, bytes: blobBytes(rows[0].bytes) };
    }
    return null;
  }

  heldEvents(): HeldEvent[] {
    return this.syncEntries()
      .map((e) => this.heldEvent(e.id))
      .filter((e): e is HeldEvent => e !== null);
  }

  /** Bytes stored by this store, for the storage ceiling. */
  storedBytes(): number {
    const rows = this.sql.exec(
      `SELECT
         (SELECT COALESCE(SUM(LENGTH(bytes)), 0) FROM bridge_events) AS events,
         (SELECT COALESCE(SUM(LENGTH(bytes)), 0) FROM bridge_waiting) AS waiting,
         (SELECT COALESCE(SUM(LENGTH(bytes)), 0) FROM bridge_proof_headers) AS proofs`,
    ).toArray();
    return Number(rows[0].events) + Number(rows[0].waiting) + Number(rows[0].proofs);
  }

  /** The canonical shape tests compare across stores and against
   * replay: chains, the applied event ids per author, the waiting
   * ids, and the fork proofs. A pure function of what arrived. */
  canonical(): {
    chains: Record<string, ChainState>;
    applied: Record<string, string[]>;
    waiting: string[];
    proofs: string[];
  } {
    const chains: Record<string, ChainState> = {};
    const applied: Record<string, string[]> = {};
    for (const author of this.knownAuthors()) {
      const chain = this.chainOf(author);
      chains[author] = chain;
      const ids: string[] = [];
      for (let seq = 0; seq < chain.next_seq; seq++) {
        const at = this.storedAt(author, seq);
        if (at) ids.push(at.id);
      }
      applied[author] = ids;
    }
    const waiting = this.sql.exec("SELECT id FROM bridge_waiting ORDER BY id").toArray().map((r) => r.id as string);
    const proofs = this.sql.exec(
      "SELECT author, seq, first_id, second_id FROM bridge_forks ORDER BY author, seq",
    ).toArray().map((r) => `${r.author}:${Number(r.seq)}:${r.first_id}:${r.second_id}`);
    return { chains, applied, waiting, proofs };
  }

  // ----- ingest ----------------------------------------------------

  /** One batch, decided in order (the native ingest_batch). Entries
   * are verified events, or a decode refusal recorded in place. */
  ingestBatch(entries: (SignedEvent | { malformed: string })[], nowMs: number): IngestOutcome[] {
    this.expire(nowMs);
    const outcomes: IngestOutcome[] = [];
    for (const entry of entries) {
      if ("malformed" in entry) {
        outcomes.push({ kind: "refused", reason: "malformed", message: entry.malformed });
        continue;
      }
      outcomes.push(this.ingestOne(entry, nowMs));
    }
    return outcomes;
  }

  private ingestOne(event: SignedEvent, nowMs: number): IngestOutcome {
    const decision = this.decide(event);
    switch (decision.t) {
      case "duplicate":
        return { kind: "duplicate", id: event.id };
      case "refuse":
        return { kind: "refused", reason: decision.reason, message: decision.message };
      case "fork": {
        this.storeFork(event.authorHex, event.seq, decision.first, decision.second);
        this.refreshAuthor(event.authorHex);
        const released: string[] = [];
        this.drain(nowMs, released);
        return { kind: "fork", author: event.authorHex, seq: event.seq };
      }
      case "store": {
        this.storeEvent(event);
        this.markPresent(event.id);
        this.refreshAuthor(event.authorHex);
        const released: string[] = [];
        this.drain(nowMs, released);
        return { kind: "accepted", id: event.id, released };
      }
      case "wait":
      case "park": {
        const held = this.hold(
          event,
          decision.t === "wait" ? decision.deps : [],
          decision.t === "park",
          nowMs,
        );
        if (held === "full") {
          return {
            kind: "refused",
            reason: "waiting_full",
            message: "the waiting set is full",
          };
        }
        return { kind: "waiting", id: event.id };
      }
    }
  }

  /** The native decision procedure (ingest.rs `decide_in`), step for
   * step. Reads only; the caller applies the decision. */
  private decide(event: SignedEvent): Decision {
    const author = event.authorHex;
    // (1) Already stored, or itself a fork proof header: a duplicate.
    if (this.isStored(event.id) || this.isProofHeader(author, event.id)) {
      return { t: "duplicate" };
    }
    const proof = this.lowestProof(author);
    // (2) Conflicts at this position: a stored event, or a held rival.
    const conflicts: { id: string; bytes: Uint8Array }[] = [];
    const stored = this.storedAt(author, event.seq);
    if (stored && stored.id !== event.id) conflicts.push(stored);
    for (const rival of this.heldRivals(author, event.seq, event.id)) {
      conflicts.push({ id: rival.id, bytes: rival.bytes });
    }
    if (proof) {
      if (event.seq > proof.seq) {
        return {
          t: "refuse",
          reason: "frozen",
          message: `the author's chain is frozen at sequence ${proof.seq}`,
        };
      }
      if (event.seq === proof.seq) {
        const pair = twoLowest([
          { id: proof.first_id, bytes: this.bytesOf(proof.first_id) },
          { id: proof.second_id, bytes: this.bytesOf(proof.second_id) },
          { id: event.id, bytes: event.bytes },
        ]);
        if (pair.first.id === proof.first_id && pair.second.id === proof.second_id) {
          return {
            t: "refuse",
            reason: "frozen",
            message: `the author's chain is frozen at sequence ${proof.seq}`,
          };
        }
        return { t: "fork", first: pair.first, second: pair.second };
      }
    }
    // (3) A conflict at an unfrozen position: the two lowest IDs are
    // the fork proof; the rest are refused by the freeze from now on.
    if (conflicts.length > 0) {
      const pair = twoLowest([...conflicts, { id: event.id, bytes: event.bytes }]);
      return { t: "fork", first: pair.first, second: pair.second };
    }
    // (4) Dependencies: `after` hashes that are not present.
    const waits: string[] = [];
    for (const dep of event.afterHex) {
      if (!this.isPresent(dep)) waits.push(dep);
    }
    // (5) A session checkpoint depends on its predecessor, which
    // must be the previous segment of the same session.
    if (event.checkpoint && event.checkpoint.prev) {
      const prevHex = bytesToHex(event.checkpoint.prev);
      const storedPrev = this.storedCheckpoint(prevHex);
      if (storedPrev) {
        const sameSession =
          storedPrev.header !== null &&
          bytesToHex(storedPrev.header) === bytesToHex(event.checkpoint.headerEncoding);
        const previousSegment =
          storedPrev.seq !== null &&
          BigInt(storedPrev.seq) + 1n === event.checkpoint.segmentSeq;
        if (!sameSession || !previousSegment) {
          return {
            t: "refuse",
            reason: "malformed",
            message: "the previous checkpoint is not the previous segment of this session",
          };
        }
      } else if (this.isPresent(prevHex)) {
        return { t: "park" };
      } else {
        waits.push(prevHex);
      }
    }
    // (6) The backlink must name the stored event just below.
    if (event.seq > 0) {
      const below = this.storedAt(author, event.seq - 1);
      const linked = below !== null && below.id === event.backlinkHex;
      if (!linked) {
        if (event.backlinkHex && !this.isPresent(event.backlinkHex)) {
          waits.push(event.backlinkHex);
        } else {
          // Present somewhere else, or itself a proof header: the
          // chain can never link through it. Park.
          return { t: "park" };
        }
      }
    }
    if (waits.length > 0) return { t: "wait", deps: waits };
    return { t: "store" };
  }

  /** Re-decides held events whose dependencies are all present, in
   * arrival order, until none are ready — the native release drain.
   * Applied ids are collected in application order. */
  private drain(nowMs: number, released: string[]): void {
    for (;;) {
      const rows = this.sql.exec(
        `SELECT * FROM bridge_waiting WHERE parked = 0 AND deps = '[]'
         ORDER BY arrival ASC LIMIT 1`,
      ).toArray();
      if (rows.length === 0) return;
      const row = this.waitingRow(rows[0]);
      this.deleteWaiting(row.id);
      // The bytes were signature-verified when they first arrived.
      const event = parseEvent(row.bytes);
      const decision = this.decide(event);
      switch (decision.t) {
        case "duplicate":
        case "refuse":
          // A proof header is never dropped: it parks, so later
          // arrivals at its position still conflict with it.
          if (this.isProofHeader(row.author, row.id)) {
            this.insertWaiting(row, [], true);
          }
          break;
        case "fork":
          this.storeFork(row.author, row.seq, decision.first, decision.second);
          this.refreshAuthor(row.author);
          break;
        case "store": {
          this.storeEvent(event);
          this.markPresent(row.id);
          const chain = this.refreshAuthor(row.author);
          if (row.seq < chain.next_seq) released.push(row.id);
          break;
        }
        case "wait":
          this.insertWaiting(row, decision.deps, false);
          break;
        case "park":
          this.insertWaiting(row, [], true);
          break;
      }
    }
  }

  // ----- writes ----------------------------------------------------

  private storeEvent(event: SignedEvent): void {
    this.sql.exec(
      `INSERT INTO bridge_events (author, seq, id, bytes, checkpoint_header, checkpoint_seq)
       VALUES (?, ?, ?, ?, ?, ?)`,
      event.authorHex,
      event.seq,
      event.id,
      event.bytes,
      event.checkpoint ? event.checkpoint.headerEncoding : null,
      event.checkpoint ? event.checkpoint.segmentSeq.toString() : null,
    );
  }

  private storeFork(
    author: string,
    seq: number,
    first: { id: string; bytes: Uint8Array },
    second: { id: string; bytes: Uint8Array },
  ): void {
    this.sql.exec(
      `INSERT OR REPLACE INTO bridge_forks (author, seq, first_id, second_id)
       VALUES (?, ?, ?, ?)`,
      author,
      seq,
      first.id,
      second.id,
    );
    for (const proofEvent of [first, second]) {
      this.sql.exec(
        "INSERT OR IGNORE INTO bridge_proof_ids (id, author) VALUES (?, ?)",
        proofEvent.id,
        author,
      );
      this.sql.exec(
        "INSERT OR IGNORE INTO bridge_proof_headers (id, bytes) VALUES (?, ?)",
        proofEvent.id,
        proofEvent.bytes,
      );
      this.markPresent(proofEvent.id);
    }
  }

  private hold(
    event: SignedEvent,
    deps: string[],
    parked: boolean,
    nowMs: number,
  ): "held" | "already" | "full" {
    const existing = this.sql.exec(
      "SELECT 1 AS x FROM bridge_waiting WHERE id = ?",
      event.id,
    ).toArray();
    if (existing.length > 0) return "already";
    const byAuthor = this.sql.exec(
      "SELECT COUNT(*) AS n FROM bridge_waiting WHERE author = ?",
      event.authorHex,
    ).toArray();
    if (Number(byAuthor[0].n) >= this.limits.perAuthor) return "full";
    const total = this.sql.exec("SELECT COUNT(*) AS n FROM bridge_waiting").toArray();
    if (Number(total[0].n) >= this.limits.total) return "full";
    const row: WaitingRow = {
      id: event.id,
      author: event.authorHex,
      seq: event.seq,
      bytes: event.bytes,
      arrived_at: nowMs,
      arrival: this.nextArrival(),
      deps,
      parked,
      checkpoint_header: event.checkpoint ? event.checkpoint.headerEncoding : null,
      checkpoint_seq: event.checkpoint ? event.checkpoint.segmentSeq.toString() : null,
    };
    this.insertWaiting(row, deps, parked);
    return "held";
  }

  private insertWaiting(row: WaitingRow, deps: string[], parked: boolean): void {
    this.sql.exec(
      `INSERT OR REPLACE INTO bridge_waiting
         (id, author, seq, bytes, arrived_at, arrival, deps, parked, checkpoint_header, checkpoint_seq)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      row.id,
      row.author,
      row.seq,
      row.bytes,
      row.arrived_at,
      row.arrival,
      JSON.stringify(deps),
      parked ? 1 : 0,
      row.checkpoint_header,
      row.checkpoint_seq,
    );
    this.sql.exec("DELETE FROM bridge_waiting_by_dep WHERE id = ?", row.id);
    for (const dep of deps) {
      this.sql.exec(
        "INSERT OR IGNORE INTO bridge_waiting_by_dep (dep, id) VALUES (?, ?)",
        dep,
        row.id,
      );
    }
  }

  private deleteWaiting(id: string): void {
    this.sql.exec("DELETE FROM bridge_waiting WHERE id = ?", id);
    this.sql.exec("DELETE FROM bridge_waiting_by_dep WHERE id = ?", id);
  }

  /** An id became present: it stops being a dependency anywhere. */
  private markPresent(id: string): void {
    const waiting = this.sql.exec(
      "SELECT id FROM bridge_waiting_by_dep WHERE dep = ?",
      id,
    ).toArray();
    this.sql.exec("DELETE FROM bridge_waiting_by_dep WHERE dep = ?", id);
    for (const w of waiting) {
      const rows = this.sql.exec(
        "SELECT deps FROM bridge_waiting WHERE id = ?",
        w.id as string,
      ).toArray();
      if (rows.length === 0) continue;
      const deps = (JSON.parse(rows[0].deps as string) as string[]).filter((d) => d !== id);
      this.sql.exec(
        "UPDATE bridge_waiting SET deps = ? WHERE id = ?",
        JSON.stringify(deps),
        w.id as string,
      );
    }
  }

  /** The applied prefix is a pure function of stored events and the
   * lowest fork proof; recompute the author's chain row from them. */
  private refreshAuthor(author: string): ChainState {
    let run = 0;
    for (;;) {
      if (!this.storedAt(author, run)) break;
      run += 1;
    }
    const proof = this.lowestProof(author);
    const next = proof ? Math.min(run, proof.seq) : run;
    const head = next > 0 ? (this.storedAt(author, next - 1)?.id ?? null) : null;
    const chain: ChainState = { next_seq: next, head, frozen: proof !== null };
    this.sql.exec(
      `INSERT OR REPLACE INTO bridge_chains (author, next_seq, head, frozen)
       VALUES (?, ?, ?, ?)`,
      author,
      chain.next_seq,
      chain.head,
      chain.frozen ? 1 : 0,
    );
    return chain;
  }

  private expire(nowMs: number): void {
    const rows = this.sql.exec(
      "SELECT id FROM bridge_waiting WHERE arrived_at + ? <= ?",
      this.limits.maxAgeMs,
      nowMs,
    ).toArray();
    for (const r of rows) this.deleteWaiting(r.id as string);
  }

  // ----- helpers ---------------------------------------------------

  private lowestProof(
    author: string,
  ): { seq: number; first_id: string; second_id: string } | null {
    const rows = this.sql.exec(
      `SELECT seq, first_id, second_id FROM bridge_forks
       WHERE author = ? ORDER BY seq ASC LIMIT 1`,
      author,
    ).toArray();
    if (rows.length === 0) return null;
    return {
      seq: Number(rows[0].seq),
      first_id: rows[0].first_id as string,
      second_id: rows[0].second_id as string,
    };
  }

  private heldRivals(author: string, seq: number, excludeId: string): WaitingRow[] {
    const rows = this.sql.exec(
      "SELECT * FROM bridge_waiting WHERE author = ? AND seq = ? AND id != ?",
      author,
      seq,
      excludeId,
    ).toArray();
    return rows.map((r) => this.waitingRow(r));
  }

  private storedCheckpoint(id: string): { header: Uint8Array | null; seq: string | null } | null {
    const rows = this.sql.exec(
      "SELECT checkpoint_header, checkpoint_seq FROM bridge_events WHERE id = ?",
      id,
    ).toArray();
    if (rows.length === 0) return null;
    return {
      header: rows[0].checkpoint_header ? blobBytes(rows[0].checkpoint_header) : null,
      seq: (rows[0].checkpoint_seq as string | null) ?? null,
    };
  }

  private bytesOf(id: string): Uint8Array {
    const held = this.heldEvent(id);
    if (!held) throw new Error(`proof header bytes missing for ${id}`);
    return held.bytes;
  }

  private waitingRow(r: Record<string, unknown>): WaitingRow {
    return {
      id: r.id as string,
      author: r.author as string,
      seq: Number(r.seq),
      bytes: blobBytes(r.bytes),
      arrived_at: Number(r.arrived_at),
      arrival: Number(r.arrival),
      deps: JSON.parse(r.deps as string) as string[],
      parked: Number(r.parked) !== 0,
      checkpoint_header: r.checkpoint_header ? blobBytes(r.checkpoint_header) : null,
      checkpoint_seq: (r.checkpoint_seq as string | null) ?? null,
    };
  }

  private nextArrival(): number {
    const rows = this.sql.exec(
      "SELECT value FROM bridge_meta WHERE key = 'arrival'",
    ).toArray();
    const next = (rows.length > 0 ? Number(rows[0].value) : 0) + 1;
    this.sql.exec(
      "INSERT OR REPLACE INTO bridge_meta (key, value) VALUES ('arrival', ?)",
      String(next),
    );
    return next;
  }

  /** Nonce and resume-token storage for the authenticated legs. */
  metaGet(key: string): string | null {
    const rows = this.sql.exec("SELECT value FROM bridge_meta WHERE key = ?", key).toArray();
    return rows.length > 0 ? (rows[0].value as string) : null;
  }

  metaSet(key: string, value: string): void {
    this.sql.exec(
      "INSERT OR REPLACE INTO bridge_meta (key, value) VALUES (?, ?)",
      key,
      value,
    );
  }

  metaDelete(key: string): void {
    this.sql.exec("DELETE FROM bridge_meta WHERE key = ?", key);
  }
}
