// MOB-09 — desired/observed reconciliation, freshness and rollback
// (#9476; design record
// `doc/2026-11-25T22-32-18Z_design_mob09_reconciliation_freshness_rollback/`).
//
// Why this module exists: MOB-05's `applyWorkAction`
// (src/work-action-contract.ts) moves `running` to whatever the
// downstream reports. A Project writer that answers "done" is an
// accepted write, not an applied one, and nothing on main moved an
// envelope out of `pending_sync` or `partial`. This module makes
// "verified" mean "read back from the source of record and seen":
//
//   1. `reconcileWorkAction` classifies each desired effect against
//      separate readbacks and moves the envelope only along MOB-05's
//      declared edges;
//   2. `applyIfMutationsEnabled` is the rollback switch: off means no
//      effect and no transition, so an accepted envelope stays
//      accepted and can be applied later.
//
// Boundaries: pure, no I/O, no clock (the caller passes `now`), no
// store. Readback results are values; persisting them belongs to the
// aggregator action store's owner (CEN-04 §3.14).

import {
  applyWorkAction,
  canTransition,
  type WorkActionActor,
  type WorkActionAuthority,
  type WorkActionEnvelope,
  type WorkActionState,
} from "./work-action-contract.ts";

/** MOB-01 D4 revision kinds. `observed_at` is the weakest and never verifies. */
export const REVISION_KINDS = ["work_version", "updated_at", "etag", "item_revision", "observed_at"] as const;
export type RevisionKind = (typeof REVISION_KINDS)[number];

export interface Revision {
  kind: string;
  value: string;
}

/** One effect the action wanted on a source of record. */
export interface DesiredEffect {
  effectId: string;
  objectRef: string;
  /** The source of record for this field (MOB-01 `SourceKind`), e.g. "project". */
  source: string;
  field: string;
  desired: unknown;
  /** The value the preview read, before the write. */
  priorValue: unknown;
  /** The revision the preview read, before the write; null when none was readable. */
  priorRevision: Revision | null;
  /** When the effect was handed to the writer (ISO-8601). */
  issuedAt: string;
}

/** A separate read of the source of record. */
export interface Readback {
  readId: string;
  effectId: string;
  objectRef: string;
  /** How many objects the read matched. Anything but 1 is ambiguous. */
  matchedObjects: number;
  source: string;
  field: string;
  value: unknown;
  revision: Revision;
  observedAt: string;
  freshness: "live" | "stale" | "fixtures" | "error";
  /** Set by a caller that is about to pass the write's own response. Refused. */
  fromWriteResponse?: boolean;
}

export type EffectResult = "verified" | "not_yet" | "drift" | "unread" | "ambiguous" | "unknown_version";

/** The per-effect readback/drift receipt the slice's goal names. */
export interface EffectReceipt {
  operationKey: string;
  effectId: string;
  field: string;
  desired: unknown;
  observed: unknown;
  observedRevision: Revision | null;
  observedAt: string | null;
  readId: string | null;
  result: EffectResult;
}

export interface ReconcileOptions {
  now: string;
  /** Per-source sync deadline in milliseconds. A source missing here never passes its deadline. */
  syncDeadlineMs: Readonly<Record<string, number>>;
}

export interface ReconcileResult {
  envelope: WorkActionEnvelope;
  effects: EffectReceipt[];
  /** What the display shows. Differs from `envelope.state` only for an unbacked verified envelope. */
  displayState: WorkActionState;
  /** True when the envelope is `verified_complete` but no qualifying readback backs it. */
  unbackedVerified: boolean;
  /** Why the envelope moved or why it stayed, for the display. */
  reason: string | null;
}

const RECONCILABLE: ReadonlySet<WorkActionState> = new Set(["running", "pending_sync", "partial", "verified_complete"]);

function isKnownKind(kind: string): kind is RevisionKind {
  return (REVISION_KINDS as readonly string[]).includes(kind);
}

