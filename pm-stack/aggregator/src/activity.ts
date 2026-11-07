// Activity ingestion for the cross-stack stream (#3321): the
// watchdog-event and Kaneo-change writers that map external activity
// into the #1739 StreamEvent log, with the ingestion discipline the
// local fire-and-forget writers never needed:
//
// - Stable source IDs: every delivery carries a per-source stable id
//   (`watchdog:<input>@<since>` for a watchdog breach episode,
//   `kaneo:<project>/<task>:status:<from>-><to>:<updatedAt>` for a
//   Kaneo change). The same real-world change always produces the
//   same id, which is what makes dedupe possible. Two refinements
//   from the Codex review of PR #5141: a Kaneo card with no
//   `updatedAt` gets a per-card transition sequence instead of the
//   constant `undated` stamp (so a later repeat of the same
//   transition is a new delivery, while a retry of the same observed
//   transition keeps its id), and watchdog episodes pass through
//   WatchdogEpisodeStabilizer, because watchdogTierItems assigns
//   `now` (or a refreshed probe time) as `since` for several breach
//   kinds and the raw derivation would mint a new episode id on
//   every render.
// - A persisted per-source checkpoint + seen-id set (one JSON file,
//   PM_AGG_ACTIVITY_FILE, following the events store's discipline:
//   atomic temp+rename writes serialized through a promise chain, a
//   missing file is the only silent empty start, a corrupt file is
//   quarantined loudly). Replaying a duplicate delivery is a no-op,
//   and a restart resumes from the checkpoint — the seen set and the
//   checkpoint survive the process, so nothing is re-emitted.
// - Gap records: when a source is known to be breached / unreachable
//   for a window, the window is recorded as a gap (source + time
//   range) instead of silently passing as complete history. A later
//   recovery/backfill pass flips the gap's status; the /events page
//   renders open gaps as holes, never as coverage.
//
// The GitHub slice is deliberately NOT here: #3321's boundary says
// GitHub activity reuses the #2072 webhook/hot-list ingestion rather
// than a competing receiver, and #2072 has not landed. The coverage
// table below states that plainly (github: unsupported) instead of
// implying coverage that does not exist.
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname } from "node:path";
import type { EventsStore } from "./events.ts";
import type { TierItem } from "./model.ts";
import { isIsoDate, isNonNegative } from "./validate.ts";

export type ActivitySourceId = "watchdog" | "kaneo" | "github";

/** One normalized activity delivery, before it becomes a StreamEvent. */
export interface ActivityDelivery {
  source: ActivitySourceId;
  /** Stable per-source event id — the dedupe key. */
  sourceId: string;
  /** ISO timestamp of the change at the source. */
  at: string;
  /** StreamEvent kind ("watchdog-event" | "kaneo-change"). */
  kind: string;
  /** StreamEvent ref (the item the change concerns). */
  ref: string;
  /** One-line human summary. */
  summary: string;
  /** Source link (e.g. the Kaneo card URL), appended to the summary. */
  link?: string;
}

/** Documented coverage for one activity source (#3321 acceptance:
 *  coverage distinguishes supported / unsupported / historical). */
export interface ActivityCoverage {
  source: ActivitySourceId;
  status: "supported" | "unsupported";
  /** What is ingested today. */
  supported: string;
  /** What is explicitly not ingested today. */
  unsupported: string;
  /** How far back the ingested history reaches. */
  historical: string;
}

