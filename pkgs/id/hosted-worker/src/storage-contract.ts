// The storage contract, budget ladder and backend manifest for the
// hosted store (#15483; gap 5 of #9080, component B of #9078).
//
// This module is the ONE place on the hosted stack where these
// values and shapes are defined. Sibling slices (the request cap
// #15485, the tenant data split #15482 and its rehome slice, the
// Turso adapter #12801) import from here; they do not restate a
// refusal kind, a threshold, a capability flag or a manifest field.
//
// Oracles, each consumed and not re-decided:
//   - #5863 storage contract   doc/2026-11-17_decision_id5863_storage_contract.md
//       §2.3 read at watermark, §2.4 head, §3.2 snapshot + delta,
//       §4.1 declared limits, §4.2 capability flags,
//       §5.1 typed refusals, §5.2 meters, §5.3 version stamp.
//   - #5868 guardrails          doc/2026-11-08_decision_id5868_cost_limits_guardrails.md
//       decision 3 (four-step ladder, 50/75/90 %, 10-point
//       hysteresis), decision 4 (fixed degrade order, journal
//       appends never shed), decision 5 (typed refusal naming
//       store, backend, dimension, threshold, operator action).
//   - #5867 backend manifest    doc/2026-11-12_decision_id5867_backend_migration.md
//       §2.2 the manifest names backend, generation and cutover
//       sequence, §2.3 a write carries the resolved generation,
//       §2.4 missing/invalid/conflicting placement fails closed as
//       a typed unavailable/placement-unknown result.
//
// Everything here is pure: no IO, no clock, no storage. The Durable
// Object (user-do.ts) is the caller that reads usage and meta and
// applies these decisions. Only erasable TypeScript syntax is used
// (no enums, namespaces or parameter properties) so the tests run
// under `node --experimental-transform-types`.

// ----- versions ------------------------------------------------------

/** #5863 §5.3: contract version 1 is the shape of §2–§5. */
export const CONTRACT_VERSION = 1;

/** Version of the backend manifest record defined below. */
export const MANIFEST_VERSION = 1;

// ----- wire names on the hosted stack ---------------------------------

/** Request header: read at (at least) this journal watermark (§2.3). */
export const HDR_READ_AT_WATERMARK = "x-read-at-watermark";
/** Response header: the watermark a response was served at, or the
 * one a write committed at (read-your-writes, §2.3). */
export const HDR_STORAGE_WATERMARK = "x-storage-watermark";
/** Request header: the placement generation a writer resolved
 * (#5867 §2.3). Optional; when present it must be current. */
export const HDR_PLACEMENT_GENERATION = "x-placement-generation";

// ----- backends and capability flags (#5863 §4.2) ---------------------

/** The stores a hosted unit can be materialised in. `do_sqlite` is
 * the per-user Durable Object's SQLite (where every hosted store
 * lives today); `d1` and `turso` are the two #5681 backends. */
export const BACKENDS = ["do_sqlite", "d1", "turso"] as const;
export type BackendId = (typeof BACKENDS)[number];

export interface CapabilityFlags {
  /** The store accepts journal appends from a disconnected endpoint. */
  offline_append: boolean;
  /** Read replicas co-located with the reader. */
  embedded_replica: boolean;
  /** Reachable from a Worker with no extra vendor credentials. */
  workers_native: boolean;
}

/** #5863 §4.2 flag values. D1 and Turso are as the record declares
 * them; the Durable Object store is online-only, has no embedded
 * replica and is a Worker binding. Flags inform placement only: no
 * code may branch on one to change semantics (§4.2, last bullet). */
export const BACKEND_CAPABILITIES: Readonly<Record<BackendId, Readonly<CapabilityFlags>>> = {
  do_sqlite: { offline_append: false, embedded_replica: false, workers_native: true },
  d1: { offline_append: false, embedded_replica: false, workers_native: true },
  turso: { offline_append: false, embedded_replica: true, workers_native: false },
};

export function isBackendId(value: unknown): value is BackendId {
  return typeof value === "string" && (BACKENDS as readonly string[]).includes(value);
}

/** The flags a placement requires that `backend` lacks; empty means
 * the placement is allowed (#5863 §4.2: refuse a placement that
 * requires a flag the store lacks). */
