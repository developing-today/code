// Per-tenant sync coordination persisted beside the tenant's event store.
// SyncCoord owns presence and resume watermarks only; event bytes and
// acceptance decisions remain with BridgeStore and its signed-event ingress.

import type { SqlStorageLike } from "./bridge-store.ts";

export interface SyncWatermark {
  author: string;
  next_seq: number;
  head: string | null;
}

export interface SyncCoordRow {
  tenant_id: string;
  peer_id: string;
  session_id: string;
  state: "active" | "idle";
  watermark: SyncWatermark[];
  started_ms: number;
  updated_ms: number;
}

export class SyncCoordConflict extends Error {
  constructor() {
    super("another live sync session owns this tenant/peer pair");
    this.name = "SyncCoordConflict";
  }
}

/** The tenant database schema for SyncCoord. No event content is stored here. */
export const SYNC_COORD_SCHEMA = [
  `CREATE TABLE IF NOT EXISTS sync_coord (
     tenant_id TEXT NOT NULL,
     peer_id TEXT NOT NULL,
     session_id TEXT NOT NULL,
     state TEXT NOT NULL CHECK (state IN ('active', 'idle')),
     watermark TEXT NOT NULL,
     started_ms INTEGER NOT NULL,
     updated_ms INTEGER NOT NULL,
     PRIMARY KEY (tenant_id, peer_id))`,
  `CREATE INDEX IF NOT EXISTS sync_coord_session ON sync_coord(session_id)`,
];

const LIVE_SESSION_TTL_MS = 24 * 60 * 60 * 1000;

/**
 * Serializes one live session per (tenant, peer), while retaining the last
 * agreed watermark after the session ends. The owning UserDO's SQLite is the
 * tenant database, so coordination and the watermark commit with tenant data.
 */
export class SyncCoord {
  constructor(
    private sql: SqlStorageLike,
    private tenantId: string,
    private now: () => number = Date.now,
  ) {}

  /** Claim or refresh a session after its peer has authenticated. */
  own(peerId: string, sessionId: string, watermark: SyncWatermark[]): SyncCoordRow {
    const now = this.now();
    const prior = this.get(peerId);
    if (
      prior &&
      prior.state === "active" &&
      prior.session_id !== sessionId &&
      now - prior.updated_ms < LIVE_SESSION_TTL_MS
    ) {
      throw new SyncCoordConflict();
    }
    const started = prior?.session_id === sessionId ? prior.started_ms : now;
    this.sql.exec(
      `INSERT INTO sync_coord (tenant_id, peer_id, session_id, state, watermark, started_ms, updated_ms)
       VALUES (?, ?, ?, 'active', ?, ?, ?)
       ON CONFLICT(tenant_id, peer_id) DO UPDATE SET
         session_id = excluded.session_id,
         state = 'active',
         watermark = excluded.watermark,
         started_ms = excluded.started_ms,
         updated_ms = excluded.updated_ms`,
      this.tenantId,
      peerId,
      sessionId,
      JSON.stringify(canonicalWatermark(watermark)),
      started,
      now,
    );
    return this.get(peerId)!;
  }

  /** Advance the durable resume point after an accepted sync message. */
  advance(peerId: string, sessionId: string, watermark: SyncWatermark[]): SyncCoordRow {
    const prior = this.get(peerId);
    if (!prior || prior.session_id !== sessionId || prior.state !== "active") {
      throw new SyncCoordConflict();
    }
    return this.own(peerId, sessionId, watermark);
  }

  /** End only the caller's session, preserving its last agreed watermark. */
  end(peerId: string, sessionId: string): void {
    this.sql.exec(
      `UPDATE sync_coord SET state = 'idle', updated_ms = ?
       WHERE tenant_id = ? AND peer_id = ? AND session_id = ? AND state = 'active'`,
      this.now(),
      this.tenantId,
      peerId,
      sessionId,
    );
  }

  get(peerId: string): SyncCoordRow | null {
    const rows = this.sql
      .exec(
        `SELECT tenant_id, peer_id, session_id, state, watermark, started_ms, updated_ms
         FROM sync_coord WHERE tenant_id = ? AND peer_id = ?`,
        this.tenantId,
        peerId,
      )
      .toArray();
    if (rows.length === 0) return null;
    const row = rows[0];
    const state = String(row.state);
    if (state !== "active" && state !== "idle") throw new Error("invalid SyncCoord state");
    const watermark = JSON.parse(String(row.watermark)) as unknown;
    if (!Array.isArray(watermark)) throw new Error("invalid SyncCoord watermark");
    return {
      tenant_id: String(row.tenant_id),
      peer_id: String(row.peer_id),
      session_id: String(row.session_id),
      state,
      watermark: canonicalWatermark(watermark as SyncWatermark[]),
      started_ms: Number(row.started_ms),
      updated_ms: Number(row.updated_ms),
    };
  }
}

function canonicalWatermark(watermark: SyncWatermark[]): SyncWatermark[] {
  const seen = new Set<string>();
  const result = watermark.map((head) => {
    if (
      typeof head.author !== "string" ||
      !Number.isSafeInteger(head.next_seq) ||
      head.next_seq < 0 ||
      (head.head !== null && typeof head.head !== "string")
    ) {
      throw new Error("invalid sync watermark");
    }
    if (seen.has(head.author)) throw new Error("duplicate author in sync watermark");
    seen.add(head.author);
    return { author: head.author, next_seq: head.next_seq, head: head.head };
  });
  return result.sort((a, b) => a.author.localeCompare(b.author));
}