export const ACTIVITY_SOURCE_COVERAGE: readonly ActivityCoverage[] = [
  {
    source: "watchdog",
    status: "supported",
    supported:
      "Watchdog breach episodes (fleet-feed stale/absent/future-dated, orchestration stale/invalid/absent, registry stale, Kaneo/Grist/Pad probe failures) are written to the stream as watchdog-event records, one per episode, keyed by the stable watchdog:<input> id and the episode's since timestamp.",
    unsupported:
      "Sub-threshold degradations the watchdog does not diagnose, and inputs outside watchdog.ts's diagnosed set.",
    historical:
      "From the first ingestion run forward. Watchdog diagnoses are derived live and were never persisted before #3321, so no pre-ingestion breach history exists or is claimed.",
  },
  {
    source: "kaneo",
    status: "supported",
    supported:
      "Kaneo board changes observed by the live source's board polls: a card appearing in a WIP column, and a card's status (column) change, written as kaneo-change records with the card's Kaneo URL as the source link.",
    unsupported:
      "Card edits that change neither presence nor status (title/priority/body edits), cards in to-do columns the board does not fetch, and deletions (a vanished card is indistinguishable from the done-aging filter at this surface).",
    historical:
      "From the first board poll after ingestion starts: the first poll is a silent baseline (no events), so changes that happened while the aggregator was down surface only if they are still visible as a status/presence diff at the next poll — outages open gap records instead of claiming that history.",
  },
  {
    source: "github",
    status: "unsupported",
    supported: "Nothing yet.",
    unsupported:
      "All GitHub activity. #3321's boundary forbids a competing receiver: GitHub events enter through the #2072 webhook/hot-list ingestion, which has not landed. When it does, its writer joins this module with the same checkpoint/dedupe discipline.",
    historical: "None claimed. GitHub's own history stays in GitHub until #2072 lands.",
  },
];

// -- writers: pure mappings from source observations to deliveries -------

/** Map watchdog tier items to deliveries, one per breach episode.
 *  Callers whose items come from repeated live derivations should
 *  pass them through WatchdogEpisodeStabilizer first (below): the
 *  delivery id `watchdog:<input>@<since>` is only as stable as the
 *  item's `since`, and not every breach kind derives a stable one. */
export function watchdogDeliveries(items: readonly TierItem[]): ActivityDelivery[] {
  return items.map((item) => ({
    source: "watchdog" as const,
    sourceId: `${item.id}@${item.since}`,
    at: item.since,
    kind: "watchdog-event",
    ref: item.id,
    summary: item.title.slice(0, 500),
  }));
}

/** Pins each watchdog breach episode to the `since` it was FIRST
 *  observed with, until the episode clears (Codex P2, PR #5141).
 *  watchdogTierItems is pure and, for absent/future-dated feeds and
 *  absent/invalid orchestration sections, assigns the current `now`
 *  as `since`; failing probes carry each refresh's probe timestamp.
 *  Without this pin, every render of one continuous outage would mint
 *  a different delivery id and flood the bounded stream with
 *  duplicates of the same episode, evicting unrelated history. The
 *  pin is per item id and is forgotten as soon as the id stops being
 *  derived, so a later, genuinely new episode gets its own id. */
export class WatchdogEpisodeStabilizer {
  private firstSinceById = new Map<string, string>();

  stabilize(items: readonly TierItem[]): TierItem[] {
    const present = new Set(items.map((i) => i.id));
    for (const id of [...this.firstSinceById.keys()]) {
      if (!present.has(id)) this.firstSinceById.delete(id);
    }
    return items.map((item) => {
      const first = this.firstSinceById.get(item.id);
      if (first !== undefined) return first === item.since ? item : { ...item, since: first };
      this.firstSinceById.set(item.id, item.since);
      return item;
    });
  }
}

/** The board-diffable shape of one Kaneo card. */
export interface KaneoCardSnapshot {
  id: string;
  status: string;
  title: string;
  updatedAt?: string;
  kaneoUrl?: string;
  /** Per-card count of transitions the caller had already ingested
   *  for this card as of this snapshot (maintained across snapshots
   *  by the caller; see kaneoChangeDeliveries). Only used to key
   *  UNDATED deliveries, where no source timestamp exists. */
  seq?: number;
}