export function missingCapabilities(backend: BackendId, required: Partial<CapabilityFlags>): (keyof CapabilityFlags)[] {
  const have = BACKEND_CAPABILITIES[backend];
  const missing: (keyof CapabilityFlags)[] = [];
  for (const flag of Object.keys(required) as (keyof CapabilityFlags)[]) {
    if (required[flag] === true && !have[flag]) missing.push(flag);
  }
  return missing;
}

// ----- declared limits and meters (#5863 §4.1, §5.2) ------------------

/** The limits a store declares. null = not declared by this store. */
export interface DeclaredLimits {
  max_stored_bytes: number | null;
  max_entry_bytes: number | null;
  max_appends_per_second: number | null;
  /** #5869 §2.2: journal online at least 30 days. */
  retention_floor_days: number | null;
}

export type DeclaredLimitName = "max_stored_bytes" | "max_entry_bytes" | "max_appends_per_second" | "retention_floor_days";

/** #5869 §2.2 retention floor, as #5863 §4.1 cites it. */
export const RETENTION_FLOOR_DAYS = 30;

/** #5863 §5.2 meters; null = this store does not meter that dimension. */
export interface Meters {
  rows_read: number | null;
  rows_written: number | null;
  stored_bytes: number;
}

/** The dimensions a #5868 budget is kept in (decision 1). */
export const BUDGET_DIMENSIONS = ["stored_bytes", "rows_read", "rows_written", "sync_bytes"] as const;
export type BudgetDimension = (typeof BUDGET_DIMENSIONS)[number];

// ----- typed refusals (#5863 §5.1) -------------------------------------

export const REFUSAL_KINDS = ["limit", "unavailable", "fork", "version", "verification"] as const;
export type RefusalKind = (typeof REFUSAL_KINDS)[number];

/** A declared limit refused the operation (#5863 §4.1), carrying the
 * #5868 decision 5 fields: store, backend, dimension, threshold,
 * and the operator action that clears it. */
export interface LimitRefusal {
  kind: "limit";
  /** The declared limit being enforced. */
  limit: DeclaredLimitName;
  dimension: BudgetDimension;
  /** The ladder step the store was at when it refused. */
  step: LadderStep;
  /** The operation class refused. */
  op: OpClass;
  used: number;
  ceiling: number;
  /** Units the refused operation would have added. */
  requested: number;
  /** The fraction of the ceiling whose crossing refused it: the
   * `refused` threshold for a ladder refusal, 1 for the hard limit. */
  threshold: number;
  store: string;
  backend: BackendId;
  action: string;
}

export type UnavailableReason =
  /** The store's watermark is below the one requested (#5863 §2.3). */
  | "behind_watermark"
  /** Placement data missing, invalid or conflicting (#5867 §2.4). */
  | "placement_unknown"
  /** The caller resolved an older placement generation (#5867 §2.3). */
  | "placement_stale"
  /** The store could not be reached. */
  | "store_unreachable";

export interface UnavailableRefusal {
  kind: "unavailable";
  reason: UnavailableReason;
  /** The store's current watermark, when it knows one. */
  watermark: number | null;
  /** The watermark or generation the caller asked for, if any. */
  requested: number | null;
  /** The current placement generation, for placement refusals. */
  generation: number | null;
  message: string;
}

export interface ForkEntry {
  id: string;
  /** The signed entry's bytes, hex, when the store has them. */
  bytes_hex: string | null;
}

/** #5863 §2.1: an append at an occupied sequence by a different
 * signed entry, carrying both entries; never an overwrite. */
export interface ForkRefusal {
  kind: "fork";
  author: string;
  seq: number;
  first: ForkEntry;
  second: ForkEntry;
}

export interface VersionRefusal {
  kind: "version";
  supported: number;
  received: number;
  /** newer: the store does not understand it. older: no migration
   * for that version pair is recorded (#5863 §5.3). */
  direction: "newer" | "older";
}

export type VerificationReason = "sequence_gap" | "sequence_misaligned" | "hash_chain" | "signature" | "malformed";

export interface VerificationRefusal {
  kind: "verification";
  reason: VerificationReason;
  detail: string;
}

export type StorageRefusal = LimitRefusal | UnavailableRefusal | ForkRefusal | VersionRefusal | VerificationRefusal;