function sameValue(a: unknown, b: unknown): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

/** True when `next` is provably after `prior`. Ordered kinds compare; opaque kinds must differ. */
function revisionAdvanced(prior: Revision | null, next: Revision): boolean {
  if (prior === null) return true;
  if (prior.kind !== next.kind) return false;
  switch (next.kind) {
    case "updated_at":
      return Date.parse(next.value) > Date.parse(prior.value);
    case "work_version": {
      const a = /^\d+$/.test(prior.value) ? BigInt(prior.value) : null;
      const b = /^\d+$/.test(next.value) ? BigInt(next.value) : null;
      return a !== null && b !== null ? b > a : next.value !== prior.value;
    }
    default:
      return next.value !== prior.value;
  }
}

function classify(effect: DesiredEffect, reads: readonly Readback[]): { result: EffectResult; read: Readback | null } {
  for (const read of reads) {
    if (read.fromWriteResponse) {
      throw new Error(
        `work-operation-reconciliation: readback ${read.readId} is the write's own response; a write response is never a readback`,
      );
    }
  }
  const relevant = reads.filter((r) => r.effectId === effect.effectId);
  if (relevant.length === 0) return { result: "unread", read: null };
  // An ambiguous or mis-targeted read poisons the effect: it is never verified from it.
  const wrong = relevant.find(
    (r) =>
      r.matchedObjects !== 1 ||
      r.objectRef !== effect.objectRef ||
      r.source !== effect.source ||
      r.field !== effect.field,
  );
  if (wrong) return { result: "ambiguous", read: wrong };
  const latest = [...relevant].sort((a, b) => Date.parse(b.observedAt) - Date.parse(a.observedAt))[0] as Readback;
  if (!isKnownKind(latest.revision.kind)) return { result: "unknown_version", read: latest };
  if (latest.freshness !== "live") return { result: "unread", read: latest };
  if (latest.revision.kind === "observed_at") return { result: "unread", read: latest };
  if (Date.parse(latest.observedAt) < Date.parse(effect.issuedAt)) return { result: "unread", read: latest };
  if (!revisionAdvanced(effect.priorRevision, latest.revision)) return { result: "not_yet", read: latest };
  if (sameValue(latest.value, effect.desired)) return { result: "verified", read: latest };
  if (sameValue(latest.value, effect.priorValue)) return { result: "not_yet", read: latest };
  return { result: "drift", read: latest };
}

function deadlinePassed(effect: DesiredEffect, opts: ReconcileOptions): boolean {
  const ms = opts.syncDeadlineMs[effect.source];
  if (ms === undefined) return false;
  return Date.parse(opts.now) - Date.parse(effect.issuedAt) > ms;
}

function move(
  envelope: WorkActionEnvelope,
  to: WorkActionState,
  reason: string,
  workVersion: string | null,
): WorkActionEnvelope {
  if (to === envelope.state) return envelope;
  if (!canTransition(envelope.state, to)) {
    throw new Error(`work-operation-reconciliation: undeclared edge ${envelope.state} → ${to}`);
  }
  return {
    ...envelope,
    state: to,
    receipt: {
      operationKey: envelope.request.operationKey,
      outcome: to as "verified_complete" | "partial" | "pending_sync" | "conflict" | "uncertain",
      workVersion,
      note: reason,
    },
    transitions: [...envelope.transitions, { from: envelope.state, to, oracle: `work-reconcile/${reason}` }],
  };
}

/**
 * One reconciliation pass. Moves `running`, `pending_sync` or `partial`
 * only along MOB-05's declared edges, and re-checks `verified_complete`
 * without moving it (it is terminal): an unbacked one is flagged and
 * displayed as pending sync.
 */