/** Diff two successive Kaneo board snapshots into change deliveries.
 *  Pure. A card that appears produces one delivery; a card whose
 *  status changed produces one; unchanged cards produce none. The
 *  delivery id embeds the card's updatedAt when the source provides
 *  it, so two distinct changes to the same card are distinct
 *  deliveries while a re-diff of the same change is a duplicate.
 *  When the source omits updatedAt (which the live reader explicitly
 *  supports), the id instead carries the card's next transition
 *  sequence, (prev snapshot's `seq`) + 1: successive observed moves
 *  in-progress -> in-review -> in-progress -> in-review then get
 *  distinct ids instead of colliding on a constant `undated` stamp
 *  (Codex P2, PR #5141), while re-diffing the SAME observed
 *  transition against the SAME prev snapshot reproduces the same
 *  id, so a retry after a failed append stays a dedupe-safe replay.
 *  Callers persist the bumped seq in their stored snapshot once the
 *  delivery is acknowledged (see live.ts board()). */
export function kaneoChangeDeliveries(
  prev: ReadonlyMap<string, KaneoCardSnapshot>,
  next: readonly KaneoCardSnapshot[],
): ActivityDelivery[] {
  const out: ActivityDelivery[] = [];
  for (const card of next) {
    const before = prev.get(card.id);
    const stamp = card.updatedAt ?? `undated:${(card.seq ?? 0) + 1}`;
    if (!before) {
      out.push({
        source: "kaneo",
        sourceId: `${card.id}:appeared:${stamp}`,
        at: card.updatedAt ?? new Date().toISOString(),
        kind: "kaneo-change",
        ref: card.id,
        summary: `Kaneo card appeared in ${card.status}: ${card.title}`.slice(0, 500),
        ...(card.kaneoUrl ? { link: card.kaneoUrl } : {}),
      });
    } else if (before.status !== card.status) {
      const undatedSeq = (before.seq ?? card.seq ?? 0) + 1;
      out.push({
        source: "kaneo",
        sourceId: `${card.id}:status:${before.status}->${card.status}:${card.updatedAt ?? `undated:${undatedSeq}`}`,
        at: card.updatedAt ?? new Date().toISOString(),
        kind: "kaneo-change",
        ref: card.id,
        summary: `Kaneo card moved ${before.status} -> ${card.status}: ${card.title}`.slice(0, 500),
        ...(card.kaneoUrl ? { link: card.kaneoUrl } : {}),
      });
    }
  }
  return out;
}

/** Highest undated transition sequence already used per card, parsed
 *  from persisted delivery ids (`<card>:appeared:undated:<n>` and
 *  `<card>:status:<a>-><b>:undated:<n>`). A restarted process seeds
 *  its in-memory counters from this so its next undated delivery for
 *  a card cannot reuse an id the ingestion store has already seen
 *  and silently dedupe-discard a genuine change (Codex P2, PR
 *  #5141). Ids without an undated stamp carry no sequence and are
 *  ignored. */
export function maxUndatedSeqByCard(seenIds: readonly string[]): Map<string, number> {
  const out = new Map<string, number>();
  for (const id of seenIds) {
    const m = /^(.*):undated:(\d+)$/.exec(id);
    if (!m) continue;
    const head = m[1]!;
    let cardId: string | null = null;
    const statusAt = head.indexOf(":status:");
    if (statusAt >= 0) cardId = head.slice(0, statusAt);
    else if (head.endsWith(":appeared")) cardId = head.slice(0, -":appeared".length);
    if (!cardId) continue;
    const seq = Number(m[2]);
    if (Number.isFinite(seq) && seq > (out.get(cardId) ?? 0)) out.set(cardId, seq);
  }
  return out;
}

// -- gap records -----------------------------------------------------------

export type GapStatus = "open" | "backfilled" | "unrecoverable";

export interface GapRecord {
  id: string;
  /** Source the hole belongs to ("watchdog:fleet-feed", "kaneo", ...). */
  source: string;
  /** ISO start of the known-missing window. */
  from: string;
  /** ISO end of the known-missing window (extends while open). */
  to: string;
  status: GapStatus;
  detectedAt: string;
  resolvedAt?: string;
}

// -- the ingestion store ---------------------------------------------------

export interface SourceCheckpoint {
  source: string;
  /** sourceId of the most recently ingested delivery. */
  lastSourceId: string | null;
  /** `at` of the most recently ingested delivery. */
  lastEventAt: string | null;
  /** Deliveries ingested (duplicates excluded). */
  deliveries: number;
  /** Deliveries dropped as duplicates. */
  duplicates: number;
  updatedAt: string;
}

