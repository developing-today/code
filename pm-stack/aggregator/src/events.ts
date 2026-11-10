// The cross-stack event log (#1739), audit v2 since #3322 slice A.
// One append-only chronological stream of what happened across the
// pm-stack: dashboard action outcomes, orchestration publishes / tier
// changes, and session registry outcomes — the facts the actions
// store, the orchestration section, and the session registry each
// held separately, with no single stream a human (or /today) could
// read.
//
// Audit v2 (#3322 slice A, follow-on to the #1739 stream): every
// stored record is a versioned AuditEvent carrying a stable id,
// separate event (`at`) and ingest (`ingestedAt`) timestamps, actor
// attribution with an explicit confidence (pre-SSO / unknown actors
// are marked as such and never asserted as authenticated), a source
// identity (+ optional link), and a hash link (`prevHash` / `hash`)
// over its predecessor so silent edits or deletions of stored
// records are detectable (`verifyEventChain`, `EventsStore.health`).
// Corrections are separate append-only `correction` records naming
// the record they correct (`corrects`) — a stored record is never
// edited in place. Retention pruning (below) is the one sanctioned
// removal: the first retained record's `prevHash` stays as the
// anchor naming the pruned predecessor, so a pruned chain still
// verifies link-by-link from its anchor.
//
// Failed appends are no longer lost (#3322 slice A): when a persist
// fails, the record is written to a durable retry outbox
// (`<file>.outbox.jsonl`) before the error is rethrown, and the
// outbox is flushed — merged by stable id, then cleared — on load
// and on every subsequent append. `EventsStore.health()` exposes the
// pending-retry count and the integrity verdict as a readable
// surface; failures are also logged loudly at the point of failure.
//
// Storage follows the views/comments store discipline (#906, #1276):
// one JSON file in the aggregator's data directory
// (PM_AGG_EVENTS_FILE); writes are atomic (unique temp file + rename)
// and serialized through a per-store promise chain; a missing file is
// the only silent empty start — an unparseable or wrongly-shaped file
// is quarantined to <file>.corrupt-<timestamp> with a loud log, never
// silently overwritten. v1 files ({version: 1}) remain readable:
// their rows are migrated in memory to v2 records with deterministic
// stable ids (see migrateV1Event) and the file is rewritten as v2 on
// the next persist.
//
// Retention is bounded (#1739 acceptance): records older than 30 days
// or beyond the newest 5000 leave the hot stream on append and on
// load, whichever bound hits first. Since the #3322 retention
// contract they are ARCHIVED BEFORE THEY ARE PRUNED, never silently
// dropped: pruned records are appended (deduplicated by stable id) to
// monthly archive partitions under `<file>.archive/`
// (`events-archive-YYYY-MM.json`, keyed by the record's `at` month),
// each partition carrying the records' original hashes so its
// integrity verifies independently (`verifyArchivedRecords`). If an
// archive write fails, pruning is deferred — the records stay in the
// hot stream and health() reports the archive failure — because a
// smaller hot stream is recoverable and a lost audit record is not.
// The one exception is a record that was never chained (hash === "",
// e.g. a v1 migration row pruned on its first load): it cannot carry
// integrity evidence into the archive, so it is dropped with a loud
// log and surfaces via health()'s history gap. A chain that FAILED
// verification is never laundered into the archive either: load keeps
// the legacy in-memory prune for it and the violation stays visible.
// health() also reports the history gap directly: when the hot
// chain's retention anchor names a pruned predecessor that no archive
// partition holds (history pruned before this contract landed, or a
// lost partition), the gap is a named field, not a silent hole. The
// full contract and the restore procedure are documented in
// docs/audit-event-retention.md.
//
// Writers are fire-and-forget (see emitEvent): an event write failure
// never blocks or fails the underlying action, publish, or merge —
// and since slice A the failed record waits in the retry outbox
// instead of being lost.
import { createHash, randomUUID } from "node:crypto";
import { appendFile, mkdir, readFile, readdir, rename, rm, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { isIsoDate } from "./validate.ts";

export interface StreamEvent {
  /** ISO timestamp of the event. */
  at: string;
  /** Who did it: drew / orchestrator / keeper / a job id / a session. */
  actor: string;
  /** action-applied | action-rejected | tier-change |
   *  orchestration-published | session-outcome | comment | ... */
  kind: string;
  /** The item / session / goal the event concerns. */
  ref: string;
  /** One-line human-readable summary. */
  summary: string;
}

/** How sure the stream is that `actor` is who it claims to be.
 *  `authenticated` may only be recorded with `asserted: true`;
 *  pre-SSO and unknown actors are explicitly marked, never silently
 *  upgraded. */
export interface EventAttribution {
  confidence: "authenticated" | "pre-sso" | "unknown";
  asserted: boolean;
}

/** Where a record came from: the writing component's identity and,
 *  when there is one, a link to the source artifact. */
export interface EventSource {
  identity: string;
  link?: string;
}

/** The v2 audit record (#3322 slice A): a StreamEvent plus the audit
 *  fields. `action` in the #3322 wording is `kind`; `object` is
 *  `ref` — the v1 field names are kept so every existing reader and
 *  writer keeps working unchanged. */
export interface AuditEvent extends StreamEvent {
  /** Record format version, per record. */
  version: 2;
  /** Stable unique id, assigned at first append and never changed. */
  id: string;
  /** ISO timestamp of when the store ingested the record; `at` is
   *  when the event itself happened. */
  ingestedAt: string;
  attribution: EventAttribution;
  source: EventSource;
  /** For `kind: "correction"` records: the id of the record this
   *  correction annotates. The corrected record is never modified. */
  corrects?: string;
  /** Set on records migrated in memory from a v1 store file. */
  migratedFromVersion?: 1;
  /** Hash of the previous record in the stored chain (the genesis
   *  constant for the first record ever; after retention pruning,
   *  the hash of the last pruned record — the chain anchor). */
  prevHash: string;
  /** sha256 over `prevHash` + this record's canonical fields. */
  hash: string;
}

export type NewStreamEvent = Omit<StreamEvent, "at"> & { at?: string };

/** A new audit record: everything a writer may supply. The store
 *  fills `id` (when omitted), `ingestedAt`, and the hash link. */
export type NewAuditEvent = NewStreamEvent & {
  id?: string;
  attribution?: EventAttribution;
  source?: EventSource;
  corrects?: string;
};

export interface EventsFile {
  version: 2;
  events: AuditEvent[];
}

interface EventsFileV1 {
  version: 1;
  events: StreamEvent[];
}

export const EVENT_RETENTION_DAYS = 30;
export const EVENT_RETENTION_MAX = 5000;

/** prevHash of the first record in an unpruned chain. */
export const EVENT_CHAIN_GENESIS = "0".repeat(64);

const FIELD = /^[\w ./#:@-]+$/;

// Boundary guard (#7740): a writer that raw-sliced actor/summary at the
// 200/500 caps can hand validateStreamEvent a value whose last code unit
// is a lone high surrogate (the astral pair was split at the cap and the
// low half is already gone). Never persist that: drop the dangling half
// so the stored, hash-chained record round-trips through UTF-8 cleanly.
function dropLoneTrailingSurrogate(t: string): string {
  if (t.length === 0) return t;
  const c = t.charCodeAt(t.length - 1);
  return c >= 0xd800 && c <= 0xdbff ? t.slice(0, -1) : t;
}

export function validateStreamEvent(e: unknown): { ok: true; value: StreamEvent } | { ok: false; error: string } {
  if (typeof e !== "object" || e === null) return { ok: false, error: "event must be an object" };
  const v = e as Record<string, unknown>;
  // Real calendar dates only (#5154): Date.parse rolls day-overflow
  // dates over (2026-02-30 parses as March 2), so the shared isIsoDate
  // primitive from validate.ts is the gate, like the DNS/charges
  // boundaries. Genuine ISO timestamps (leap dates, offsets, fractional
  // seconds) still pass.
  if (typeof v.at !== "string" || !isIsoDate(v.at)) {
    return { ok: false, error: "event.at must be an ISO timestamp string" };
  }
  // Identity fields align with their upstream contracts (#4489 Codex
  // re-review): the action-result route accepts any nonblank byRun
  // (stored as appliedByRun, sliced to 200) - punctuation such as
  // parentheses included - and an action objectRef is any
  // whitespace-free string of at most 300 chars (isObjectRef in
  // comments.ts). kind is an internal constant, so it keeps the
  // narrow charset.
  if (typeof v.actor !== "string" || v.actor.trim() === "" || v.actor.trim().length > 200) {
    return { ok: false, error: "event.actor must be a non-empty string of at most 200 chars" };
  }
  if (typeof v.kind !== "string" || v.kind.trim() === "" || v.kind.length > 200 || !FIELD.test(v.kind)) {
    return { ok: false, error: "event.kind must be a non-empty string" };
  }
  if (typeof v.ref !== "string" || v.ref.trim() === "" || v.ref.trim().length > 300 || /\s/.test(v.ref.trim())) {
    return { ok: false, error: "event.ref must be a non-empty ref string of at most 300 chars" };
  }
  if (typeof v.summary !== "string" || v.summary.trim() === "" || v.summary.length > 500) {
    return { ok: false, error: "event.summary must be a non-empty string of at most 500 chars" };
  }
  return {
    ok: true,
    value: {
      at: v.at,
      actor: dropLoneTrailingSurrogate((v.actor as string).trim()),
      kind: (v.kind as string).trim(),
      ref: (v.ref as string).trim(),
      summary: dropLoneTrailingSurrogate(v.summary.trim()),
    },
  };
}

function isStreamEvent(value: unknown): value is StreamEvent {
  return validateStreamEvent(value).ok;
}

/** Validate the v2-only fields a writer may supply. Attribution that
 *  claims `authenticated` without asserting it is rejected outright:
 *  an unauthenticated actor must be recorded as pre-sso / unknown. */
function validateAuditExtras(input: NewAuditEvent): { ok: true } | { ok: false; error: string } {
  if (input.id !== undefined && (typeof input.id !== "string" || input.id.trim() === "" || input.id.length > 200)) {
    return { ok: false, error: "event.id must be a non-empty string of at most 200 chars" };
  }
  if (input.attribution !== undefined) {
    const a = input.attribution;
    if (
      typeof a !== "object" ||
      a === null ||
      !["authenticated", "pre-sso", "unknown"].includes(a.confidence) ||
      typeof a.asserted !== "boolean"
    ) {
      return {
        ok: false,
        error: "event.attribution must be {confidence: authenticated|pre-sso|unknown, asserted: boolean}",
      };
    }
    if (a.confidence === "authenticated" && !a.asserted) {
      return { ok: false, error: "event.attribution: confidence 'authenticated' requires asserted: true" };
    }
  }
  if (input.source !== undefined) {
    const s = input.source;
    if (
      typeof s !== "object" ||
      s === null ||
      typeof s.identity !== "string" ||
      s.identity.trim() === "" ||
      s.identity.length > 200
    ) {
      return { ok: false, error: "event.source.identity must be a non-empty string of at most 200 chars" };
    }
    if (s.link !== undefined && (typeof s.link !== "string" || s.link.length > 500)) {
      return { ok: false, error: "event.source.link must be a string of at most 500 chars" };
    }
  }
  if (input.corrects !== undefined && (typeof input.corrects !== "string" || input.corrects.trim() === "")) {
    return { ok: false, error: "event.corrects must be the non-empty id of the record being corrected" };
  }
  return { ok: true };
}

function canonicalFields(e: AuditEvent): string {
  return JSON.stringify({
    version: e.version,
    id: e.id,
    at: e.at,
    ingestedAt: e.ingestedAt,
    actor: e.actor,
    attribution: e.attribution,
    kind: e.kind,
    ref: e.ref,
    summary: e.summary,
    source: e.source,
    corrects: e.corrects ?? null,
    migratedFromVersion: e.migratedFromVersion ?? null,
  });
}

/** The hash a record must carry given its predecessor's hash. */
export function eventHash(prevHash: string, e: AuditEvent): string {
  return createHash("sha256")
    .update(`${prevHash}\n${canonicalFields(e)}`)
    .digest("hex");
}

/** Recompute the chain in stored order and re-stamp prevHash/hash.
 *  Used at persist time so backfilled (out-of-order `at`) appends
 *  still produce one consistent chain in stored order. */
function rechain(events: AuditEvent[]): AuditEvent[] {
  let prev = EVENT_CHAIN_GENESIS;
  return events.map((e) => {
    const stamped = { ...e, prevHash: prev };
    stamped.hash = eventHash(prev, stamped);
    prev = stamped.hash;
    return stamped;
  });
}

/** Verify a stored chain: every record's hash must recompute from
 *  its fields and its recorded prevHash, and every record after the
 *  first must link to its predecessor's hash. The first record's
 *  prevHash is the retention anchor (genesis, or the last pruned
 *  record's hash) and is covered by its own hash, so pruning does
 *  not break verification — but a silently edited field, a re-stamped
 *  hash, or a deleted middle record all fail here. */
export function verifyEventChain(
  events: readonly AuditEvent[],
): { ok: true } | { ok: false; error: string; index: number } {
  for (let i = 0; i < events.length; i++) {
    const e = events[i]!;
    if (eventHash(e.prevHash, e) !== e.hash) {
      return {
        ok: false,
        error: `record ${e.id} (index ${i}): hash does not match its fields — stored record was altered`,
        index: i,
      };
    }
    if (i > 0 && e.prevHash !== events[i - 1]!.hash) {
      return {
        ok: false,
        error: `record ${e.id} (index ${i}): prevHash does not match its predecessor — a record was deleted or reordered`,
        index: i,
      };
    }
  }
  return { ok: true };
}

/** Migrate one v1 row to a v2 record. The stable id is deterministic
 *  (a hash of the row's fields + position) so the same v1 file
 *  migrates to the same ids on every load until it is persisted as
 *  v2. Ingest time is unknown for migrated rows; `ingestedAt` falls
 *  back to the event time and `migratedFromVersion` records why —
 *  attribution is `unknown`, never asserted. */
export function migrateV1Event(row: StreamEvent, index: number): AuditEvent {
  const digest = createHash("sha256")
    .update(JSON.stringify([index, row.at, row.actor, row.kind, row.ref, row.summary]))
    .digest("hex")
    .slice(0, 24);
  return {
    ...row,
    version: 2,
    id: `evt_v1_${digest}`,
    ingestedAt: row.at,
    attribution: { confidence: "unknown", asserted: false },
    source: { identity: "unknown (migrated from v1 events file)" },
    migratedFromVersion: 1,
    prevHash: EVENT_CHAIN_GENESIS,
    hash: "",
  };
}

function isAuditEvent(value: unknown): value is AuditEvent {
  if (!isStreamEvent(value)) return false;
  const v = value as unknown as Record<string, unknown>;
  return (
    v.version === 2 &&
    typeof v.id === "string" &&
    v.id !== "" &&
    typeof v.ingestedAt === "string" &&
    isIsoDate(v.ingestedAt) &&
    typeof v.prevHash === "string" &&
    typeof v.hash === "string"
  );
}

/** Drop records past the retention bounds (#1739): older than
 *  EVENT_RETENTION_DAYS, or beyond the newest EVENT_RETENTION_MAX.
 *  Input and output are chronological (oldest first). */
export function pruneEvents<T extends StreamEvent>(events: readonly T[], now = new Date()): T[] {
  const cutoff = now.getTime() - EVENT_RETENTION_DAYS * 24 * 60 * 60 * 1000;
  const fresh = events
    .map((event, index) => ({ event, index }))
    .filter(({ event }) => Date.parse(event.at) >= cutoff)
    .sort((a, b) => Date.parse(a.event.at) - Date.parse(b.event.at) || a.index - b.index)
    .map(({ event }) => event);
  return fresh.length > EVENT_RETENTION_MAX ? fresh.slice(fresh.length - EVENT_RETENTION_MAX) : fresh;
}

/** Split a stream into the records retention keeps hot and the
 *  records it would drop, using exactly pruneEvents' bounds (older
 *  than EVENT_RETENTION_DAYS, or beyond the newest
 *  EVENT_RETENTION_MAX). WHICH records survive is pruneEvents'
 *  decision; the ORDER of both halves is the input's (stored-chain)
 *  order — never pruneEvents' chronological sort — because the store
 *  persists `retained` and verifies its hash links in stored order
 *  (#3322 Codex review, PR #7665: re-sorting a backfilled record
 *  into the middle broke the chain and set the sticky
 *  integrityViolation). `pruned` keeps that same stored order, which
 *  is the order the archive partitions preserve. This partition is
 *  the single gate for BOTH bounds: nothing leaves the hot stream
 *  except through it, and the store archives `pruned` before
 *  dropping it (#3322 retention contract). */
export function partitionRetention<T extends StreamEvent>(
  events: readonly T[],
  now = new Date(),
): { retained: T[]; pruned: T[] } {
  const kept = new Set<T>(pruneEvents(events, now));
  const retained: T[] = [];
  const pruned: T[] = [];
  for (const event of events) (kept.has(event) ? retained : pruned).push(event);
  return { retained, pruned };
}

/** One monthly archive partition file: pruned audit records in
 *  stored-chain order, with their original hashes. */
export interface EventArchivePartition {
  version: 1;
  /** ISO timestamp of the most recent write to this partition. */
  archivedAt: string;
  records: AuditEvent[];
}

/** The partition file a record belongs to, by its `at` month. */
export function archivePartitionName(atIso: string): string {
  return `events-archive-${atIso.slice(0, 7)}.json`;
}

/** Verify archived records: every record's hash must recompute from
 *  its fields and its recorded prevHash (the same per-record check
 *  verifyEventChain makes). Inter-record links are counted, not
 *  required: a partition holds a contiguous chain slice in the
 *  normal flow, but a backfilled record pruned out of stored order
 *  legitimately breaks contiguity while its own hash still proves
 *  it unaltered. `linksVerified` reports how much of the chain the
 *  archive proves continuous. */
export function verifyArchivedRecords(
  records: readonly AuditEvent[],
): { ok: boolean; records: number; linksVerified: number; error?: string } {
  let linksVerified = 0;
  for (let i = 0; i < records.length; i++) {
    const e = records[i]!;
    // A damaged partition can hold non-records (e.g. a null from a
    // torn write): that is an archive verdict, never a throw —
    // health() must report the unhealthy archive, not reject
    // (#3322 Codex re-review, PR #7665).
    if (
      typeof e !== "object" ||
      e === null ||
      typeof e.id !== "string" ||
      typeof e.prevHash !== "string" ||
      typeof e.hash !== "string"
    ) {
      return {
        ok: false,
        records: records.length,
        linksVerified,
        error: `archived record at index ${i} is malformed (not an audit record) — archive partition is damaged`,
      };
    }
    if (e.hash === "" || eventHash(e.prevHash, e) !== e.hash) {
      return {
        ok: false,
        records: records.length,
        linksVerified,
        error: `archived record ${e.id} (index ${i}): hash does not match its fields — archived record was altered`,
      };
    }
    if (i > 0 && e.prevHash === records[i - 1]!.hash) linksVerified++;
  }
  return { ok: true, records: records.length, linksVerified };
}

export interface EventFilter {
  kind?: string;
  ref?: string;
  refs?: readonly string[];
  actor?: string;
  project?: string;
  epic?: string;
  search?: string;
  /** Metadata resolved from the live project pool; event records stay source-native. */
  itemContext?: ReadonlyMap<string, { project?: string | null; epic?: string | null }>;
}

/** The read surface: newest first, with stable ids breaking equal-time ties. */
export function filterEvents<T extends StreamEvent>(events: readonly T[], filter: EventFilter = {}): T[] {
  const eq = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();
  const normalizeRef = (ref: string) => ref.replace(/^gh:/i, "").toLowerCase();
  const refs = [filter.ref, ...(filter.refs ?? [])].filter((ref): ref is string => Boolean(ref));
  const search = filter.search?.trim().toLowerCase();
  // Newest first by parsed time. Audit ids break equal-time ties so a
  // cursor remains stable when later records are appended; legacy
  // StreamEvent callers without ids retain append-order tie handling.
  return [...events]
    .map((event, index) => ({ event, index }))
    .sort((a, b) => {
      const time = Date.parse(b.event.at) - Date.parse(a.event.at);
      if (time) return time;
      const aId = "id" in a.event && typeof a.event.id === "string" ? a.event.id : null;
      const bId = "id" in b.event && typeof b.event.id === "string" ? b.event.id : null;
      return aId !== null && bId !== null ? bId.localeCompare(aId) : b.index - a.index;
    })
    .map(({ event }) => event)
    .filter((e) => {
      const context = filter.itemContext?.get(normalizeRef(e.ref));
      return (
        (!filter.kind || eq(e.kind, filter.kind)) &&
        (!refs.length || refs.some((ref) => normalizeRef(e.ref) === normalizeRef(ref))) &&
        (!filter.actor || eq(e.actor, filter.actor)) &&
        (!filter.project || Boolean(context?.project && eq(context.project, filter.project))) &&
        (!filter.epic || Boolean(context?.epic && eq(context.epic, filter.epic))) &&
        (!search || `${e.actor} ${e.kind} ${e.ref} ${e.summary}`.toLowerCase().includes(search))
      );
    });
}

export interface EventPageCursor {
  at: string;
  id: string;
  snapshotAt: string;
}

export interface EventPage {
  events: AuditEvent[];
  nextCursor: string | null;
  snapshotAt: string;
}

function compareEventOrder(a: Pick<AuditEvent, "at" | "id">, b: Pick<AuditEvent, "at" | "id">): number {
  return Date.parse(b.at) - Date.parse(a.at) || b.id.localeCompare(a.id);
}

export function encodeEventCursor(cursor: EventPageCursor): string {
  return Buffer.from(JSON.stringify(cursor), "utf8").toString("base64url");
}

export function decodeEventCursor(value: string | null | undefined): EventPageCursor | null {
  if (!value) return null;
  try {
    const parsed: unknown = JSON.parse(Buffer.from(value, "base64url").toString("utf8"));
    if (
      typeof parsed === "object" &&
      parsed !== null &&
      typeof (parsed as EventPageCursor).at === "string" &&
      Number.isFinite(Date.parse((parsed as EventPageCursor).at)) &&
      typeof (parsed as EventPageCursor).id === "string" &&
      typeof (parsed as EventPageCursor).snapshotAt === "string" &&
      Number.isFinite(Date.parse((parsed as EventPageCursor).snapshotAt))
    )
      return parsed as EventPageCursor;
  } catch {
    // An invalid cursor starts a new first page; it never becomes a filter.
  }
  return null;
}

/** Stable descending keyset pagination over a fixed ingestion snapshot. */
export function pageEvents(
  events: readonly AuditEvent[],
  options: { cursor?: string | null; limit?: number } = {},
): EventPage {
  const limit = Math.max(1, Math.min(100, Math.floor(options.limit ?? 50) || 50));
  const cursor = decodeEventCursor(options.cursor);
  // Compare ingestion timestamps as instants, never lexicographically:
  // migrated v1 rows keep their original timestamp as `ingestedAt`,
  // including UTC offsets and missing fractional seconds, and string
  // order disagrees with instant order for those (Codex P2, PR #5266).
  const snapshotAt =
    cursor?.snapshotAt ??
    events.reduce(
      (latest, event) => (Date.parse(event.ingestedAt) > Date.parse(latest) ? event.ingestedAt : latest),
      "1970-01-01T00:00:00.000Z",
    );
  const snapshotMs = Date.parse(snapshotAt);
  const visible = events.filter(
    (event) => Date.parse(event.ingestedAt) <= snapshotMs && (!cursor || compareEventOrder(event, cursor) > 0),
  );
  const page = visible.slice(0, limit);
  const last = page.at(-1);
  return {
    events: page,
    snapshotAt,
    nextCursor: visible.length > limit && last ? encodeEventCursor({ at: last.at, id: last.id, snapshotAt }) : null,
  };
}

/**
 * The fire-and-forget writer every emission point uses (#1739
 * acceptance: writing an event never blocks the underlying action).
 * Never throws: a failed append is logged and reported as `false`;
 * the caller's action / publish / merge proceeds regardless. Since
 * #3322 slice A a failed append to the real store is also persisted
 * to the durable retry outbox, so "lost" below means "not yet in the
 * main file" — it is retried on the next load / append.
 */
export async function emitEvent(
  store: { record: (event: NewAuditEvent) => Promise<AuditEvent | StreamEvent> },
  event: NewAuditEvent,
): Promise<boolean> {
  try {
    await store.record(event);
    return true;
  } catch (err) {
    console.error(
      `[events] WARNING: event append failed (${err instanceof Error ? err.message : String(err)}); ` +
        `the underlying action proceeds — event queued for retry if the store has an outbox, otherwise lost: ${event.kind} ${event.ref}`,
    );
    return false;
  }
}

export interface EventsHealth {
  version: 2;
  events: number;
  /** Records waiting in the durable retry outbox. */
  pendingRetries: number;
  integrity: { ok: boolean; detail: string };
  /** Records preserved in the archive partitions (#3322 retention
   *  contract): history that left the hot stream is counted here,
   *  never silently dropped. */
  archivedRecords: number;
  /** The archive's own integrity verdict (per-record hash
   *  verification across all partitions), or the last archive
   *  write failure that deferred pruning. */
  archive: { ok: boolean; detail: string };
  /** Visible-gap report: present when the hot chain's retention
   *  anchor names a pruned predecessor that no archive partition
   *  holds — history outside the retained+archived set. */
  historyGap: { present: boolean; detail: string };
}

// -- the store ------------------------------------------------------------

export class EventsStore {
  private events: AuditEvent[] = [];
  private loadPromise: Promise<void> | null = null;
  private writeChain: Promise<void> = Promise.resolve();
  private recordChain: Promise<void> = Promise.resolve();
  private writeSeq = 0;
  private integrityViolation: string | null = null;
  private archiveError: string | null = null;
  /** Missing-anchor evidence captured at load, before retention
   *  or rechaining can touch the chain (#3322 Codex re-review,
   *  PR #7665): preserved for health()'s history-gap report. */
  private missingAnchorDetail: string | null = null;
  /** Loss evidence for never-chained records dropped by retention
   *  (#3322 Codex re-review, PR #7665): unstamped rows (e.g. v1
   *  migration rows pruned on their first load) carry no hash and a
   *  genesis predecessor, so the anchor check in archiveHealth()
   *  cannot see their loss — it is recorded here at the moment the
   *  records are actually dropped, and health() reports it as a
   *  history gap. */
  private droppedUnstampedDetail: string | null = null;

  constructor(private readonly file: string) {}

  /** The durable retry outbox: one JSON audit record per line,
   *  written when a persist fails, flushed on load / next append. */
  get outboxFile(): string {
    return `${this.file}.outbox.jsonl`;
  }

  /** The archive partition directory (#3322 retention contract):
   *  monthly `events-archive-YYYY-MM.json` files holding the records
   *  retention pruned from the hot stream. */
  get archiveDir(): string {
    return `${this.file}.archive`;
  }

  async load(): Promise<void> {
    if (!this.loadPromise) this.loadPromise = this.doLoad();
    return this.loadPromise;
  }

  private async doLoad(): Promise<void> {
    let raw: string;
    try {
      raw = await readFile(this.file, "utf8");
    } catch (err) {
      if ((err as { code?: unknown })?.code === "ENOENT") {
        this.events = [];
        await this.flushOutbox();
        return;
      }
      this.events = [];
      await this.quarantine(`could not be read (${err instanceof Error ? err.message : String(err)})`);
      return;
    }
    try {
      const parsed: unknown = JSON.parse(raw);
      if (typeof parsed !== "object" || parsed === null || !Array.isArray((parsed as EventsFile).events)) {
        throw new Error("store file does not have the expected { events: [...] } shape");
      }
      const fileVersion = (parsed as { version?: unknown }).version;
      const rows = (parsed as { events: unknown[] }).events;
      const normalized: AuditEvent[] = [];
      for (let i = 0; i < rows.length; i++) {
        const row = rows[i];
        if (isAuditEvent(row) && fileVersion === 2 && row.hash === "") {
          // A v2 main-file record with an empty hash is damaged, not
          // unhashed-by-design (#3322 Codex re-review, PR #5137):
          // persist() rechains before every write, so a record the
          // store wrote always carries a hash. Accepting an empty
          // one would exclude it from verification (both checks
          // filter hash !== "") and let a silent edit pass as
          // healthy. Unhashed records remain legitimate ONLY as v1
          // migration rows (fileVersion 1, handled below) and as
          // retry-outbox lines (parsed in flushOutbox, never here).
          throw new Error(`event row ${i} is a v2 audit record with an empty hash`);
        }
        if (isAuditEvent(row)) {
          normalized.push({
            ...row,
            attribution: row.attribution ?? { confidence: "unknown", asserted: false },
            source: row.source ?? { identity: "unknown" },
          });
        } else if (fileVersion === 1 && isStreamEvent(row)) {
          // Legacy migration is gated on the FILE version (#3322
          // Codex re-review, PR #5137): in a v2 file, a row that
          // fails the v2 shape is a damaged audit record, not a v1
          // row — migrating it would give it an empty hash, exclude
          // it from verification, and hide a possible silent edit.
          // It falls through to the shape error below instead, and
          // the file is quarantined with its bytes preserved.
          normalized.push(migrateV1Event(row, i));
        } else {
          throw new Error(`event row ${i} does not have the expected StreamEvent/AuditEvent shape`);
        }
      }
      // Integrity is checked against the STORED hashes before any
      // persist can re-stamp them — and before retention pruning
      // drops any record (#3322 Codex re-review, PR #5137): pruning
      // first would let an edit hide by also backdating the altered
      // record past the retention cutoff (it would be dropped
      // unverified, and the remaining chain's first record is a
      // trusted retention anchor), so the full normalized chain
      // verifies first and pruning applies only afterwards. A
      // mismatch means the file was altered outside the store. The
      // records still load (the stream stays readable) but the
      // violation is loud and is reported by health() until a
      // human looks.
      const verdict = verifyEventChain(normalized.filter((e) => e.hash !== ""));
      // Missing-anchor evidence is captured BEFORE retention and
      // before any persist can rechain the retained records from
      // genesis (#3322 Codex re-review, PR #7665): if the loaded
      // chain's first record names a predecessor that no archive
      // partition holds, that gap is a fact about the loaded store
      // and must survive this load's archiving + rechaining.
      const loadAnchor = normalized[0]?.prevHash;
      if (loadAnchor && loadAnchor !== EVENT_CHAIN_GENESIS) {
        let anchorArchived = false;
        let archiveReadable = false;
        try {
          const archivedNow = await this.listArchived();
          archiveReadable = true;
          anchorArchived = archivedNow.some((e) => e?.hash === loadAnchor);
        } catch {
          archiveReadable = false;
        }
        if (archiveReadable && !anchorArchived) {
          this.missingAnchorDetail =
            "the loaded chain's retention anchor (prevHash " + loadAnchor.slice(0, 12) + "…) names a predecessor " +
            "that no archive partition holds — history before the anchor is outside the retained+archived set " +
            "(pruned before the #3322 archive contract, or a lost partition)";
        }
      }
      if (verdict.ok) {
        // Archive before pruning (#3322 retention contract): records
        // leaving the hot stream are preserved in the archive
        // partitions first, then the retained set is persisted so
        // the hot file no longer carries them. A chain that FAILED
        // verification is never laundered into the archive — it
        // keeps the legacy in-memory prune (else-branch) and the
        // violation stays visible via health().
        this.events = normalized;
        const archived = await this.applyRetention();
        if (archived > 0) {
          try {
            await this.persist();
          } catch (err) {
            console.error(
              `[events] WARNING: archived ${archived} record(s) from ${this.file} but could not persist the retained stream ` +
                `(${err instanceof Error ? err.message : String(err)}); the hot file still holds them and they will be ` +
                `re-archived (deduplicated by id) on the next load.`,
            );
          }
        }
      } else {
        this.events = pruneEvents(normalized);
      }
      if (!verdict.ok) {
        this.integrityViolation = verdict.error;
        console.error(
          `[events] WARNING: events store ${this.file} failed its integrity check: ${verdict.error}. ` +
            `Stored audit records must not be edited or deleted outside retention pruning — investigate before trusting this stream.`,
        );
      }
    } catch (err) {
      this.events = [];
      await this.quarantine(`is unparseable (${err instanceof Error ? err.message : String(err)})`);
      return;
    }
    await this.flushOutbox();
  }

  private async quarantine(why: string): Promise<void> {
    const quarantined = `${this.file}.corrupt-${new Date().toISOString().replace(/[:.]/g, "-")}`;
    try {
      await rename(this.file, quarantined);
      console.error(
        `[events] WARNING: events store ${this.file} ${why}; quarantined the original bytes to ${quarantined} and started empty. ` +
          `The quarantined file holds the event history — do not delete it.`,
      );
    } catch (renameErr) {
      console.error(
        `[events] WARNING: events store ${this.file} ${why} and could not be quarantined ` +
          `(${renameErr instanceof Error ? renameErr.message : String(renameErr)}); starting empty — ` +
          `the next persist will overwrite it, so copy it aside now to preserve the event history.`,
      );
    }
  }

  private persist(): Promise<void> {
    const run = this.writeChain.then(async () => {
      // Never re-stamp a chain that failed verification (#3322
      // Codex re-review, PR #5137): rechain() recomputes every
      // hash, so an ordinary append after a tamper would launder
      // the altered history into a valid chain and health() would
      // report success after a restart, with nobody having
      // investigated. Refuse instead — the caller's record is
      // queued in the retry outbox, the on-disk evidence stays
      // byte-identical, and the violation stays visible until a
      // human investigates and repairs the store deliberately.
      if (this.integrityViolation) {
        throw new Error(
          `events store integrity violation: ${this.integrityViolation} — refusing to persist over a chain that failed verification`,
        );
      }
      const verdict = verifyEventChain(this.events.filter((e) => e.hash !== ""));
      if (!verdict.ok) {
        this.integrityViolation = verdict.error;
        throw new Error(
          `events store integrity violation: ${verdict.error} — refusing to persist over a chain that failed verification`,
        );
      }
      this.events = rechain(this.events);
      const data: EventsFile = { version: 2, events: this.events };
      await mkdir(dirname(this.file), { recursive: true });
      const tmp = `${this.file}.tmp-${process.pid}-${++this.writeSeq}`;
      await writeFile(tmp, JSON.stringify(data, null, 2));
      await rename(tmp, this.file);
    });
    this.writeChain = run.catch(() => {});
    return run;
  }

  /** Persist one record to the retry outbox after a failed append.
   *  Best-effort on top of a failure (the disk may be the problem),
   *  but a failed outbox write is logged as the true loss it is. */
  private async queueForRetry(event: AuditEvent, cause: unknown): Promise<void> {
    try {
      await mkdir(dirname(this.file), { recursive: true });
      await appendFile(this.outboxFile, `${JSON.stringify(event)}\n`);
      console.error(
        `[events] WARNING: event ${event.id} (${event.kind} ${event.ref}) could not be appended ` +
          `(${cause instanceof Error ? cause.message : String(cause)}); persisted to the retry outbox ${this.outboxFile} — ` +
          `it will be merged on the next load/append. Pending retries are visible via the events health surface.`,
      );
    } catch (outboxErr) {
      console.error(
        `[events] WARNING: event ${event.id} (${event.kind} ${event.ref}) could not be appended and the retry outbox write also failed ` +
          `(${outboxErr instanceof Error ? outboxErr.message : String(outboxErr)}); this event is lost.`,
      );
    }
  }

  /** Merge any outbox records not already in the stream (by stable
   *  id), persist, and clear the outbox. Runs on load and at the
   *  start of every append; a still-failing persist leaves the
   *  outbox in place for the next attempt. */
  private async flushOutbox(): Promise<void> {
    let raw: string;
    try {
      raw = await readFile(this.outboxFile, "utf8");
    } catch {
      return; // no outbox — the common case
    }
    const pending: AuditEvent[] = [];
    for (const line of raw.split("\n")) {
      if (line.trim() === "") continue;
      try {
        const parsed: unknown = JSON.parse(line);
        if (isAuditEvent(parsed)) pending.push(parsed);
        else
          console.error(
            `[events] WARNING: dropping an unparseable retry-outbox line in ${this.outboxFile}: ${line.slice(0, 200)}`,
          );
      } catch {
        console.error(
          `[events] WARNING: dropping an unparseable retry-outbox line in ${this.outboxFile}: ${line.slice(0, 200)}`,
        );
      }
    }
    const known = new Set(this.events.map((e) => e.id));
    // An outbox record can already be archived (#3322 Codex
    // re-review, PR #7665): a backup copies the outbox first, then
    // the running store can flush and archive that record before
    // the hot file and archive are copied. Deduping only against
    // the hot stream would append a twin that can survive count
    // retention and leave one stable id in both histories with
    // different hashes — include archived ids, as recordSerialized
    // does for direct replays.
    try {
      for (const e of await this.listArchived()) if (e?.id) known.add(e.id);
    } catch (err) {
      console.error(
        `[events] WARNING: could not check the archive while flushing the retry outbox for ${this.file} ` +
          `(${err instanceof Error ? err.message : String(err)}); proceeding with hot-stream dedupe only — ` +
          `the archive failure is reported via health().`,
      );
    }
    const fresh = pending.filter((e) => !known.has(e.id));
    if (fresh.length > 0) {
      // Merged unpruned: the persist below chains (hash-stamps) the
      // fresh records first, and retention runs afterwards so a
      // pruned record always carries its hash into the archive
      // (#3322 retention contract).
      this.events = [...this.events, ...fresh];
    }
    // Clear the outbox only after a successful persist of everything
    // it held (#3322 Codex re-review, PR #5137): a pending record can
    // already be in memory — record() keeps the event in this.events
    // when its persist fails — so "already known" does not mean
    // durable. Clearing on presence alone dropped the earlier record
    // when the next append also failed and the process restarted:
    // only the newer failure was re-queued.
    if (fresh.length > 0 || pending.length > 0) {
      try {
        await this.persist();
      } catch {
        return; // still failing — leave the outbox for the next attempt
      }
      await this.applyRetentionAfterPersist();
    }
    await rm(this.outboxFile, { force: true });
  }

  private async pendingRetryCount(): Promise<number> {
    try {
      const raw = await readFile(this.outboxFile, "utf8");
      return raw.split("\n").filter((line) => line.trim() !== "").length;
    } catch {
      return 0;
    }
  }

  /** All archived records, partition by partition (filename order is
   *  chronological by month), each partition in stored-chain order.
   *  A missing archive directory is an empty archive; a corrupt
   *  partition is a loud error, never a silent skip — the archive is
   *  audit history. */
  async listArchived(): Promise<AuditEvent[]> {
    let names: string[];
    try {
      names = await readdir(this.archiveDir);
    } catch (err) {
      if ((err as { code?: unknown })?.code === "ENOENT") return [];
      throw err;
    }
    const records: AuditEvent[] = [];
    for (const name of names
      .filter((n) => n.startsWith("events-archive-") && n.endsWith(".json"))
      .sort()) {
      const parsed: unknown = JSON.parse(await readFile(join(this.archiveDir, name), "utf8"));
      if (
        typeof parsed !== "object" ||
        parsed === null ||
        !Array.isArray((parsed as EventArchivePartition).records)
      ) {
        throw new Error(`archive partition ${name} does not have the expected { records: [...] } shape`);
      }
      // Iterate, never spread: a monthly partition has no record-count
      // bound, and pushing an entire partition as call arguments throws
      // RangeError once it is large enough (#3322 Codex review, PR #7665).
      for (const record of (parsed as EventArchivePartition).records) records.push(record);
    }
    return records;
  }

  /** Append records to their monthly archive partitions,
   *  deduplicated by stable id, verifying each merged partition
   *  before it is written (atomically, temp file + rename, like the
   *  hot file). Throws on any failure — the caller then defers
   *  pruning rather than lose the records. */
  private async archivePruned(records: readonly AuditEvent[]): Promise<void> {
    const byPartition = new Map<string, AuditEvent[]>();
    for (const record of records) {
      const name = archivePartitionName(record.at);
      const batch = byPartition.get(name) ?? [];
      batch.push(record);
      byPartition.set(name, batch);
    }
    await mkdir(this.archiveDir, { recursive: true });
    for (const [name, batch] of byPartition) {
      const path = join(this.archiveDir, name);
      let existing: AuditEvent[] = [];
      try {
        const parsed: unknown = JSON.parse(await readFile(path, "utf8"));
        // A partition that parses but is not the expected shape is
        // DAMAGED, not empty: treating it as empty would overwrite the
        // original bytes with only this batch and destroy recoverable
        // history. Fail instead, so pruning is deferred and the file
        // stays available for investigation (#3322 Codex review, PR #7665).
        if (
          typeof parsed !== "object" ||
          parsed === null ||
          !Array.isArray((parsed as EventArchivePartition).records)
        ) {
          throw new Error(`archive partition ${name} does not have the expected { records: [...] } shape`);
        }
        existing = (parsed as EventArchivePartition).records;
      } catch (err) {
        if ((err as { code?: unknown })?.code !== "ENOENT") throw err;
      }
      const known = new Set(existing.map((e) => e.id));
      const merged = [...existing, ...batch.filter((e) => !known.has(e.id))];
      const verdict = verifyArchivedRecords(merged);
      if (!verdict.ok) {
        throw new Error(`archive partition ${name} failed verification: ${verdict.error}`);
      }
      const data: EventArchivePartition = {
        version: 1,
        archivedAt: new Date().toISOString(),
        records: merged,
      };
      const tmp = `${path}.tmp-${process.pid}-${++this.writeSeq}`;
      await writeFile(tmp, JSON.stringify(data, null, 2));
      await rename(tmp, path);
    }
  }

  /** The retention gate (#3322): partition the hot stream, archive
   *  the pruned records, and only then drop them from memory.
   *  Returns how many records were archived. On archive failure the
   *  hot stream is left untouched (pruning deferred, failure
   *  recorded for health()) — a record is never dropped unarchived.
   *  Never-chained records (hash === "") cannot carry integrity
   *  evidence into an archive; they are dropped with a loud log and
   *  surface through health()'s history gap. */
  private async applyRetention(now = new Date()): Promise<number> {
    const { retained, pruned } = partitionRetention(this.events, now);
    if (pruned.length === 0) return 0;
    const stamped = pruned.filter((e) => e.hash !== "");
    const unstamped = pruned.length - stamped.length;
    if (unstamped > 0) {
      console.error(
        `[events] WARNING: ${unstamped} never-chained record(s) in ${this.file} fell outside retention ` +
          `before being persisted into the hash chain and cannot be archived with integrity evidence; ` +
          `dropping them — this loss is reported via the events health history gap.`,
      );
    }
    if (stamped.length > 0) {
      try {
        await this.archivePruned(stamped);
      } catch (err) {
        this.archiveError =
          `archive write failed (${err instanceof Error ? err.message : String(err)}); ` +
          `pruning deferred — records remain in the hot stream`;
        console.error(`[events] WARNING: ${this.archiveError} for ${this.file}.`);
        return 0;
      }
    }
    this.archiveError = null;
    if (unstamped > 0) {
      // The unstamped records are dropped by the assignment below
      // (they are not in `retained`): record the loss as sticky gap
      // evidence now, because after rechaining the retained chain
      // starts at genesis and the anchor check alone would report
      // complete history.
      this.droppedUnstampedDetail =
        `${unstamped} never-chained record(s) in ${this.file} fell outside retention ` +
        `before being persisted into the hash chain and were dropped unarchived \u2014 ` +
        `history they carried is outside the retained+archived set`;
    }
    // A timestamp-based retention decision can remove a record from
    // the middle of append/stored order when events were backfilled.
    // We verified the complete old chain before reaching this gate
    // (load verifies before retention; append persists/verifies before
    // retention), and the removed records now have their original
    // hashes safely archived. If their removal leaves a hole, rechain
    // only the retained hot stream so its next persist verifies. This
    // is a deliberate retention transition, not a repair of an
    // unverified chain.
    this.events = verifyEventChain(retained).ok ? retained : rechain(retained);
    return stamped.length;
  }

  /** Retention after a successful persist (the records are chained,
   *  so everything pruned can be archived): archive + prune in
   *  memory, then persist the retained stream. Failures are logged
   *  and swallowed — the records are durable in the hot file either
   *  way and are re-archived (deduplicated) on the next load. */
  private async applyRetentionAfterPersist(): Promise<void> {
    try {
      const archived = await this.applyRetention();
      if (archived > 0) await this.persist();
    } catch (err) {
      console.error(
        `[events] WARNING: retention after persist failed for ${this.file} ` +
          `(${err instanceof Error ? err.message : String(err)}); records remain in the hot file on disk ` +
          `and will be re-archived on the next load/append.`,
      );
    }
  }

  /** The archive side of health(): record count, the archive's own
   *  integrity verdict, and the visible-gap report for the hot
   *  chain's retention anchor. */
  private async archiveHealth(): Promise<
    Pick<EventsHealth, "archivedRecords" | "archive" | "historyGap">
  > {
    let archived: AuditEvent[] = [];
    let readError: string | null = null;
    try {
      archived = await this.listArchived();
    } catch (err) {
      readError = `archive unreadable (${err instanceof Error ? err.message : String(err)})`;
    }
    const verification = readError ? null : verifyArchivedRecords(archived);
    const archive = this.archiveError
      ? { ok: false, detail: this.archiveError }
      : readError
        ? { ok: false, detail: readError }
        : verification && !verification.ok
          ? { ok: false, detail: verification.error ?? "archive verification failed" }
          : {
              ok: true,
              detail: `${archived.length} archived record(s) verify (${verification?.linksVerified ?? 0} inter-record link(s) verified)`,
            };
    const stickyGap = [this.missingAnchorDetail, this.droppedUnstampedDetail]
      .filter((d): d is string => d !== null)
      .join("; ");
    if (stickyGap) {
      return {
        archivedRecords: archived.length,
        archive,
        historyGap: { present: true, detail: stickyGap },
      };
    }
    const first = this.events[0];
    let historyGap: EventsHealth["historyGap"];
    if (!first || first.prevHash === EVENT_CHAIN_GENESIS) {
      historyGap = { present: false, detail: "the hot chain starts at genesis — no pruned predecessor" };
    } else if (archived.some((e) => e.hash === first.prevHash)) {
      historyGap = {
        present: false,
        detail: "the hot chain's retention anchor is the last archived record — retained + archived history is continuous",
      };
    } else {
      historyGap = {
        present: true,
        detail:
          `the hot chain's retention anchor (prevHash ${first.prevHash.slice(0, 12)}…) names a pruned predecessor ` +
          `that no archive partition holds — history before the anchor is outside the retained+archived set ` +
          `(pruned before the #3322 archive contract, or a lost partition)`,
      };
    }
    return { archivedRecords: archived.length, archive, historyGap };
  }

  /** The readable failure/integrity surface (#3322 slice A): how
   *  many records wait in the retry outbox, and whether the stored
   *  hash chain still verifies. */
  async health(): Promise<EventsHealth> {
    await this.load();
    const verdict = this.integrityViolation
      ? { ok: false, detail: this.integrityViolation }
      : (() => {
          const v = verifyEventChain(this.events.filter((e) => e.hash !== ""));
          return v.ok ? { ok: true, detail: "hash chain verifies" } : { ok: false, detail: v.error };
        })();
    return {
      version: 2,
      events: this.events.length,
      pendingRetries: await this.pendingRetryCount(),
      integrity: verdict,
      ...(await this.archiveHealth()),
    };
  }

  /** All retained events, newest first, with optional filters. */
  async list(filter: EventFilter = {}): Promise<AuditEvent[]> {
    await this.load();
    return filterEvents(this.events, filter);
  }

  /** Durable identity check used to avoid replaying an orchestration run
   *  after process restart. */
  async has(kind: string, ref: string): Promise<boolean> {
    await this.load();
    return this.events.some((event) => event.kind === kind && event.ref === ref);
  }

  /** Append one event (retention enforced on append). On a persist
   *  failure the record is queued in the durable retry outbox and
   *  the error is rethrown, so fire-and-forget callers still see the
   *  failure exactly as before — the record is retried, not lost.
   *  Whole calls are serialized on recordChain (#3322 Codex
   *  re-review, PR #5137): flushOutbox's outbox removal and a failed
   *  append's queueForRetry are both file operations on the outbox,
   *  and interleaved they lose retries — a flush that read the
   *  outbox before a concurrent failure queued its record could
   *  unlink the file after the queue, deleting a record no
   *  successful persist had ever held. Serializing the complete
   *  flush/append/retry lifecycle makes the removal always cover
   *  exactly the snapshot that was persisted. */
  async record(input: NewAuditEvent): Promise<AuditEvent> {
    let result!: AuditEvent;
    const run = this.recordChain.then(async () => {
      result = await this.recordSerialized(input);
    });
    this.recordChain = run.then(
      () => {},
      () => {},
    );
    await run;
    return result;
  }

  private async recordSerialized(input: NewAuditEvent): Promise<AuditEvent> {
    await this.load();
    await this.flushOutbox();
    const parsed = validateStreamEvent({ ...input, at: input.at ?? new Date().toISOString() });
    if (!parsed.ok) throw new Error(parsed.error);
    const extras = validateAuditExtras(input);
    if (!extras.ok) throw new Error(extras.error);
    // A caller-supplied id is a stable identity (#3322 Codex
    // re-review, PR #5137): replaying it returns the existing record
    // instead of appending a twin that the id can no longer
    // unambiguously name (record() and recordCorrection() both
    // resolve ids with find()). flushOutbox above has already
    // merged any pending retry with this id into the stream, so a
    // retry of a queued-but-unlanded append replays correctly too.
    if (input.id !== undefined) {
      const existing = this.events.find((e) => e.id === input.id);
      if (existing) return existing;
      // The id may already have left the hot stream: retention
      // archives before pruning, so a replayed stable id can live
      // only in the archive (#3322 Codex re-review, PR #7665).
      // Resolve it there too — appending a twin would put two
      // records with one stable id in the retained+archived set.
      try {
        const archivedHit = (await this.listArchived()).find((e) => e?.id === input.id);
        if (archivedHit) return archivedHit;
      } catch (err) {
        console.error(
          `[events] WARNING: could not check the archive for replayed id ${input.id} in ${this.file} ` +
            `(${err instanceof Error ? err.message : String(err)}); proceeding with the append — if that id was ` +
            `archived, this may duplicate it; the archive failure is reported via health().`,
        );
      }
    }
    // Ingestion stamps are strictly monotonic within the stream
    // (#3322, PR #7665): pageEvents fixes its snapshot at the newest
    // ingestedAt it has seen and excludes anything ingested later,
    // but a wall-clock stamp can tie at millisecond resolution when
    // appends land in the same ms — a tied late append then leaks
    // into a snapshot taken before it existed. Stamping at least
    // 1ms past the newest stamp already in the stream (record calls
    // are serialized on recordChain, so this is race-free) makes
    // the snapshot boundary exact.
    const lastIngestedMs = this.events.reduce(
      (latest, e) => Math.max(latest, Date.parse(e.ingestedAt) || 0),
      0,
    );
    const ingestedAt = new Date(Math.max(Date.now(), lastIngestedMs + 1)).toISOString();
    const event: AuditEvent = {
      ...parsed.value,
      version: 2,
      id: input.id ?? randomUUID(),
      ingestedAt,
      attribution: input.attribution ?? { confidence: "unknown", asserted: false },
      source: input.source ?? { identity: "pm-stack/aggregator" },
      ...(input.corrects !== undefined ? { corrects: input.corrects } : {}),
      prevHash: EVENT_CHAIN_GENESIS,
      hash: "",
    };
    this.events = [...this.events, event];
    try {
      await this.persist();
      // Retention after the persist (#3322): the new record is now
      // chained, so anything the bounds push out of the hot stream
      // is archived before it is pruned.
      await this.applyRetentionAfterPersist();
    } catch (err) {
      await this.queueForRetry(event, err);
      throw err;
    }
    // persist() rechains in place-order; return the stamped record.
    return this.events.find((e) => e.id === event.id) ?? event;
  }

  /** Append a correction to an existing record (#3322 slice A):
   *  corrections are separate, attributable, append-only records —
   *  the corrected record is never edited. */
  async recordCorrection(
    targetId: string,
    input: Omit<NewAuditEvent, "kind" | "ref"> & { kind?: string; ref?: string },
  ): Promise<AuditEvent> {
    await this.load();
    const target = this.events.find((e) => e.id === targetId);
    if (!target) throw new Error(`cannot correct event ${targetId}: no such record in the stream`);
    return this.record({ ...input, kind: "correction", ref: input.ref ?? target.ref, corrects: targetId });
  }
}