export type Checked<T> = { ok: true; value: T } | { ok: false; refusal: StorageRefusal };

/** The HTTP status each refusal kind is answered with on the hosted
 * stack. `limit` keeps the 413 the storage ceiling has always used
 * (PR #10132), so existing clients see one storage-full class. */
export const REFUSAL_HTTP_STATUS: Readonly<Record<RefusalKind, number>> = {
  limit: 413,
  unavailable: 503,
  fork: 409,
  version: 409,
  verification: 422,
};

export function unavailable(
  reason: UnavailableReason,
  message: string,
  fields: { watermark?: number | null; requested?: number | null; generation?: number | null } = {},
): UnavailableRefusal {
  return {
    kind: "unavailable",
    reason,
    watermark: fields.watermark ?? null,
    requested: fields.requested ?? null,
    generation: fields.generation ?? null,
    message,
  };
}

export function forkRefusal(author: string, seq: number, first: ForkEntry, second: ForkEntry): ForkRefusal {
  return { kind: "fork", author, seq, first, second };
}

export function verificationRefusal(reason: VerificationReason, detail: string): VerificationRefusal {
  return { kind: "verification", reason, detail };
}

/** #5863 §5.3: refuse a version newer than this contract, and an
 * older one unless a migration for that pair is recorded. */
export function checkContractVersion(received: unknown, migratableFrom: readonly number[] = []): StorageRefusal | null {
  if (typeof received !== "number" || !Number.isInteger(received) || received < 1) {
    return verificationRefusal("malformed", "contract version is not a positive integer");
  }
  if (received === CONTRACT_VERSION) return null;
  if (received > CONTRACT_VERSION) {
    return { kind: "version", supported: CONTRACT_VERSION, received, direction: "newer" };
  }
  if (migratableFrom.includes(received)) return null;
  return { kind: "version", supported: CONTRACT_VERSION, received, direction: "older" };
}

// ----- read at watermark and head (#5863 §2.3, §2.4) -------------------

/** #5863 §2.4. On a single-writer store the served watermark equals
 * the head sequence; a replica reports a lower watermark. */
export interface StoreHead {
  seq: number;
  /** ISO time of the commit at `seq`; null before the first commit. */
  timestamp: string | null;
  watermark: number;
}

/** Parse a requested watermark: a non-negative safe integer, as a
 * string (a header value). null = none requested; undefined-shaped
 * garbage is reported as malformed rather than ignored. */
export function parseWatermark(raw: string | null | undefined): Checked<number | null> {
  if (raw === null || raw === undefined || raw === "") return { ok: true, value: null };
  if (!/^[0-9]{1,16}$/.test(raw)) {
    return { ok: false, refusal: verificationRefusal("malformed", "requested watermark is not a non-negative integer") };
  }
  const n = Number(raw);
  if (!Number.isSafeInteger(n)) {
    return { ok: false, refusal: verificationRefusal("malformed", "requested watermark is out of range") };
  }
  return { ok: true, value: n };
}

/** #5863 §2.3: serve a view that includes every entry up to the
 * requested sequence, or refuse carrying the current watermark.
 * Serving at a higher watermark than requested is the read-at-≥N
 * the contract allows; it never serves silently stale data. Returns
 * the watermark the read is served at. */
export function checkReadAtWatermark(requested: number | null, head: StoreHead): Checked<number> {
  if (requested !== null && requested > head.watermark) {
    return {
      ok: false,
      refusal: unavailable(
        "behind_watermark",
        `store watermark ${head.watermark} is below the requested ${requested}`,
        { watermark: head.watermark, requested },
      ),
    };
  }
  return { ok: true, value: head.watermark };
}

// ----- snapshot plus delta (#5863 §3.2) --------------------------------

export interface SnapshotRef {
  /** #5863 §5.3: the contract version that produced the snapshot. */
  contract_version: number;
  /** The journal sequence the snapshot is complete as of. */
  at_seq: number;
}

/** Check that a snapshot plus a delta reproduce the store through
 * `target` (#5863 §3.2): the delta must start right after the
 * snapshot and be contiguous; a gap or misaligned delta is a
 * verification refusal (never repaired silently, §3.3); a delta that
 * stops short of the target is `unavailable` at the watermark it
 * does reach. Returns the watermark the replay reaches (= target).
 * Entries after `target` are not applied. */