export interface IngestionFile {
  version: 1;
  checkpoints: Record<string, SourceCheckpoint>;
  seen: Record<string, string[]>;
  gaps: GapRecord[];
}

/** Per-source seen-id bound: dedupe memory is finite by design; an id
 *  older than the newest 2000 deliveries from its source can be
 *  re-ingested if replayed (the stream's own 30-day retention makes
 *  that window consistent). */
export const SEEN_RETENTION_MAX = 2000;

function isGapRecord(v: unknown): v is GapRecord {
  if (typeof v !== "object" || v === null) return false;
  const g = v as Record<string, unknown>;
  return (
    typeof g.id === "string" &&
    typeof g.source === "string" &&
    isIsoDate(g.from) &&
    isIsoDate(g.to) &&
    (g.status === "open" || g.status === "backfilled" || g.status === "unrecoverable") &&
    isIsoDate(g.detectedAt) &&
    (g.resolvedAt === undefined || isIsoDate(g.resolvedAt))
  );
}

function isCheckpoint(v: unknown): v is SourceCheckpoint {
  if (typeof v !== "object" || v === null) return false;
  const c = v as Record<string, unknown>;
  return (
    typeof c.source === "string" &&
    (c.lastSourceId === null || typeof c.lastSourceId === "string") &&
    (c.lastEventAt === null || isIsoDate(c.lastEventAt)) &&
    isNonNegative(c.deliveries) &&
    Number.isInteger(c.deliveries) &&
    isNonNegative(c.duplicates) &&
    Number.isInteger(c.duplicates) &&
    isIsoDate(c.updatedAt)
  );
}

export class IngestionStore {
  private data: IngestionFile = { version: 1, checkpoints: {}, seen: {}, gaps: [] };
  private loadPromise: Promise<void> | null = null;
  private ingestChain: Promise<void> = Promise.resolve();
  private writeChain: Promise<void> = Promise.resolve();
  private writeSeq = 0;

  constructor(private readonly file: string) {}

  async load(): Promise<void> {
    if (!this.loadPromise) this.loadPromise = this.doLoad();
    return this.loadPromise;
  }

  private async doLoad(): Promise<void> {
    let raw: string;
    try {
      raw = await readFile(this.file, "utf8");
    } catch (err) {
      if ((err as { code?: unknown })?.code === "ENOENT") return;
      await this.quarantine(`could not be read (${err instanceof Error ? err.message : String(err)})`);
      return;
    }
    try {
      const parsed: unknown = JSON.parse(raw);
      if (typeof parsed !== "object" || parsed === null) throw new Error("not an object");
      const f = parsed as Partial<IngestionFile>;
      if (typeof f.checkpoints !== "object" || f.checkpoints === null || Array.isArray(f.checkpoints)) {
        throw new Error("checkpoints is not an object");
      }
      if (typeof f.seen !== "object" || f.seen === null || Array.isArray(f.seen)) {
        throw new Error("seen is not an object");
      }
      if (!Array.isArray(f.gaps)) throw new Error("gaps is not an array");
      for (const cp of Object.values(f.checkpoints)) {
        if (!isCheckpoint(cp)) throw new Error("a checkpoint row is malformed");
      }
      for (const ids of Object.values(f.seen)) {
        if (!Array.isArray(ids) || ids.some((id) => typeof id !== "string")) {
          throw new Error("a seen row is malformed");
        }
      }
      for (const gap of f.gaps) {
        if (!isGapRecord(gap)) throw new Error("a gap row is malformed");
      }
      this.data = {
        version: 1,
        checkpoints: f.checkpoints as Record<string, SourceCheckpoint>,
        seen: f.seen as Record<string, string[]>,
        gaps: f.gaps as GapRecord[],
      };
    } catch (err) {
      await this.quarantine(`is unparseable (${err instanceof Error ? err.message : String(err)})`);
    }
  }