export function reconcileWorkAction(
  envelope: WorkActionEnvelope,
  desired: readonly DesiredEffect[],
  readbacks: readonly Readback[],
  opts: ReconcileOptions,
): ReconcileResult {
  if (!RECONCILABLE.has(envelope.state)) {
    throw new Error(`work-operation-reconciliation: cannot reconcile an envelope in ${envelope.state}`);
  }
  if (desired.length === 0) throw new Error("work-operation-reconciliation: at least one desired effect is required");

  const effects: EffectReceipt[] = [];
  const results: EffectResult[] = [];
  let newestRevision: string | null = null;
  for (const effect of desired) {
    const { result, read } = classify(effect, readbacks);
    results.push(result);
    if (read && result === "verified") newestRevision = read.revision.value;
    effects.push({
      operationKey: envelope.request.operationKey,
      effectId: effect.effectId,
      field: effect.field,
      desired: effect.desired,
      observed: read ? read.value : null,
      observedRevision: read ? read.revision : null,
      observedAt: read ? read.observedAt : null,
      readId: read ? read.readId : null,
      result,
    });
  }

  const all = (r: EffectResult) => results.every((x) => x === r);
  const any = (r: EffectResult) => results.includes(r);
  const waiting = results.map((r, i) => (r === "not_yet" || r === "unread" ? i : -1)).filter((i) => i >= 0);
  const pastDeadline = waiting.some((i) => deadlinePassed(desired[i] as DesiredEffect, opts));

  if (envelope.state === "verified_complete") {
    const backed = all("verified");
    return {
      envelope,
      effects,
      displayState: backed ? "verified_complete" : "pending_sync",
      unbackedVerified: !backed,
      reason: backed ? null : "unbacked_verified",
    };
  }

  const from = envelope.state;
  let to: WorkActionState = from;
  let reason: string | null = null;
  if (all("verified")) {
    to = "verified_complete";
    reason = "readback_verified";
  } else if (any("drift")) {
    // pending_sync → conflict is not a declared MOB-05 edge.
    to = from === "partial" ? "conflict" : "uncertain";
    reason = "drift";
  } else if (from === "partial") {
    to = "partial";
    reason = pastDeadline ? "sync_deadline_passed" : null;
  } else if (any("ambiguous")) {
    to = "uncertain";
    reason = "ambiguous_resource";
  } else if (any("unknown_version")) {
    to = "uncertain";
    reason = "unknown_version";
  } else if (any("verified")) {
    to = "partial";
    reason = "partially_verified";
  } else if (pastDeadline) {
    to = "uncertain";
    reason = "sync_deadline_passed";
  } else {
    to = "pending_sync";
    reason = from === "pending_sync" ? null : "awaiting_readback";
  }

  // `running → pending_sync` first when the target is only reachable from there.
  let next = envelope;
  if (from === "running" && !canTransition("running", to)) {
    next = move(next, "pending_sync", "awaiting_readback", envelope.previewedVersion);
  }
  next = move(next, to, reason ?? "awaiting_readback", newestRevision ?? next.receipt?.workVersion ?? null);
  return { envelope: next, effects, displayState: next.state, unbackedVerified: false, reason };
}

export type ApplyGateResult =
  | { applied: true; envelope: WorkActionEnvelope }
  | { applied: false; refused: "mutations_disabled"; envelope: WorkActionEnvelope };

/**
 * The rollback switch. Off: no effect, no transition, no authority call —
 * the accepted envelope comes back unchanged so it stays inspectable and
 * can be applied once mutations are on again. On: MOB-05's apply, with
 * every one of its gates.
 */
export function applyIfMutationsEnabled(
  mutationsEnabled: boolean,
  envelope: WorkActionEnvelope,
  actor: WorkActionActor,
  authority: WorkActionAuthority,
  report: Parameters<typeof applyWorkAction>[3],
): ApplyGateResult {
  if (!mutationsEnabled) return { applied: false, refused: "mutations_disabled", envelope };
  return { applied: true, envelope: applyWorkAction(envelope, actor, authority, report) };
}
