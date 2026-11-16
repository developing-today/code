import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import type { FreshnessInput, FreshnessNotice, FreshnessSource } from "./freshness.ts";
import { parseIsoDate } from "./validate.ts";

/**
 * #3976: anti-entropy for the #3977 freshness journal + the push-health
 * view the health surfaces render.
 *
 * Event-driven freshness has a silent failure mode: a source accepts a
 * revision (a new fleet-feed snapshot lands, a Kaneo board fingerprint
 * moves, a webhook is applied) but its notice never reaches the journal
 * — the publisher crashed, the IPC POST failed, the process restarted
 * between accept and publish. Nothing downstream can see that hole:
 * the stream is connected, the journal cursor is where it was, and the
 * dashboard keeps rendering the previous canonical data as if current.
 *
 * The reconciler closes the hole on the design cadence (5 minutes):
 * it reads each canonical source's currently-accepted revision — the
 * exact notice identity (eventId) a source-event for that revision
 * carries — and compares it with the journal. A revision with no
 * matching notice gets a synthetic invalidation: a notice published
 * under the revision's own eventId with reason "reconcile", so the
 * late real notice dedupes against it instead of double-invalidating,
 * and every consumer rereads canonical data exactly as if the original
 * notice had arrived.
 *
 * Honesty rules (the issue's, applied to this module):
 * - Push connectivity and source freshness are separate facts. The
 *   push state below describes the journal + reconciler machinery; a
 *   quiet source is "silent", never an outage, and a connected stream
 *   is never reported as proof that source data is fresh.
 * - A source whose canonical revision cannot be read is
 *   "unavailable" with the failure kept — never silently treated as
 *   unchanged, never repaired with an invented revision.
 * - A reconcile that could not make its synthetic notice durable
 *   (the journal answered "pending") leaves the gap OPEN and does
 *   not count as successful.
 * - Journal resets are counted, not hidden: when the journal cursor
 *   moves backwards the retained history was lost, and the health
 *   view says so (resetsTotal / lastResetAt) instead of quietly
 *   re-baselining.
 *
 * Per-source windows: every wired source reconciles on the 5-minute
 * design default. The fleet-feed reader below is the aggregator's own
 * canonical source (its fingerprint is the collector's sha256 of the
 * feed file). Additional sources plug in through CanonicalRevisionReader
 * without changing the loop; a source with a slower measured window
 * records that window on the issue with its missed-event detection
 * bound (= its window + this interval) before being wired slower.
 */

export const RECONCILE_INTERVAL_MS = 5 * 60_000;
/** Two missed intervals: the anti-entropy loop itself is off cadence. */
export const RECONCILE_STALE_MS = 2 * RECONCILE_INTERVAL_MS;
/** A source notice younger than this is "live" (mirrors the delivery
 *  layer's LIVE_WINDOW_MS in freshness-delivery.ts). */
const SOURCE_LIVE_MS = 5 * 60_000;

/** The currently-accepted canonical revision of one source, identified
 *  by the eventId its source-event notice carries (or would carry). */
export interface CanonicalRevision {
  source: FreshnessSource;
  eventId: string;
  scope: string[];
  occurredAt: string | null;
}

/** Reads one source's canonical revision. Returns null when the source
 *  has no accepted revision to check (quiet). Throws when the canonical
 *  state cannot be read — an outage is reported, never guessed at. */
export interface CanonicalRevisionReader {
  source: FreshnessSource;
  read(): Promise<CanonicalRevision | null>;
}

/** The journal seam the reconciler needs — FreshnessStore satisfies it. */
export interface ReconcileJournal {
  listAfter(after: number): Promise<{ cursor: number; resetRequired: boolean; events: FreshnessNotice[] }>;
  publish(input: FreshnessInput): Promise<{ notice: FreshnessNotice; duplicate: boolean; pending: boolean }>;
  /** Optional durable-state verification (FreshnessStore provides
   *  it): resolves while the journal on disk matches what listAfter
   *  serves, throws when the durable journal was lost, replaced, or
   *  became unreadable after startup. listAfter alone cannot detect
   *  that — it serves the in-memory listing. */
  verifyDurable?(): Promise<void>;
}