  private async quarantine(why: string): Promise<void> {
    const quarantined = `${this.file}.corrupt-${new Date().toISOString().replace(/[:.]/g, "-")}`;
    try {
      await rename(this.file, quarantined);
      console.error(
        `[activity] WARNING: ingestion store ${this.file} ${why}; quarantined the original bytes to ${quarantined} and started empty. ` +
          `Dedupe memory was reset — replays may re-emit already-streamed activity once.`,
      );
    } catch (renameErr) {
      console.error(
        `[activity] WARNING: ingestion store ${this.file} ${why} and could not be quarantined ` +
          `(${renameErr instanceof Error ? renameErr.message : String(renameErr)}); starting empty.`,
      );
    }
  }

  private persist(): Promise<void> {
    const run = this.writeChain.then(async () => {
      await mkdir(dirname(this.file), { recursive: true });
      const tmp = `${this.file}.tmp-${process.pid}-${++this.writeSeq}`;
      await writeFile(tmp, JSON.stringify(this.data, null, 2));
      await rename(tmp, this.file);
    });
    this.writeChain = run.catch(() => {});
    return run;
  }

  async checkpoint(source: string): Promise<SourceCheckpoint | null> {
    await this.load();
    return this.data.checkpoints[source] ?? null;
  }

  async checkpoints(): Promise<SourceCheckpoint[]> {
    await this.load();
    return Object.values(this.data.checkpoints).sort((a, b) => a.source.localeCompare(b.source));
  }

  /** The persisted seen-id set for a source (dedupe memory). Read
   *  by callers that keep in-memory counters keyed to those ids —
   *  e.g. the live source's undated Kaneo sequence — so a restart
   *  can resume above every id already ingested (#3321). */
  async seenIds(source: string): Promise<string[]> {
    await this.load();
    return [...(this.data.seen[source] ?? [])];
  }

  async gaps(): Promise<GapRecord[]> {
    await this.load();
    return [...this.data.gaps].sort((a, b) => Date.parse(a.from) - Date.parse(b.from));
  }

  /**
   * Ingest deliveries into the stream with dedupe + checkpointing.
   * A delivery whose sourceId is already in the source's seen set is
   * a duplicate: dropped, counted, never re-emitted (so replaying a
   * delivery batch is a no-op). A delivery the stream rejects is NOT
   * marked seen, so a later replay can retry it. The checkpoint and
   * seen set persist before returning, so a restart resumes exactly
   * here.
   */
  /** Serialize whole ingest operations (Codex P2, PR #5141): the
   *  dedupe check and the stream append are a check-and-act pair, so
   *  two concurrent ingest calls carrying the same delivery must not
   *  both pass the seen check before either appends. The persist
   *  write chain alone does not prevent that — it serializes file
   *  writes, not this operation. A failed operation does not wedge
   *  the chain: later calls still run, and unappended deliveries stay
   *  unseen so a replay can retry them. */
  async ingest(
    events: Pick<EventsStore, "record">,
    deliveries: readonly ActivityDelivery[],
  ): Promise<{ ingested: number; duplicates: number; failed: number; failedIds: string[] }> {
    const run = this.ingestChain.then(() => this.ingestSerialized(events, deliveries));
    this.ingestChain = run.then(
      () => {},
      () => {},
    );
    return run;
  }

