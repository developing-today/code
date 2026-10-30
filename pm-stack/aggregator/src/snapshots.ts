// The usage snapshot store (hub design §4): append-only history for
// usage metrics, one JSONL file per month (`usage-YYYY-MM.jsonl`) in a
// durable directory beside the comments/registry stores. Writers are
// the owning jobs (fleet-feed collector, stewardship-cycle, the Jules
// keeper) and the one-shot backfill script; the aggregator only reads.
//
// Retention (§6, #1117): raw monthly files are kept for 13 months,
// then compacted in place into monthly rollup files
// (`usage-YYYY-MM.rollup.jsonl`) that keep min/max/last per
// service+metric and are kept indefinitely. SnapshotStore.readAll
// expands rollups back into records, so existing readers see a
// compacted store unchanged in shape.
//
// Records keep their original observation timestamp and source class —
// backfilled prose observations stay dated 2026-10-20 even when the
// backfill runs later. Estimates are stored as a low/high pair with
// unit "usd-est"; the 1.2×high counting rule applies at render, never
// baked into history.
import { appendFile, link, mkdir, readdir, readFile, rename, rm, stat, writeFile } from "node:fs/promises";
import { join } from "node:path";
import type { SnapshotRecord, UsageSourceClass } from "./model.ts";

const SOURCES: ReadonlySet<string> = new Set(["api", "console", "manual", "estimate", "invoice"]);
const UNITS: ReadonlySet<string> = new Set(["pct", "usd", "cny", "count", "gb", "tokens", "usd-est", "minutes", "hours", "gb-hours"]);
const FILE_RE = /^usage-(\d{4}-\d{2})\.jsonl$/;

const ROLLUP_RE = /^usage-(\d{4}-\d{2})\.rollup\.jsonl$/;

/** The rollup file a month's raw file compacts into. */
export function rollupFileFor(month: string): string {
  return `usage-${month}.rollup.jsonl`;
}

/**
 * One month's compaction of every record for one
 * service+metric+unit+source: the count of raw records it replaces
 * and their min/max/last values (each with the timestamp it was
 * observed at). Estimate pairs ({low, high}) order by low, then high.
 * `resetsAt`/`limit` are the last non-null values seen, so a quota
 * window's reset effect survives compaction on the `last` record.
 */
export interface SnapshotRollup {
  kind: "snapshot-rollup/v1";
  month: string;
  serviceId: string;
  metricId: string;
  unit: SnapshotRecord["unit"];
  source: SnapshotRecord["source"];
  observedBy: string;
  count: number;
  firstTs: string;
  lastTs: string;
  valueMin: SnapshotRecord["value"];
  minTs: string;
  valueMax: SnapshotRecord["value"];
  maxTs: string;
  valueLast: SnapshotRecord["value"];
  resetsAt?: string | null;
  /** Timestamp of the observation `resetsAt` came from (metadata is
   * merged on its own clock, independently of the last value). */
  resetsAtTs?: string;
  limit?: number | null;
  /** Timestamp of the observation `limit` came from. */
  limitTs?: string;
  /** Dedupe keys (snapshotKey form) of every compacted observation,
   * so SnapshotStore.append stays safe to re-run after compaction. */
  keys: string[];
}

function rollupOrderKey(v: SnapshotRecord["value"]): [number, number] {
  return typeof v === "number" ? [v, v] : [v.low, v.high];
}

function cmpValue(a: SnapshotRecord["value"], b: SnapshotRecord["value"]): number {
  const [al, ah] = rollupOrderKey(a);
  const [bl, bh] = rollupOrderKey(b);
  return al - bl || ah - bh;
}