export function checkSnapshotDelta(
  snapshot: SnapshotRef,
  deltaSeqs: readonly number[],
  target: number,
): Checked<number> {
  const version = checkContractVersion(snapshot.contract_version);
  if (version !== null) return { ok: false, refusal: version };
  if (!Number.isInteger(snapshot.at_seq) || snapshot.at_seq < 0 || !Number.isInteger(target) || target < 0) {
    return { ok: false, refusal: verificationRefusal("malformed", "snapshot and target sequences must be non-negative integers") };
  }
  if (target < snapshot.at_seq) {
    return {
      ok: false,
      refusal: verificationRefusal("sequence_misaligned", `target ${target} is before the snapshot at ${snapshot.at_seq}`),
    };
  }
  let reached = snapshot.at_seq;
  for (const seq of deltaSeqs) {
    if (reached >= target) break;
    if (seq !== reached + 1) {
      return {
        ok: false,
        refusal: verificationRefusal(
          seq <= reached ? "sequence_misaligned" : "sequence_gap",
          `delta entry ${seq} does not follow sequence ${reached}`,
        ),
      };
    }
    reached = seq;
  }
  if (reached < target) {
    return {
      ok: false,
      refusal: unavailable("behind_watermark", `snapshot plus delta reach ${reached}, not ${target}`, {
        watermark: reached,
        requested: target,
      }),
    };
  }
  return { ok: true, value: target };
}

// ----- the budget ladder (#5868 decisions 3 and 4) ---------------------

export const LADDER_STEPS = ["normal", "warning", "degraded", "refused"] as const;
export type LadderStep = (typeof LADDER_STEPS)[number];

export function isLadderStep(value: unknown): value is LadderStep {
  return typeof value === "string" && (LADDER_STEPS as readonly string[]).includes(value);
}

export interface LadderThresholds {
  /** Fractions of the ceiling at which each step is entered. */
  warning: number;
  degraded: number;
  refused: number;
  /** A step is left only once usage is this far below its entry. */
  hysteresis: number;
}

/** #5868 decision 3 defaults: warn at 50 %, degraded at 75 %,
 * refuse non-journal work at 90 %, clear 10 points below entry. */
export const DEFAULT_LADDER_THRESHOLDS: Readonly<LadderThresholds> = {
  warning: 0.5,
  degraded: 0.75,
  refused: 0.9,
  hysteresis: 0.1,
};

/** Operation classes the ladder rules on. */
export const OP_CLASSES = [
  /** A signed journal append (#806); the hosted bridge push. */
  "journal_append",
  /** A non-journal write a client is waiting on (KV, blob PUT). */
  "write",
  /** A non-journal write that can be put off: backfill, index build,
   * compaction (#5868 decision 4 (c)). */
  "deferrable_write",
  /** A write that does not grow storage: a delete, or an overwrite
   * no larger than what it replaces. How a capped user recovers. */
  "reclaim",
  /** A point read. */
  "read",
  /** A non-journal bulk read (#5868 decision 4 (b)). */
  "bulk_read",
  /** Advisory or derived surfaces: summaries, analytics, dashboard
   * aggregates (#5868 decision 4 (a)). */
  "advisory_read",
  /** Export: the exit path (#5869), available at cap (#3811). */
  "export",
] as const;
export type OpClass = (typeof OP_CLASSES)[number];

/** #5868 decision 4: the fixed order work is shed in. Each step
 * sheds a prefix of this list; journal appends are last and are
 * never shed by the ladder. Classes not listed (reclaim, read,
 * export) are never shed by the ladder either. */
export const DEGRADE_ORDER: readonly OpClass[] = [
  "advisory_read",
  "bulk_read",
  "deferrable_write",
  "write",
  "journal_append",
];

/** How many classes of DEGRADE_ORDER each step sheds. */
const SHED_COUNT: Readonly<Record<LadderStep, number>> = {
  normal: 0,
  warning: 0,
  // (a) advisory, (b) bulk reads, (c) deferrable writes.
  degraded: 3,
  // "refuse non-journal work and reserve the remaining write budget
  // for signed journal appends".
  refused: 4,
};