  private async ingestSerialized(
    events: Pick<EventsStore, "record">,
    deliveries: readonly ActivityDelivery[],
  ): Promise<{ ingested: number; duplicates: number; failed: number; failedIds: string[] }> {
    await this.load();
    let ingested = 0;
    let duplicates = 0;
    let failed = 0;
    const failedIds: string[] = [];
    let dirty = false;
    for (const d of deliveries) {
      const seen = (this.data.seen[d.source] ??= []);
      const cp = (this.data.checkpoints[d.source] ??= {
        source: d.source,
        lastSourceId: null,
        lastEventAt: null,
        deliveries: 0,
        duplicates: 0,
        updatedAt: new Date().toISOString(),
      });
      if (seen.includes(d.sourceId)) {
        duplicates++;
        cp.duplicates++;
        cp.updatedAt = new Date().toISOString();
        dirty = true;
        continue;
      }
      try {
        // Stable id (#3322 v2 record contract): a persist failure queues
        // the record in the retry outbox and rethrows; the replay of this
        // delivery re-presents the same id and returns the landed record
        // instead of appending a duplicate (the v1 rollback this replaces).
        await events.record({
          id: `activity:${d.source}:${d.sourceId}`,
          at: d.at,
          actor: d.source,
          kind: d.kind,
          ref: d.ref,
          summary: (d.link ? `${d.summary} — ${d.link}` : d.summary).slice(0, 500),
        });
      } catch (err) {
        failed++;
        failedIds.push(d.sourceId);
        console.error(
          `[activity] WARNING: stream append failed for ${d.source} delivery ${d.sourceId} ` +
            `(${err instanceof Error ? err.message : String(err)}); left unseen so a replay can retry it.`,
        );
        continue;
      }
      seen.push(d.sourceId);
      if (seen.length > SEEN_RETENTION_MAX) seen.splice(0, seen.length - SEEN_RETENTION_MAX);
      cp.deliveries++;
      cp.lastSourceId = d.sourceId;
      if (!cp.lastEventAt || Date.parse(d.at) >= Date.parse(cp.lastEventAt)) cp.lastEventAt = d.at;
      cp.updatedAt = new Date().toISOString();
      ingested++;
      dirty = true;
    }
    if (dirty) await this.persist();
    return { ingested, duplicates, failed, failedIds };
  }

  /** Record (or extend) an open gap for a source window. An already
   *  open gap for the same source absorbs the window, so a continuing
   *  outage is one record, not a pile. */
  async recordGap(source: string, from: string, to: string): Promise<GapRecord> {
    await this.load();
    const open = this.data.gaps.find((g) => g.source === source && g.status === "open");
    if (open) {
      if (Date.parse(from) < Date.parse(open.from)) open.from = from;
      if (Date.parse(to) > Date.parse(open.to)) open.to = to;
      await this.persist();
      return open;
    }
    const gap: GapRecord = {
      id: `gap:${source}:${from}`,
      source,
      from,
      to,
      status: "open",
      detectedAt: new Date().toISOString(),
    };
    this.data.gaps.push(gap);
    await this.persist();
    return gap;
  }

  /** Flip a gap's status (backfill landed, or the window aged past
   *  any possible recovery). Unknown / already-resolved ids no-op.
   *  When the caller knows when the outage actually ended (`to`,
   *  Codex P2 on PR #5141), the gap's window extends to that moment
   *  before resolving: a gap recorded from failure timestamps alone
   *  otherwise reports a zero/short duration that understates the
   *  interruption, which ran until recovery, not until the last
   *  failed poll. */
  async resolveGap(id: string, status: Exclude<GapStatus, "open">, to?: string): Promise<GapRecord | null> {
    await this.load();
    const gap = this.data.gaps.find((g) => g.id === id);
    if (!gap || gap.status !== "open") return gap ?? null;
    if (to && Date.parse(to) > Date.parse(gap.to)) gap.to = to;
    gap.status = status;
    gap.resolvedAt = new Date().toISOString();
    await this.persist();
    return gap;
  }
}

/** Ingestion for call sites that must never throw on it (page
 *  renders, board polls): failures are logged, never thrown — but
 *  the outcome is RETURNED, so a caller that holds state it may only
 *  discard once a delivery is durable (the live source's Kaneo
 *  baseline, Codex P2 on PR #5141) can check `failed` and retain
 *  the delivery for retry instead of dropping it. */
export async function ingestActivity(
  store: IngestionStore,
  events: Pick<EventsStore, "record">,
  deliveries: readonly ActivityDelivery[],
): Promise<{ ingested: number; duplicates: number; failed: number; failedIds: string[] }> {
  if (!deliveries.length) return { ingested: 0, duplicates: 0, failed: 0, failedIds: [] };
  try {
    return await store.ingest(events, deliveries);
  } catch (err) {
    console.error(
      `[activity] WARNING: ingestion failed (${err instanceof Error ? err.message : String(err)}); ` +
        `deliveries stay unseen and a later pass can retry them.`,
    );
    return { ingested: 0, duplicates: 0, failed: deliveries.length, failedIds: deliveries.map((d) => d.sourceId) };
  }
}