/** Structural validation for one rollup line (store reads). */
export function isSnapshotRollup(v: unknown): v is SnapshotRollup {
  if (typeof v !== "object" || v === null) return false;
  const r = v as Partial<SnapshotRollup>;
  if (r.kind !== "snapshot-rollup/v1") return false;
  if (typeof r.month !== "string" || !/^\d{4}-\d{2}$/.test(r.month)) return false;
  if (typeof r.serviceId !== "string" || r.serviceId === "") return false;
  if (typeof r.metricId !== "string" || r.metricId === "") return false;
  if (typeof r.unit !== "string" || !UNITS.has(r.unit)) return false;
  if (typeof r.source !== "string" || !SOURCES.has(r.source)) return false;
  if (typeof r.count !== "number" || !(r.count >= 1)) return false;
  if (typeof r.observedBy !== "string" || r.observedBy === "") return false;
  for (const f of [r.firstTs, r.lastTs, r.minTs, r.maxTs]) {
    if (typeof f !== "string" || Number.isNaN(Date.parse(f))) return false;
  }
  if (r.keys !== undefined && (!Array.isArray(r.keys) || r.keys.some((k) => typeof k !== "string"))) return false;
  const val = (x: unknown): boolean =>
    (typeof x === "number" && Number.isFinite(x)) ||
    (typeof x === "object" &&
      x !== null &&
      typeof (x as { low?: unknown }).low === "number" &&
      typeof (x as { high?: unknown }).high === "number");
  return val(r.valueMin) && val(r.valueMax) && val(r.valueLast);
}

interface RollupAgg {
  rollup: SnapshotRollup;
  keys: Set<string>;
}

function aggFromRecord(month: string, r: SnapshotRecord): RollupAgg {
  return {
    keys: new Set([snapshotKey(r)]),
    rollup: {
      kind: "snapshot-rollup/v1",
      month,
      serviceId: r.serviceId,
      metricId: r.metricId,
      unit: r.unit,
      source: r.source,
      observedBy: r.observedBy,
      count: 1,
      firstTs: r.ts,
      lastTs: r.ts,
      valueMin: r.value,
      minTs: r.ts,
      valueMax: r.value,
      maxTs: r.ts,
      valueLast: r.value,
      keys: [],
      ...(r.resetsAt != null ? { resetsAt: r.resetsAt, resetsAtTs: r.ts } : {}),
      ...(r.limit != null ? { limit: r.limit, limitTs: r.ts } : {}),
    },
  };
}

function aggFromRollup(r: SnapshotRollup): RollupAgg {
  const rollup = { ...r };
  // Rollups written before the metadata clocks existed: their
  // metadata came from the last observation by construction.
  if (rollup.resetsAt != null && rollup.resetsAtTs === undefined) rollup.resetsAtTs = rollup.lastTs;
  if (rollup.limit != null && rollup.limitTs === undefined) rollup.limitTs = rollup.lastTs;
  return { rollup, keys: new Set(r.keys ?? []) };
}

function aggAddRecord(agg: RollupAgg, r: SnapshotRecord): void {
  const o = agg.rollup;
  const key = snapshotKey(r);
  // Recovery after an interrupted compaction (rollup published, raw
  // file still present) re-reads records already folded into the
  // rollup; count and aggregates must not grow twice for them.
  if (agg.keys.has(key)) return;
  agg.keys.add(key);
  o.count += 1;
  if (r.ts < o.firstTs) o.firstTs = r.ts;
  if (cmpValue(r.value, o.valueMin) < 0) {
    o.valueMin = r.value;
    o.minTs = r.ts;
  }
  if (cmpValue(r.value, o.valueMax) > 0) {
    o.valueMax = r.value;
    o.maxTs = r.ts;
  }
  if (r.ts >= o.lastTs) {
    o.lastTs = r.ts;
    o.valueLast = r.value;
    o.observedBy = r.observedBy;
  }
  // Quota metadata merges on its own clock: a late record that
  // predates lastTs still contributes its resetsAt/limit when it is
  // the freshest observation carrying them, exactly as a one-pass
  // compaction of the whole month would.
  if (r.resetsAt != null && r.ts >= (o.resetsAtTs ?? "")) {
    o.resetsAt = r.resetsAt;
    o.resetsAtTs = r.ts;
  }
  if (r.limit != null && r.ts >= (o.limitTs ?? "")) {
    o.limit = r.limit;
    o.limitTs = r.ts;
  }
}

function rollupGroupKey(serviceId: string, metricId: string, unit: string, source: string): string {
  return `${serviceId}|${metricId}|${unit}|${source}`;
}

/**
 * Compact one month's records into rollups: one per
 * service+metric+unit+source, sorted for a stable file. Existing
 * rollups for the month seed the aggregates (their counts and
 * min/max/last survive), so re-compacting after late raw records
 * arrive for an already-compacted month merges instead of losing
 * history.
 */