export type PushState = "up" | "degraded" | "down";
export type PushSourceState = "live" | "stale" | "silent" | "unavailable";

export interface SourcePushHealth {
  source: FreshnessSource;
  state: PushSourceState;
  /** Newest journal notice observedAt for this source (any reason). */
  lastEventAt: string | null;
  lastEventId: string | null;
  /** Age of the newest notice at view time; null when silent. */
  ageMs: number | null;
  /** Detection lag for the source's current canonical revision: notice
   *  observedAt minus revision occurredAt once noticed; while the
   *  notice is missing, now minus occurredAt (the lag still growing).
   *  Null when either timestamp is unknown. */
  lagMs: number | null;
  notices: number;
  /** The current canonical revision has no durable journal notice. */
  missingNotice: boolean;
}

export interface PushHealthView {
  schemaVersion: 1;
  state: PushState;
  detail: string;
  /** Newest notice in the journal, any source — "last event". */
  lastEventAt: string | null;
  lastSuccessfulReconcileAt: string | null;
  lastReconcileAttemptAt: string | null;
  reconcileIntervalMs: number;
  /** Journal cursor at the last successful journal read. */
  cursor: number | null;
  sources: SourcePushHealth[];
  gaps: {
    /** Canonical revisions currently lacking a durable notice. */
    open: number;
    /** Synthetic invalidations this process has published. */
    repairedTotal: number;
    /** Journal resets (cursor moved backwards) this process observed. */
    resetsTotal: number;
    lastResetAt: string | null;
  };
}

const SOURCE_NAMES: FreshnessSource[] = ["github", "kaneo", "fleet-feed", "orchestration", "reconcile"];

export class FreshnessReconciler {
  private lastSuccessfulAtMs: number | null = null;
  private lastAttemptAtMs: number | null = null;
  private journalDown = false;
  private journalError: string | null = null;
  private cursorSeen: number | null = null;
  private lastPage: { cursor: number; events: FreshnessNotice[] } | null = null;
  private resetsTotal = 0;
  private lastResetAtMs: number | null = null;
  private repairedTotal = 0;
  private readonly unavailable = new Map<FreshnessSource, string>();
  private readonly missing = new Map<FreshnessSource, CanonicalRevision>();
  private readonly revisions = new Map<FreshnessSource, CanonicalRevision>();
  private timer: ReturnType<typeof setInterval> | null = null;
  private running = false;

  constructor(
    private readonly journal: ReconcileJournal,
    private readonly readers: CanonicalRevisionReader[],
  ) {}

