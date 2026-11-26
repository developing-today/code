// The durable source fence (#3404 slice 1; design
// doc/2026-11-01_jurisdiction_aware_user_store_design.md §7, "Write
// fence").
//
// A migration freezes the source user object before export. The fence
// lives in the object's own storage, so it holds against every path
// that reaches the object — a stale route, a live WebSocket, a direct
// stub — not only against requests that read the directory first. The
// object checks it at the commit point of every mutation; the
// directory's pending state alone is never the fence.
//
// The fence has two states:
//
//   frozen ── unfreeze (abort before switch, at a NEWER epoch) ──► none
//      │
//      └── seal (after the directory switched to the target) ──► sealed
//
// A sealed fence is permanent for its migration: after the switch the
// target may have accepted writes, so the source never unfreezes
// (recover forward, or run a new reverse migration).
//
// Every transition names its migration id and routing epoch. The object
// keeps the highest epoch it has seen (the floor), so a freeze replayed
// from a migration that was already aborted cannot re-freeze it, and a
// retried call with the same id and epoch is answered, not refused.
//
// Only the migration principal may call these. The principal is an
// internal header the front door strips from every client request, the
// same trust posture as the routing context.

import { FailClosed } from "./routing.ts";

/** Set only by the migration coordinator on its stub calls; the front
 * door deletes any client-supplied copy. */
export const HDR_MIGRATION = "x-hosted-migration-principal";

/** Retry-After, in seconds, on a mutation the fence refused. */
export const FENCE_RETRY_AFTER_SECONDS = 30;

export type FenceState = "frozen" | "sealed";

export interface SourceFence {
  v: 1;
  migration_id: string;
  routing_epoch: number;
  state: FenceState;
  frozen_at_ms: number;
  /** The object's mutation count when the fence was set: export reads
   * at this watermark, and it can never move while the fence stands. */
  watermark: number;
}

export interface FenceRequest {
  migrationId: string;
  routingEpoch: number;
}

const MIGRATION_ID = /^[A-Za-z0-9._:-]{1,128}$/;

/** Validate a control-route body: `{migration_id, routing_epoch}`. */
export function parseFenceRequest(body: unknown): FenceRequest {
  const value = (typeof body === "object" && body !== null ? body : {}) as {
    migration_id?: unknown;
    routing_epoch?: unknown;
  };
  if (typeof value.migration_id !== "string" || !MIGRATION_ID.test(value.migration_id)) {
    throw new FailClosed(400, "invalid_migration_id", "migration_id must be 1 to 128 of A-Z a-z 0-9 . _ : -");
  }
  const epoch = value.routing_epoch;
  if (typeof epoch !== "number" || !Number.isSafeInteger(epoch) || epoch < 1) {
    throw new FailClosed(400, "invalid_routing_epoch", "routing_epoch must be a positive safe integer");
  }
  return { migrationId: value.migration_id, routingEpoch: epoch };
}

/** Read the persisted fence. A row that does not parse is corruption:
 * it fails closed, so no mutation commits past a fence it cannot read. */
export function readFence(raw: string | null): SourceFence | null {
  if (raw === null) return null;
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    throw fenceCorrupt();
  }
  const f = parsed as Partial<SourceFence>;
  if (
    f === null ||
    typeof f !== "object" ||
    f.v !== 1 ||
    typeof f.migration_id !== "string" ||
    !MIGRATION_ID.test(f.migration_id) ||
    typeof f.routing_epoch !== "number" ||
    !Number.isSafeInteger(f.routing_epoch) ||
    f.routing_epoch < 1 ||
    (f.state !== "frozen" && f.state !== "sealed") ||
    typeof f.frozen_at_ms !== "number" ||
    typeof f.watermark !== "number" ||
    !Number.isSafeInteger(f.watermark) ||
    f.watermark < 0
  ) {
    throw fenceCorrupt();
  }
  return f as SourceFence;
}

function fenceCorrupt(): FailClosed {
  return new FailClosed(500, "fence_corrupt", "the migration fence record is unreadable; failing closed");
}

/** Read the epoch floor: the highest routing epoch any transition named. */
export function readEpochFloor(raw: string | null): number {
  if (raw === null) return 0;
  const n = Number(raw);
  if (!Number.isSafeInteger(n) || n < 0) throw fenceCorrupt();
  return n;
}

/** The fence a freeze leaves, or the refusal. `changed` is false for a
 * retried freeze of the same migration at the same epoch. */
export function planFreeze(
  current: SourceFence | null,
  floor: number,
  request: FenceRequest,
  watermark: number,
  nowMs: number,
): { fence: SourceFence; changed: boolean } {
  if (current !== null) {
    if (current.migration_id !== request.migrationId) {
      throw new FailClosed(409, "fence_held", "another migration holds this store's fence");
    }
    if (current.state === "sealed") {
      throw new FailClosed(409, "fence_sealed", "this migration already switched; the source stays sealed");
    }
    if (current.routing_epoch !== request.routingEpoch) {
      throw new FailClosed(409, "stale_epoch", "the freeze names a different routing epoch than the fence holds");
    }
    return { fence: current, changed: false };
  }
  if (request.routingEpoch <= floor) {
    throw new FailClosed(409, "stale_epoch", "the routing epoch is not newer than one this store already saw");
  }
  return {
    fence: {
      v: 1,
      migration_id: request.migrationId,
      routing_epoch: request.routingEpoch,
      state: "frozen",
      frozen_at_ms: nowMs,
      watermark,
    },
    changed: true,
  };
}

/** Check an unfreeze (abort before switch). It must name the fence's
 * migration and a routing epoch newer than the fence's: the directory
 * moved back to the source at a fresh epoch before the source reopens.
 * Returns the new epoch floor. */
export function planUnfreeze(current: SourceFence | null, request: FenceRequest): number {
  if (current === null) throw new FailClosed(409, "not_fenced", "this store holds no migration fence");
  if (current.migration_id !== request.migrationId) {
    throw new FailClosed(409, "fence_held", "another migration holds this store's fence");
  }
  if (current.state === "sealed") {
    throw new FailClosed(409, "fence_sealed", "this migration already switched; the source never unfreezes");
  }
  if (request.routingEpoch <= current.routing_epoch) {
    throw new FailClosed(409, "stale_epoch", "an unfreeze needs a routing epoch newer than the fence's");
  }
  return request.routingEpoch;
}

/** The sealed fence after the switch. Sealing is idempotent at the same
 * epoch and refuses an epoch older than the fence's. */
export function planSeal(
  current: SourceFence | null,
  request: FenceRequest,
): { fence: SourceFence; changed: boolean } {
  if (current === null) throw new FailClosed(409, "not_fenced", "only a frozen store can be sealed");
  if (current.migration_id !== request.migrationId) {
    throw new FailClosed(409, "fence_held", "another migration holds this store's fence");
  }
  if (current.state === "sealed") {
    if (current.routing_epoch !== request.routingEpoch) {
      throw new FailClosed(409, "stale_epoch", "the seal names a different routing epoch than the fence holds");
    }
    return { fence: current, changed: false };
  }
  if (request.routingEpoch < current.routing_epoch) {
    throw new FailClosed(409, "stale_epoch", "a seal cannot name a routing epoch older than the fence's");
  }
  return { fence: { ...current, state: "sealed", routing_epoch: request.routingEpoch }, changed: true };
}