export function compactRecordsToRollups(
  month: string,
  records: SnapshotRecord[],
  existing: SnapshotRollup[] = [],
): SnapshotRollup[] {
  const aggs = new Map<string, RollupAgg>();
  for (const r of existing) {
    aggs.set(rollupGroupKey(r.serviceId, r.metricId, r.unit, r.source), aggFromRollup(r));
  }
  const sorted = [...records].sort((a, b) => a.ts.localeCompare(b.ts));
  for (const r of sorted) {
    const k = rollupGroupKey(r.serviceId, r.metricId, r.unit, r.source);
    const agg = aggs.get(k);
    if (agg) aggAddRecord(agg, r);
    else aggs.set(k, aggFromRecord(month, r));
  }
  return [...aggs.values()]
    .map((a) => ({ ...a.rollup, keys: [...a.keys].sort() }))
    .sort((a, b) =>
      rollupGroupKey(a.serviceId, a.metricId, a.unit, a.source).localeCompare(
        rollupGroupKey(b.serviceId, b.metricId, b.unit, b.source),
      ),
    );
}

/**
 * Expand a rollup back into records for the existing reading path:
 * min, max, and last (deduped when they coincide). The `last` record
 * carries the preserved resetsAt/limit, so latest-value readers and
 * reset markers behave as they did on the raw store.
 */
export function expandRollup(r: SnapshotRollup): SnapshotRecord[] {
  const base = {
    serviceId: r.serviceId,
    metricId: r.metricId,
    unit: r.unit,
    source: r.source,
    observedBy: r.observedBy,
  };
  const out: SnapshotRecord[] = [];
  const seen = new Set<string>();
  const push = (rec: SnapshotRecord): void => {
    const k = snapshotKey(rec);
    if (seen.has(k)) return;
    seen.add(k);
    out.push(rec);
  };
  // Last first: when min or max IS the last observation (same ts and
  // value), the dedupe keeps the record that carries resetsAt/limit.
  push({
    ...base,
    ts: r.lastTs,
    value: r.valueLast,
    ...(r.resetsAt !== undefined ? { resetsAt: r.resetsAt } : {}),
    ...(r.limit !== undefined ? { limit: r.limit } : {}),
  });
  push({ ...base, ts: r.minTs, value: r.valueMin });
  push({ ...base, ts: r.maxTs, value: r.valueMax });
  return out.sort((a, b) => a.ts.localeCompare(b.ts));
}

/** `YYYY-MM` shifted by whole months (negative = earlier). */
export function shiftMonth(month: string, delta: number): string {
  const [y, m] = month.split("-").map(Number);
  const total = y * 12 + (m - 1) + delta;
  const yy = Math.floor(total / 12);
  const mm = (total % 12) + 1;
  return `${yy}-${String(mm).padStart(2, "0")}`;
}

/** Per-service before/after counts for one compacted month. */
export interface CompactionServiceCounts {
  before: number;
  after: number;
}

export interface CompactionMonthReport {
  month: string;
  /** Raw records read from the month's raw file. */
  recordsBefore: number;
  /** Rollup records the month is represented by afterwards. */
  recordsAfter: number;
  perService: Record<string, CompactionServiceCounts>;
  dryRun: boolean;
  /** True when nothing was (or would be) changed for this month. */
  unchanged: boolean;
}

export interface CompactionReport {
  /** Oldest month still kept raw; older months were eligible. */
  cutoffMonth: string;
  months: CompactionMonthReport[];
  dryRun: boolean;
}

/** The monthly file a record belongs in, by its own timestamp. */
export function snapshotFileFor(ts: string): string {
  return `usage-${ts.slice(0, 7)}.jsonl`;
}

/** Structural validation for one untrusted record (store reads, writer input). */
export function isSnapshotRecord(v: unknown): v is SnapshotRecord {
  if (typeof v !== "object" || v === null) return false;
  const r = v as Partial<SnapshotRecord>;
  if (typeof r.ts !== "string" || Number.isNaN(Date.parse(r.ts))) return false;
  if (typeof r.serviceId !== "string" || r.serviceId === "") return false;
  if (typeof r.metricId !== "string" || r.metricId === "") return false;
  if (typeof r.unit !== "string" || !UNITS.has(r.unit)) return false;
  if (typeof r.source !== "string" || !SOURCES.has(r.source)) return false;
  if (typeof r.observedBy !== "string" || r.observedBy === "") return false;
  if (typeof r.value === "number") return Number.isFinite(r.value);
  if (typeof r.value === "object" && r.value !== null) {
    const p = r.value as { low?: unknown; high?: unknown };
    return typeof p.low === "number" && typeof p.high === "number";
  }
  return false;
}