  /** One reconcile pass. Never throws: failures fold into the health
   *  view (journal down / source unavailable / gap left open). */
  async reconcile(now: Date = new Date()): Promise<PushHealthView> {
    const nowMs = now.getTime();
    this.lastAttemptAtMs = nowMs;
    let page: { cursor: number; resetRequired: boolean; events: FreshnessNotice[] };
    try {
      page = await this.journal.listAfter(0);
    } catch (err) {
      this.journalDown = true;
      this.journalError = err instanceof Error ? err.message : String(err);
      return this.health(now);
    }
    this.journalDown = false;
    this.journalError = null;

    // Durable-state verification: a journal file deleted, replaced, or
    // made unreadable after startup still answers listAfter from
    // memory, so the listing above cannot prove durability. When the
    // journal offers the seam, a failed verification is push down —
    // exactly like an unreadable journal — never a healthy pass over
    // durable state that no longer exists.
    if (this.journal.verifyDurable !== undefined) {
      try {
        await this.journal.verifyDurable();
      } catch (err) {
        this.journalDown = true;
        this.journalError = err instanceof Error ? err.message : String(err);
        return this.health(now);
      }
    }

    // Journal loss/reset: the cursor is monotonic within one journal
    // file, so a backwards move means retained history was replaced.
    if (this.cursorSeen !== null && page.cursor < this.cursorSeen) {
      this.resetsTotal += 1;
      this.lastResetAtMs = nowMs;
    }
    this.cursorSeen = page.cursor;
    this.lastPage = { cursor: page.cursor, events: page.events };

    const noticed = new Set(page.events.map((event) => event.eventId));
    let durable = true;
    for (const reader of this.readers) {
      let revision: CanonicalRevision | null;
      try {
        revision = await reader.read();
        this.unavailable.delete(reader.source);
      } catch (err) {
        // Producer outage: report the source unavailable and leave its
        // previous gap state untouched — nothing is repaired or hidden.
        // The pass is NOT successful: no canonical revision could be
        // checked for this source, so lastSuccessfulReconcileAt must
        // not advance over an outage (Codex review on PR #9193).
        this.unavailable.set(reader.source, err instanceof Error ? err.message : String(err));
        durable = false;
        continue;
      }
      if (revision === null) {
        this.revisions.delete(reader.source);
        this.missing.delete(reader.source);
        continue;
      }
      this.revisions.set(reader.source, revision);
      if (noticed.has(revision.eventId)) {
        this.missing.delete(reader.source);
        continue;
      }
      // Accepted revision, no notice: emit the synthetic invalidation
      // under the revision's own eventId (reason "reconcile").
      try {
        const published = await this.journal.publish({
          eventId: revision.eventId,
          source: revision.source,
          scope: [...revision.scope],
          occurredAt: revision.occurredAt,
          reason: "reconcile",
        });
        if (published.pending) {
          // Accepted but not durable: the gap is still open.
          this.missing.set(reader.source, revision);
          durable = false;
        } else {
          this.missing.delete(reader.source);
          noticed.add(revision.eventId);
          if (!published.duplicate) {
            this.repairedTotal += 1;
            // Keep the cached page truthful for health() until the
            // next pass rereads the journal.
            this.lastPage.events.push(published.notice);
            this.lastPage.cursor = Math.max(this.lastPage.cursor, published.notice.sequence);
            this.cursorSeen = this.lastPage.cursor;
          }
        }
      } catch {
        this.missing.set(reader.source, revision);
        durable = false;
      }
    }
    if (durable) this.lastSuccessfulAtMs = nowMs;
    return this.health(now);
  }

  health(now: Date = new Date()): PushHealthView {
    const nowMs = now.getTime();
    const events = this.lastPage?.events ?? [];
    const bySource = new Map<FreshnessSource, FreshnessNotice[]>();
    for (const event of events) {
      const list = bySource.get(event.source) ?? [];
      list.push(event);
      bySource.set(event.source, list);
    }
    let lastEventAt: string | null = null;
    for (const event of events) {
      if (lastEventAt === null || event.observedAt > lastEventAt) lastEventAt = event.observedAt;
    }

    const sources: SourcePushHealth[] = SOURCE_NAMES.map((source) => {
      const rows = bySource.get(source) ?? [];
      const newest = rows.reduce<FreshnessNotice | null>(
        (best, event) => (best === null || event.observedAt > best.observedAt ? event : best),
        null,
      );
      const revision = this.revisions.get(source) ?? null;
      let lagMs: number | null = null;
      if (revision !== null && revision.occurredAt !== null) {
        const occurredMs = Date.parse(revision.occurredAt);
        if (Number.isFinite(occurredMs)) {
          const notice = rows.find((event) => event.eventId === revision.eventId);
          lagMs = notice ? Math.max(0, Date.parse(notice.observedAt) - occurredMs) : Math.max(0, nowMs - occurredMs);
        }
      }
      const unavailableReason = this.unavailable.get(source);
      const state: PushSourceState =
        unavailableReason !== undefined
          ? "unavailable"
          : newest === null
            ? "silent"
            : nowMs - Date.parse(newest.observedAt) <= SOURCE_LIVE_MS
              ? "live"
              : "stale";
      return {
        source,
        state,
        lastEventAt: newest?.observedAt ?? null,
        lastEventId: newest?.eventId ?? null,
        ageMs: newest ? Math.max(0, nowMs - Date.parse(newest.observedAt)) : null,
        lagMs,
        notices: rows.length,
        missingNotice: this.missing.has(source),
      };
    });

    let state: PushState;
    let detail: string;
    if (this.journalDown) {
      state = "down";
      detail = `freshness journal unreadable${this.journalError ? `: ${this.journalError}` : ""} — push state cannot be proven`;
    } else if (this.lastAttemptAtMs === null) {
      state = "degraded";
      detail = "reconcile has not run yet in this process — push state is unproven, not healthy";
    } else if (this.missing.size > 0) {
      state = "degraded";
      detail = `${this.missing.size} canonical revision(s) have no durable notice yet — synthetic invalidation pending`;
    } else if (this.unavailable.size > 0) {
      state = "degraded";
      detail = `canonical revision unreadable for: ${[...this.unavailable.keys()].join(", ")} — reported unavailable, not treated as unchanged`;
    } else if (this.lastSuccessfulAtMs === null) {
      state = "degraded";
      detail = "no fully durable reconcile yet in this process";
    } else if (nowMs - this.lastSuccessfulAtMs > RECONCILE_STALE_MS) {
      state = "degraded";
      detail = `last successful reconcile is ${Math.round((nowMs - this.lastSuccessfulAtMs) / 60_000)}m old — anti-entropy is off its 5m cadence`;
    } else if (this.lastResetAtMs !== null && this.lastResetAtMs > this.lastSuccessfulAtMs) {
      state = "degraded";
      detail = "journal reset observed since the last successful reconcile";
    } else {
      state = "up";
      detail = "journal readable and the 5m reconcile is on cadence with no open gaps";
    }

    return {
      schemaVersion: 1,
      state,
      detail,
      lastEventAt,
      lastSuccessfulReconcileAt:
        this.lastSuccessfulAtMs === null ? null : new Date(this.lastSuccessfulAtMs).toISOString(),
      lastReconcileAttemptAt: this.lastAttemptAtMs === null ? null : new Date(this.lastAttemptAtMs).toISOString(),
      reconcileIntervalMs: RECONCILE_INTERVAL_MS,
      cursor: this.lastPage?.cursor ?? null,
      sources,
      gaps: {
        open: this.missing.size,
        repairedTotal: this.repairedTotal,
        resetsTotal: this.resetsTotal,
        lastResetAt: this.lastResetAtMs === null ? null : new Date(this.lastResetAtMs).toISOString(),
      },
    };
  }