/** Keep watchdog gap records in sync with the current breaches: each
 *  breached input (`watchdog:<input>`) has an open gap from its
 *  episode's since until now; an input that is no longer breached has
 *  its open gap resolved as backfilled (the aggregator's live
 *  re-derivation on recovery is the backfill pass for diagnoses). */
export async function syncWatchdogGaps(
  store: IngestionStore,
  items: readonly TierItem[],
  now: Date = new Date(),
): Promise<void> {
  const breached = new Set(items.map((i) => i.id));
  for (const item of items) {
    await store.recordGap(item.id, item.since, now.toISOString());
  }
  for (const gap of await store.gaps()) {
    if (gap.status === "open" && gap.source.startsWith("watchdog:") && !breached.has(gap.source)) {
      await store.resolveGap(gap.id, "backfilled", now.toISOString());
    }
  }
}

/** Serialize whole watchdog gap-sync passes in observation order
 *  (Codex P2, PR #5141 round 5). A pass is a multi-step
 *  reconciliation — record a gap per current breach, then resolve
 *  the open gaps whose breach is gone — and the store's write chain
 *  serializes only the individual persists, not the pass. Two
 *  passes started fire-and-forget from overlapping derivations can
 *  interleave: an older pass that observed breaches A and B pauses
 *  while persisting A, a newer healthy pass resolves A and
 *  finishes, and the older pass then creates B — an outage the
 *  newest observation says is over stays reported as open until
 *  some later derivation repairs it. Chaining the passes whole, in
 *  the order they were enqueued, makes the newest observation the
 *  last word. A failed pass does not wedge the chain. */
export function createWatchdogGapSyncQueue(): (
  store: IngestionStore,
  items: readonly TierItem[],
  now?: Date,
) => Promise<void> {
  let chain: Promise<void> = Promise.resolve();
  return (store, items, now) => {
    const run = chain.then(
      () => syncWatchdogGaps(store, items, now),
      () => syncWatchdogGaps(store, items, now),
    );
    chain = run.then(
      () => {},
      () => {},
    );
    return run;
  };
}

// -- rendering ---------------------------------------------------------------

function esc(s: unknown): string {
  return String(s ?? "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

/** Render gap records for the /events page: a gap is a hole in the
 *  history, stated as one — never smoothed into apparent coverage. */
export function gapsHtml(gaps: readonly GapRecord[]): string {
  if (!gaps.length) return `<p>No known activity gaps.</p>`;
  const rows = gaps
    .map(
      (g) =>
        `<tr><td>${esc(g.source)}</td><td>${esc(g.from)}</td><td>${esc(g.to)}</td>` +
        `<td>${esc(g.status)}</td><td>${esc(g.detectedAt)}</td></tr>`,
    )
    .join("\n");
  return (
    `<p>These windows are KNOWN HOLES in the activity history for their source — ` +
    `the stream is not complete there, and nothing in it should be read as if it were.</p>` +
    `<table><thead><tr><th>source</th><th>from</th><th>to</th><th>status</th><th>detected</th></tr></thead>` +
    `<tbody>${rows}</tbody></table>`
  );
}

/** Render the activity-source coverage table (#3321 documentation
 *  surface, kept next to the data it describes). */
export function activityCoverageHtml(coverage: readonly ActivityCoverage[] = ACTIVITY_SOURCE_COVERAGE): string {
  const rows = coverage
    .map(
      (c) =>
        `<tr><td>${esc(c.source)}</td><td>${esc(c.status)}</td><td>${esc(c.supported)}</td>` +
        `<td>${esc(c.unsupported)}</td><td>${esc(c.historical)}</td></tr>`,
    )
    .join("\n");
  return (
    `<table><thead><tr><th>source</th><th>status</th><th>supported</th><th>unsupported</th><th>historical</th></tr></thead>` +
    `<tbody>${rows}</tbody></table>`
  );
}