/** Dedupe key: same service, metric, instant, and value = same observation. */
export function snapshotKey(r: SnapshotRecord): string {
  const v = typeof r.value === "number" ? String(r.value) : `${r.value.low}/${r.value.high}`;
  return `${r.serviceId}|${r.metricId}|${r.ts}|${v}`;
}

const LOCK_STALE_MS = 120_000;
const LOCK_TIMEOUT_MS = 30_000;

/**
 * Serialize store mutations (append, compact) across processes with
 * a lockfile in the store dir: compaction's read/publish/delete
 * sequence must never overlap an append to the same month, or the
 * appended records would be deleted without entering the rollup.
 * A lock older than LOCK_STALE_MS is treated as abandoned (its
 * holder died) and stolen; failing to acquire within
 * LOCK_TIMEOUT_MS is an error, never a silent unlocked write.
 */
export async function withSnapshotStoreLock<T>(dir: string, fn: () => Promise<T>): Promise<T> {
  await mkdir(dir, { recursive: true });
  const lockPath = join(dir, ".snapshots.lock");
  const guardPath = join(dir, ".snapshots.reclaim.guard");
  const token = `${process.pid}:${Date.now()}:${Math.random().toString(36).slice(2)}`;
  const deadline = Date.now() + LOCK_TIMEOUT_MS;

  // Acquire the reclaim guard: a plain wx lock file whose critical
  // sections are milliseconds long. Its own staleness uses the
  // rename-and-verify reclaim below, which is safe here because the
  // guard only ever serializes the *decision* to reclaim — the main
  // lock is re-verified inside the guard before anything is removed.
  //
  // Restore protocol (Codex round 3 on PR #2379): a reclaimer that
  // renames the guard may find it moved a *live* guard (a faster
  // reclaimer had already replaced the abandoned one). The moved
  // guard is linked back — but if its owner released it while it was
  // moved aside, the restored file would be an ownerless orphan and
  // waiters would stall until it went stale. So releaseGuard leaves
  // a release tombstone whenever it cannot remove its own guard
  // file, and the restorer (and, as a backstop, any later acquirer
  // that finds a tombstoned guard token) deletes a restored guard
  // whose owner has tombstoned its release. A guard is only ever
  // deleted under its owner's token, and a tombstone is only ever
  // written by that token's owner at release time.
  const releasedPathFor = (t: string): string =>
    `${guardPath}.released-${t.replace(/[^a-zA-Z0-9.-]/g, "_")}`;
  const restoreGuard = async (reclaimPath: string): Promise<void> => {
    const movedToken = await readFile(reclaimPath, "utf8").catch(() => "");
    try {
      await link(reclaimPath, guardPath);
    } catch {
      await rm(reclaimPath, { force: true });
      return; // a guard is already in place — the moved file is superseded
    }
    await rm(reclaimPath, { force: true });
    if (!movedToken) return;
    // Poll briefly for the owner's release tombstone: the owner
    // releases the guard immediately after its (millisecond) critical
    // section, so a tombstone appears promptly or not at all — in
    // which case the owner is still holding the restored guard and
    // will remove it itself at release.
    for (let i = 0; i < 200; i++) {
      const cur = await readFile(guardPath, "utf8").catch(() => "");
      if (cur !== movedToken) return; // released or replaced — not ours to touch
      const tomb = await stat(releasedPathFor(movedToken)).catch(() => null);
      const lockToken = await readFile(lockPath, "utf8").catch(() => "");
      if (tomb || lockToken === movedToken) {
        // Tombstone: the owner released the guard while it was moved.
        // Main-lock token: the owner uses the same token there and
        // creates it immediately after releasing the guard, so its
        // presence proves the same release. Either way the restored
        // guard is ownerless.
        const again = await readFile(guardPath, "utf8").catch(() => "");
        if (again === movedToken) await rm(guardPath, { force: true });
        await rm(releasedPathFor(movedToken), { force: true });
        return;
      }
      await new Promise((r) => setTimeout(r, 5));
    }
  };
  const acquireGuard = async (): Promise<boolean> => {
    for (;;) {
      try {
        await writeFile(guardPath, token, { flag: "wx" });
        return true;
      } catch (err) {
        if ((err as NodeJS.ErrnoException).code !== "EEXIST") throw err;
        // Backstop orphan sweep: a guard whose owner tombstoned its
        // release is ownerless (see restore protocol above) — remove
        // it instead of waiting for it to go stale.
        const heldToken = await readFile(guardPath, "utf8").catch(() => "");
        if (heldToken && heldToken !== token) {
          const tomb = await stat(releasedPathFor(heldToken)).catch(() => null);
          if (tomb) {
            const again = await readFile(guardPath, "utf8").catch(() => "");
            if (again === heldToken) await rm(guardPath, { force: true });
            await rm(releasedPathFor(heldToken), { force: true });
            continue;
          }
        }
        let st;
        try {
          st = await stat(guardPath);
        } catch (statErr) {
          if ((statErr as NodeJS.ErrnoException).code === "ENOENT") continue;
          throw statErr;
        }
        if (Date.now() - st.mtimeMs > LOCK_STALE_MS) {
          const reclaimPath = `${guardPath}.reclaim-${token}`;
          try {
            await rename(guardPath, reclaimPath);
          } catch (renameErr) {
            if ((renameErr as NodeJS.ErrnoException).code === "ENOENT") continue;
            throw renameErr;
          }
          const moved = await stat(reclaimPath).catch(() => null);
          if (moved && moved.ino === st.ino && moved.mtimeMs === st.mtimeMs) {
            await rm(reclaimPath, { force: true });
          } else if (moved) {
            await restoreGuard(reclaimPath);
          }
          continue;
        }
        if (Date.now() > deadline) return false; // another reclaimer is at work — just wait for the lock instead
        await new Promise((r) => setTimeout(r, 25));
      }
    }
  };
  const releaseGuard = async (): Promise<void> => {
    const cur = await readFile(guardPath, "utf8").catch(() => "");
    if (cur === token) {
      await rm(guardPath, { force: true });
    } else {
      // Our guard file was moved aside by a stale reclaimer (or is
      // already gone): leave a release tombstone so whoever restores
      // the moved file can tell it is an orphan, not a held guard.
      await writeFile(releasedPathFor(token), token).catch(() => undefined);
    }
  };

  for (;;) {
    try {
      await writeFile(lockPath, token, { flag: "wx" });
      break;
    } catch (err) {
      // Only EEXIST means contention. Anything else (EACCES on an
      // unwritable store dir, ENOSPC, ...) is permanent and must
      // surface instead of spinning past the deadline.
      if ((err as NodeJS.ErrnoException).code !== "EEXIST") throw err;
      let st;
      try {
        st = await stat(lockPath);
      } catch (statErr) {
        if ((statErr as NodeJS.ErrnoException).code === "ENOENT") continue; // lock vanished between attempts — retry at once
        throw statErr;
      }
      if (Date.now() - st.mtimeMs > LOCK_STALE_MS) {
        if (Date.now() > deadline) throw new Error(`snapshot store lock timeout: ${lockPath}`);
        // The lock looks abandoned. Reclaimers are serialized by the
        // guard, and staleness is re-verified *inside* the guard
        // against the file identity statted there: a lock is removed
        // only while no other reclaimer can be replacing it, and
        // plain acquirers cannot create a lock while the stale one
        // is present — so the file removed is always the abandoned
        // one, never a fresh replacement. (Unserialized, a slower
        // reclaimer's rm deleted the fresh lock a faster reclaimer
        // had just acquired and both critical sections ran at once.)
        if (await acquireGuard()) {
          try {
            const cur = await stat(lockPath).catch(() => null);
            if (cur && cur.ino === st.ino && Date.now() - cur.mtimeMs > LOCK_STALE_MS) {
              await rm(lockPath, { force: true });
            }
          } finally {
            await releaseGuard();
          }
        }
        continue;
      }
      if (Date.now() > deadline) throw new Error(`snapshot store lock timeout: ${lockPath}`);
      await new Promise((r) => setTimeout(r, 25));
    }
  }
  try {
    return await fn();
  } finally {
    const cur = await readFile(lockPath, "utf8").catch(() => "");
    if (cur === token) await rm(lockPath, { force: true });
    // Final orphan sweep: the guard is only ever held transiently
    // during a reclaim, long before fn() ran, so a guard file still
    // bearing our token here is a restored orphan (see the restore
    // protocol above) — remove it and our release tombstone. If no
    // restorer consumed the tombstone and no guard was restored,
    // this just removes the tombstone.
    const guardCur = await readFile(guardPath, "utf8").catch(() => "");
    if (guardCur === token) await rm(guardPath, { force: true });
    await rm(releasedPathFor(token), { force: true });
  }
}