/** Per-step policy: whether each operation class is allowed. */
export const LADDER_POLICY: Readonly<Record<LadderStep, Readonly<Record<OpClass, boolean>>>> = (() => {
  const policy = {} as Record<LadderStep, Record<OpClass, boolean>>;
  for (const step of LADDER_STEPS) {
    const shed = new Set(DEGRADE_ORDER.slice(0, SHED_COUNT[step]));
    const row = {} as Record<OpClass, boolean>;
    for (const op of OP_CLASSES) row[op] = !shed.has(op);
    policy[step] = row;
  }
  return policy;
})();

export function opAllowed(step: LadderStep, op: OpClass): boolean {
  return LADDER_POLICY[step][op];
}

/** Validate thresholds: increasing, within (0, 1], hysteresis ≥ 0. */
export function validThresholds(t: LadderThresholds): boolean {
  const fracs = [t.warning, t.degraded, t.refused];
  return (
    fracs.every((f) => Number.isFinite(f) && f > 0 && f <= 1) &&
    t.warning < t.degraded &&
    t.degraded < t.refused &&
    Number.isFinite(t.hysteresis) &&
    t.hysteresis >= 0
  );
}

/** The ladder step for `used` of `ceiling` (#5868 decision 3).
 * `previous` is the step the store was last in: a step it was
 * already in (or above) is held until usage drops `hysteresis`
 * below that step's entry, so usage near a threshold does not flap.
 * A null ceiling means no budget is configured: always normal. A
 * zero ceiling is fully spent: refused. */
export function ladderStep(
  used: number,
  ceiling: number | null,
  previous: LadderStep = "normal",
  thresholds: LadderThresholds = DEFAULT_LADDER_THRESHOLDS,
): LadderStep {
  if (ceiling === null) return "normal";
  if (ceiling <= 0) return "refused";
  const fraction = used / ceiling;
  const prevRank = LADDER_STEPS.indexOf(previous);
  for (const step of ["refused", "degraded", "warning"] as const) {
    const entry = thresholds[step];
    const hold = prevRank >= LADDER_STEPS.indexOf(step) ? entry - thresholds.hysteresis : entry;
    // Compare in whole parts-per-million so 0.9 * 10 lands on 9.
    if (Math.round(fraction * 1e6) >= Math.round(hold * 1e6)) return step;
  }
  return "normal";
}

/** The worst step across several dimensions (#5868 decision 7:
 * a world's state is the worst state among its shards). */
export function worstStep(steps: readonly LadderStep[]): LadderStep {
  let worst = 0;
  for (const s of steps) worst = Math.max(worst, LADDER_STEPS.indexOf(s));
  return LADDER_STEPS[worst];
}

/** The operator action named by a hard-limit refusal. */
export const HARD_LIMIT_ACTION = "delete data to recover headroom, or export and raise the store's ceiling";

/** A refusal at the hard limit itself (threshold 1), for callers
 * that refuse before they know the full size of a write (a bounded
 * body read that ran past the headroom). */
export function hardLimitRefusal(fields: {
  op: OpClass;
  used: number;
  ceiling: number;
  requested: number;
  step: LadderStep;
  store: string;
  backend: BackendId;
  dimension?: BudgetDimension;
}): LimitRefusal {
  return {
    kind: "limit",
    limit: "max_stored_bytes",
    dimension: fields.dimension ?? "stored_bytes",
    step: fields.step,
    op: fields.op,
    used: fields.used,
    ceiling: fields.ceiling,
    requested: fields.requested,
    threshold: 1,
    store: fields.store,
    backend: fields.backend,
    action: HARD_LIMIT_ACTION,
  };
}

export interface AdmissionInput {
  op: OpClass;
  /** Units the operation adds; ≤ 0 for one that adds nothing. */
  delta: number;
  used: number;
  ceiling: number | null;
  previous?: LadderStep;
  thresholds?: LadderThresholds;
  dimension?: BudgetDimension;
  store: string;
  backend: BackendId;
}

export interface Admission {
  step: LadderStep;
}