  /** Start the 5-minute anti-entropy loop. Passes never overlap; the
   *  timer does not hold the process open. */
  start(intervalMs: number = RECONCILE_INTERVAL_MS): void {
    if (this.timer !== null) return;
    this.timer = setInterval(() => {
      if (this.running) return;
      this.running = true;
      void this.reconcile()
        .catch(() => undefined)
        .finally(() => {
          this.running = false;
        });
    }, intervalMs);
    (this.timer as unknown as { unref?: () => void }).unref?.();
  }

  stop(): void {
    if (this.timer !== null) clearInterval(this.timer);
    this.timer = null;
  }
}

/**
 * The fleet-feed canonical reader: the collector publishes the feed
 * file atomically and its source-event notice carries
 * `fleet-feed:<generatedAt>:<sha256 of the file>` (scripts/collect-
 * fleet-feed.ts). The reconciler recomputes exactly that identity from
 * the file on disk — a landed snapshot whose notice was lost is
 * indistinguishable from one whose notice arrived, except here.
 */
export function fleetFeedRevisionReader(
  file: string,
  read: (path: string) => Promise<string> = (path) => readFile(path, "utf8"),
): CanonicalRevisionReader {
  return {
    source: "fleet-feed",
    async read(): Promise<CanonicalRevision | null> {
      const contents = await read(file);
      let generatedAt: unknown;
      try {
        generatedAt = (JSON.parse(contents) as { generatedAt?: unknown }).generatedAt;
      } catch {
        throw new Error("fleet feed file is not valid JSON");
      }
      if (typeof generatedAt !== "string" || parseIsoDate(generatedAt) === null) {
        throw new Error("fleet feed file carries no valid generatedAt");
      }
      const sha256 = createHash("sha256").update(contents, "utf8").digest("hex");
      return {
        source: "fleet-feed",
        eventId: `fleet-feed:${generatedAt}:${sha256}`,
        scope: ["feed"],
        occurredAt: new Date(Date.parse(generatedAt)).toISOString(),
      };
    },
  };
}