export class SnapshotStore {
  constructor(private readonly dir: string) {}

  /** Every valid record in the store, oldest first. Malformed lines are skipped. */
  async readAll(): Promise<SnapshotRecord[]> {
    let names: string[];
    try {
      names = (await readdir(this.dir)).filter((n) => FILE_RE.test(n) || ROLLUP_RE.test(n)).sort();
    } catch {
      return []; // no store yet — history simply has not started
    }
    const out: SnapshotRecord[] = [];
    for (const name of names) {
      let text: string;
      try {
        text = await readFile(join(this.dir, name), "utf8");
      } catch {
        continue;
      }
      const isRollup = ROLLUP_RE.test(name);
      for (const line of text.split("\n")) {
        if (!line.trim()) continue;
        try {
          const parsed: unknown = JSON.parse(line);
          if (isRollup) {
            if (isSnapshotRollup(parsed)) out.push(...expandRollup(parsed));
          } else if (isSnapshotRecord(parsed)) {
            out.push(parsed);
          }
        } catch {
          // one bad line never poisons the store
        }
      }
    }
    return out.sort((a, b) => a.ts.localeCompare(b.ts));
  }

  /** History grouped by service id, oldest first per service. */
  async byService(): Promise<Record<string, SnapshotRecord[]>> {
    const grouped: Record<string, SnapshotRecord[]> = {};
    for (const r of await this.readAll()) {
      (grouped[r.serviceId] ??= []).push(r);
    }
    return grouped;
  }