/** Admit one operation against the ladder (#5868 decisions 3–5).
 *
 *  1. Hard limit (#5863 §4.1, #5868 "at 100 % … all writes"): any
 *     operation that would take usage past the ceiling is refused,
 *     journal appends included — a refused append is reported, never
 *     swallowed (#5868 invariant 1).
 *  2. Ladder: the step's policy decides the class. Journal appends
 *     are allowed at every step, `refused` included: the band from
 *     the refused threshold to the ceiling is their reserve.
 *
 * The step is that of usage before the operation; a caller persists
 * it as the next `previous`. */
export function admit(input: AdmissionInput): Checked<Admission> {
  const thresholds = input.thresholds ?? DEFAULT_LADDER_THRESHOLDS;
  const dimension = input.dimension ?? "stored_bytes";
  const step = ladderStep(input.used, input.ceiling, input.previous ?? "normal", thresholds);
  if (input.ceiling === null) return { ok: true, value: { step } };
  const base = {
    kind: "limit" as const,
    limit: "max_stored_bytes" as const,
    dimension,
    step,
    op: input.op,
    used: input.used,
    ceiling: input.ceiling,
    requested: input.delta,
    store: input.store,
    backend: input.backend,
  };
  if (input.delta > 0 && input.used + input.delta > input.ceiling) {
    return { ok: false, refusal: { ...base, threshold: 1, action: HARD_LIMIT_ACTION } };
  }
  if (!opAllowed(step, input.op)) {
    return {
      ok: false,
      refusal: {
        ...base,
        threshold: thresholds[step === "refused" ? "refused" : "degraded"],
        action:
          step === "refused"
            ? "the remaining budget is reserved for journal appends; delete data or raise the ceiling"
            : "this work is shed while the store is degraded; retry once usage falls",
      },
    };
  }
  return { ok: true, value: { step } };
}

// ----- the backend manifest (#5867 §2) ---------------------------------

/** The placement fact for one unit (#5867 §2.2): which backend holds
 * it, at which placement generation, since which journal sequence.
 *
 * On the hosted stack the Durable Object records its own manifest as
 * an observation of its placement; the signed, versioned manifest
 * #5681 defines remains the authority and is the rehome slice's to
 * publish. `signature` is null until that signing exists. */
export interface BackendManifest {
  manifest_version: number;
  /** The unit placed: a binding id (tenant), shard or deployment. */
  unit: string;
  backend: BackendId;
  /** Identifier for this unit's D1 database; never a credential. Required
   * for `d1` placements and omitted for other backends (#15538). */
  database_id?: string;
  /** Monotonic placement generation, ≥ 1. */
  generation: number;
  /** The journal sequence at which this backend became the unit's
   * sole write authority (#5867 §3.6, fence F); 0 for a unit that
   * has lived on one backend since its first entry. */
  cutover_seq: number;
  /** #5863 §5.3 version the store serves. */
  contract_version: number;
  recorded_at: string;
  signature: string | null;
}

function placementUnknown(message: string, generation: number | null = null): StorageRefusal {
  return unavailable("placement_unknown", message, { generation });
}

/** Validate an untrusted manifest record (e.g. read back from
 * storage). Missing, malformed or conflicting data fails closed as
 * unavailable/placement_unknown (#5867 §2.4); a version this code
 * does not understand is a `version` refusal (#5863 §5.3). */