  /** Dedupe keys of every observation already compacted into rollups. */
  async rollupKeys(): Promise<string[]> {
    let names: string[];
    try {
      names = (await readdir(this.dir)).filter((n) => ROLLUP_RE.test(n)).sort();
    } catch {
      return [];
    }
    const out: string[] = [];
    for (const name of names) {
      const text = await readFile(join(this.dir, name), "utf8").catch(() => "");
      for (const line of text.split("\n")) {
        if (!line.trim()) continue;
        try {
          const parsed: unknown = JSON.parse(line);
          if (isSnapshotRollup(parsed)) out.push(...(parsed.keys ?? []));
        } catch {
          // one bad line never poisons the store
        }
      }
    }
    return out;
  }

  /**
   * Append records, skipping any already present (same dedupe key), so
   * writers and the backfill are safe to re-run. Returns how many were
   * actually appended. Records land in their own month's file.
   * Dedupe consults rollup keys too: observations compacted out of
   * the raw files stay known, so replaying a historical batch after
   * compaction appends nothing.
   */
  async append(records: SnapshotRecord[]): Promise<number> {
    return withSnapshotStoreLock(this.dir, () => this.appendLocked(records));
  }

  private async appendLocked(records: SnapshotRecord[]): Promise<number> {
    const valid = records.filter(isSnapshotRecord);
    if (!valid.length) return 0;
    const seen = new Set((await this.readAll()).map(snapshotKey));
    for (const k of await this.rollupKeys()) seen.add(k);
    const fresh = valid.filter((r) => {
      const k = snapshotKey(r);
      if (seen.has(k)) return false; // stored already, or earlier in this batch
      seen.add(k);
      return true;
    });
    if (!fresh.length) return 0;
    await mkdir(this.dir, { recursive: true });
    const byFile = new Map<string, SnapshotRecord[]>();
    for (const r of fresh) {
      const f = snapshotFileFor(r.ts);
      byFile.set(f, [...(byFile.get(f) ?? []), r]);
    }
    for (const [file, rows] of byFile) {
      const lines = rows.map((r) => JSON.stringify(r)).join("\n") + "\n";
      await appendFile(join(this.dir, file), lines, "utf8");
    }
    return fresh.length;
  }

  /**
   * Retention (§6, #1117): compact every raw monthly file older
   * than `retentionMonths` (default 13) into its rollup file, then
   * remove the raw file. Months at or newer than the cutoff stay
   * raw. Idempotent: a month with no raw file is skipped, and a
   * month whose raw file reappears (late writer) merges into the
   * existing rollup on the next run. With `dryRun`, the same report
   * is computed and nothing on disk changes.
   */
  async compact(opts: { now?: Date; retentionMonths?: number; dryRun?: boolean } = {}): Promise<CompactionReport> {
    if (opts.dryRun) return this.compactInner(opts);
    return withSnapshotStoreLock(this.dir, () => this.compactInner(opts));
  }

  private async compactInner(opts: { now?: Date; retentionMonths?: number; dryRun?: boolean }): Promise<CompactionReport> {
    const now = opts.now ?? new Date();
    const retentionMonths = opts.retentionMonths ?? 13;
    const dryRun = opts.dryRun ?? false;
    const currentMonth = now.toISOString().slice(0, 7);
    const cutoffMonth = shiftMonth(currentMonth, -retentionMonths);
    const report: CompactionReport = { cutoffMonth, months: [], dryRun };
    let names: string[];
    try {
      names = (await readdir(this.dir)).sort();
    } catch {
      return report; // no store yet — nothing to compact
    }
    for (const name of names) {
      const m = FILE_RE.exec(name);
      if (!m) continue;
      const month = m[1];
      if (month > cutoffMonth) continue; // still within the raw window
      const rawText = await readFile(join(this.dir, name), "utf8").catch(() => "");
      const records: SnapshotRecord[] = [];
      for (const line of rawText.split("\n")) {
        if (!line.trim()) continue;
        try {
          const parsed: unknown = JSON.parse(line);
          if (isSnapshotRecord(parsed)) records.push(parsed);
        } catch {
          // malformed lines are dropped by compaction, as reads skip them
        }
      }
      const rollupName = rollupFileFor(month);
      const existing: SnapshotRollup[] = [];
      if (names.includes(rollupName)) {
        // An unreadable rollup must abort compaction: treating it as
        // empty would overwrite previously compacted history with only
        // the late raw records, and the raw file is deleted right after.
        let rollText: string;
        try {
          rollText = await readFile(join(this.dir, rollupName), "utf8");
        } catch (err) {
          if ((err as NodeJS.ErrnoException).code === "ENOENT") {
            rollText = "";
          } else {
            throw new Error(
              `compaction aborted: cannot read existing rollup ${rollupName}: ${(err as Error).message}`,
            );
          }
        }
        for (const line of rollText.split("\n")) {
          if (!line.trim()) continue;
          try {
            const parsed: unknown = JSON.parse(line);
            if (isSnapshotRollup(parsed)) existing.push(parsed);
          } catch {
            // a bad rollup line never blocks compaction of the raw file
          }
        }
      }
      const rollups = compactRecordsToRollups(month, records, existing);
      const perService: Record<string, CompactionServiceCounts> = {};
      for (const r of records) {
        (perService[r.serviceId] ??= { before: 0, after: 0 }).before += 1;
      }
      for (const r of existing) {
        (perService[r.serviceId] ??= { before: 0, after: 0 }).before += r.count;
      }
      for (const r of rollups) {
        (perService[r.serviceId] ??= { before: 0, after: 0 }).after += 1;
      }
      const monthReport: CompactionMonthReport = {
        month,
        recordsBefore: records.length + existing.reduce((n, r) => n + r.count, 0),
        recordsAfter: rollups.length,
        perService,
        dryRun,
        unchanged: records.length === 0,
      };
      report.months.push(monthReport);
      if (dryRun || records.length === 0) continue;
      const tmp = join(this.dir, `.${rollupName}.tmp`);
      await writeFile(tmp, rollups.map((r) => JSON.stringify(r)).join("\n") + "\n", "utf8");
      await rename(tmp, join(this.dir, rollupName));
      await rm(join(this.dir, name));
    }
    return report;
  }
}

/** Source classes the store accepts, for writer-side validation messages. */
export const SNAPSHOT_SOURCES: readonly UsageSourceClass[] = ["api", "console", "manual", "estimate", "invoice"];