export function validateManifest(value: unknown, expectedUnit?: string): Checked<BackendManifest> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return { ok: false, refusal: placementUnknown("backend manifest is missing or not an object") };
  }
  const m = value as Record<string, unknown>;
  if (m.manifest_version !== MANIFEST_VERSION) {
    if (typeof m.manifest_version === "number" && Number.isInteger(m.manifest_version) && m.manifest_version > MANIFEST_VERSION) {
      return {
        ok: false,
        refusal: { kind: "version", supported: MANIFEST_VERSION, received: m.manifest_version, direction: "newer" },
      };
    }
    return { ok: false, refusal: placementUnknown("backend manifest has no recognised manifest_version") };
  }
  const version = checkContractVersion(m.contract_version);
  if (version !== null) {
    return { ok: false, refusal: version.kind === "version" ? version : placementUnknown("backend manifest contract_version is malformed") };
  }
  if (typeof m.unit !== "string" || m.unit === "") {
    return { ok: false, refusal: placementUnknown("backend manifest names no unit") };
  }
  if (!isBackendId(m.backend)) {
    return { ok: false, refusal: placementUnknown("backend manifest names no known backend") };
  }
  if (m.backend === "d1") {
    if (typeof m.database_id !== "string" || m.database_id.trim() === "") {
      return { ok: false, refusal: placementUnknown("D1 backend manifest has no database_id") };
    }
  } else if (m.database_id !== undefined && m.database_id !== null) {
    return { ok: false, refusal: placementUnknown("non-D1 backend manifest carries a D1 database_id") };
  }
  if (typeof m.generation !== "number" || !Number.isSafeInteger(m.generation) || m.generation < 1) {
    return { ok: false, refusal: placementUnknown("backend manifest generation is not a positive integer") };
  }
  if (typeof m.cutover_seq !== "number" || !Number.isSafeInteger(m.cutover_seq) || m.cutover_seq < 0) {
    return { ok: false, refusal: placementUnknown("backend manifest cutover_seq is not a non-negative integer") };
  }
  if (typeof m.recorded_at !== "string" || Number.isNaN(Date.parse(m.recorded_at))) {
    return { ok: false, refusal: placementUnknown("backend manifest recorded_at is not a timestamp") };
  }
  if (m.signature !== null && typeof m.signature !== "string") {
    return { ok: false, refusal: placementUnknown("backend manifest signature is malformed") };
  }
  if (expectedUnit !== undefined && m.unit !== expectedUnit) {
    return {
      ok: false,
      refusal: placementUnknown("backend manifest names a different unit; conflicting placement fails closed", m.generation),
    };
  }
  return {
    ok: true,
    value: {
      manifest_version: MANIFEST_VERSION,
      unit: m.unit,
      backend: m.backend,
      ...(m.backend === "d1" ? { database_id: m.database_id as string } : {}),
      generation: m.generation,
      cutover_seq: m.cutover_seq,
      contract_version: m.contract_version as number,
      recorded_at: m.recorded_at,
      signature: (m.signature as string | null) ?? null,
    },
  };
}

/** The first manifest for a unit: generation 1 on `backend`. */
export function createManifest(input: {
  unit: string;
  backend: BackendId;
  databaseId?: string;
  recordedAt: string;
  cutoverSeq?: number;
}): BackendManifest {
  const checked = validateManifest({
    manifest_version: MANIFEST_VERSION,
    unit: input.unit,
    backend: input.backend,
    database_id: input.databaseId,
    generation: 1,
    cutover_seq: input.cutoverSeq ?? 0,
    contract_version: CONTRACT_VERSION,
    recorded_at: input.recordedAt,
    signature: null,
  });
  if (!checked.ok) throw new Error(`invalid backend manifest: ${JSON.stringify(checked.refusal)}`);
  return checked.value;
}

/** The next generation after a cutover (#5867 §3.6): a new backend
 * from fence `cutoverSeq`, which may not move backwards. */
export function advanceManifest(
  current: BackendManifest,
  next: { backend: BackendId; databaseId?: string; cutoverSeq: number; recordedAt: string },
): Checked<BackendManifest> {
  if (!Number.isSafeInteger(next.cutoverSeq) || next.cutoverSeq < current.cutover_seq) {
    return {
      ok: false,
      refusal: verificationRefusal(
        "sequence_misaligned",
        `cutover ${next.cutoverSeq} is before the current cutover ${current.cutover_seq}`,
      ),
    };
  }
  return validateManifest({
    ...current,
    backend: next.backend,
    database_id:
      next.backend === "d1"
        ? next.databaseId ?? (current.backend === "d1" ? current.database_id : undefined)
        : undefined,
    generation: current.generation + 1,
    cutover_seq: next.cutoverSeq,
    recorded_at: next.recordedAt,
    signature: null,
  });
}

/** #5867 §2.3: a write carries the generation it resolved; one that
 * no longer matches is refused so a stale writer cannot continue on
 * an old placement. null = the caller presented none. */
export function checkPlacementGeneration(manifest: BackendManifest, presented: number | null): StorageRefusal | null {
  if (presented === null || presented === manifest.generation) return null;
  return unavailable(
    "placement_stale",
    `placement generation ${presented} is not the current generation ${manifest.generation}`,
    { requested: presented, generation: manifest.generation },
  );
}
