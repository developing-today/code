// Spend rollup store (#4823, slice 2 of the #1259 spend-rollups design,
// doc/spend-rollups-design-1259.md): calendar-week and calendar-month
// rollups per project and per epic, built from source events that carry
// their own timestamps (source event time, never processing time).
//
// Invariants (from the design note, enforced here and in tests):
// - Three unit families are stored as distinct measures, never merged:
//   tokens per pool, matched PAYG dollars, plan-pool share-of-window.
//   There is no cross-unit total field anywhere in the schema, and the
//   validator rejects records carrying one.
// - A window is labelled complete only when an ok source-coverage span
//   covers the whole window; otherwise records are labelled partial.
//   Windows with no ok coverage and no events emit nothing at all —
//   incomplete/failed/unknown coverage is never emitted as zero spend.
// - Every record names its coverageStart: the earliest retained source
//   history, so a short history cannot imply a longer backfill horizon.
// - Rollups are immutable per attributionVersion. Restatement writes a
//   new version alongside the old records; the store refuses to append
//   a changed record under an existing (key, version).
import { appendFile, mkdir, readFile } from "node:fs/promises";
import { join } from "node:path";
import { withSnapshotStoreLock } from "./snapshots.ts";

export type RollupWindowKind = "week" | "month";
export type RollupScopeKind = "project" | "epic" | "unattributed";
export type RollupUnit = "tokens" | "usd" | "share_pct";
export type RollupCoverage = "complete" | "partial";

export interface SpendRollupEvent {
  /** Source event timestamp (ISO); windows are derived from this. */
  ts: string;
  projectId: string | null;
  epicRef: string | null;
  /** Token consumption per pool (pool names are the unit's scope). */
  tokensByPool?: Readonly<Record<string, number>>;
  /** Matched PAYG charges only. null/absent = unknown, never zero. */
  paygUsd?: number | null;
  /** Plan-pool share-of-window consumed, per pool, in percent. */
  sharePctByPool?: Readonly<Record<string, number>>;
}

export interface SourceCoverageSpan {
  from: string;
  to: string;
  status: "ok" | "failed" | "unknown";
}

export interface SpendRollup {
  kind: "spend-rollup/v1";
  windowKind: RollupWindowKind;
  /** Inclusive window start (UTC, ISO). */
  windowStart: string;
  /** Exclusive window end (UTC, ISO). */
  windowEnd: string;
  scopeKind: RollupScopeKind;
  /** Project id, epic ref, or "unattributed". */
  scopeId: string;
  unit: RollupUnit;
  /** Pool for tokens / share_pct measures; null for usd. */
  pool: string | null;
  value: number;
  coverage: RollupCoverage;
  /** Earliest retained source history (ISO date/timestamp). */
  coverageStart: string;
  attributionVersion: string;
}

const ROLLUP_KEYS: ReadonlySet<string> = new Set([
  "kind",
  "windowKind",
  "windowStart",
  "windowEnd",
  "scopeKind",
  "scopeId",
  "unit",
  "pool",
  "value",
  "coverage",
  "coverageStart",
  "attributionVersion",
]);

export function rollupKey(r: SpendRollup): string {
  return [r.windowKind, r.windowStart, r.scopeKind, r.scopeId, r.unit, r.pool ?? "", r.attributionVersion].join("|");
}

function versionRank(v: string): number {
  const m = /^v(\d+)$/.exec(v);
  return m ? Number(m[1]) : 0;
}

/** Validate one rollup record; throws on any schema violation. A record
 * carrying a merged cross-unit field (any key outside the allowlist —
 * e.g. `totalSpend`, `usdEquivalent`) is rejected here. */
export function validateSpendRollup(r: unknown): asserts r is SpendRollup {
  if (typeof r !== "object" || r === null) throw new Error("spend rollup: not an object");
  for (const k of Object.keys(r)) {
    if (!ROLLUP_KEYS.has(k))
      throw new Error(`spend rollup: unexpected field "${k}" (merged cross-unit fields are banned)`);
  }
  const v = r as Record<string, unknown>;
  if (v.kind !== "spend-rollup/v1") throw new Error("spend rollup: bad kind");
  if (v.windowKind !== "week" && v.windowKind !== "month") throw new Error("spend rollup: bad windowKind");
  if (v.scopeKind !== "project" && v.scopeKind !== "epic" && v.scopeKind !== "unattributed")
    throw new Error("spend rollup: bad scopeKind");
  if (v.unit !== "tokens" && v.unit !== "usd" && v.unit !== "share_pct") throw new Error("spend rollup: bad unit");
  if (v.unit === "usd" && v.pool !== null) throw new Error("spend rollup: usd measure must not name a pool");
  if (v.unit !== "usd" && (typeof v.pool !== "string" || v.pool.length === 0))
    throw new Error("spend rollup: tokens/share_pct measures must name a pool");
  if (typeof v.value !== "number" || !Number.isFinite(v.value) || v.value < 0)
    throw new Error("spend rollup: value must be a finite non-negative number");
  if (v.coverage !== "complete" && v.coverage !== "partial") throw new Error("spend rollup: bad coverage");
  for (const f of ["windowStart", "windowEnd", "scopeId", "coverageStart", "attributionVersion"] as const) {
    if (typeof v[f] !== "string" || (v[f] as string).length === 0) throw new Error(`spend rollup: bad ${f}`);
  }
}

/** UTC calendar window [start, end) containing `ts`. Weeks start Monday. */
export function windowRange(kind: RollupWindowKind, ts: string): { start: string; end: string } {
  const d = new Date(ts);
  if (Number.isNaN(d.getTime())) throw new Error(`spend rollup: bad timestamp ${ts}`);
  const start = new Date(Date.UTC(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate()));
  if (kind === "month") {
    start.setUTCDate(1);
    const end = new Date(start);
    end.setUTCMonth(end.getUTCMonth() + 1);
    return { start: start.toISOString(), end: end.toISOString() };
  }
  const dow = (start.getUTCDay() + 6) % 7; // Monday = 0
  start.setUTCDate(start.getUTCDate() - dow);
  const end = new Date(start);
  end.setUTCDate(end.getUTCDate() + 7);
  return { start: start.toISOString(), end: end.toISOString() };
}

export interface BuildRollupsOptions {
  attributionVersion: string;
  /** Earliest retained source history; stamped on every record. */
  coverageStart: string;
  coverage: readonly SourceCoverageSpan[];
}

interface Group {
  windowKind: RollupWindowKind;
  windowStart: string;
  windowEnd: string;
  scopeKind: RollupScopeKind;
  scopeId: string;
  tokens: Map<string, number>;
  share: Map<string, number>;
  usdValues: number[];
  eventCount: number;
}

/** Build rollup records from source events. Pure: the same inputs always
 * produce the same records, which is what makes restatement auditable. */
export function buildSpendRollups(events: readonly SpendRollupEvent[], opts: BuildRollupsOptions): SpendRollup[] {
  const groups = new Map<string, Group>();
  const addMeasure = (map: Map<string, number>, pool: string, value: number) => {
    if (!Number.isFinite(value) || value < 0) throw new Error(`spend rollup: bad measure value ${value}`);
    map.set(pool, (map.get(pool) ?? 0) + value);
  };
  for (const ev of events) {
    const scopes: Array<[RollupScopeKind, string]> = [];
    if (ev.projectId) scopes.push(["project", ev.projectId]);
    if (ev.epicRef) scopes.push(["epic", ev.epicRef]);
    if (scopes.length === 0) scopes.push(["unattributed", "unattributed"]);
    for (const kind of ["week", "month"] as const) {
      const { start, end } = windowRange(kind, ev.ts);
      for (const [scopeKind, scopeId] of scopes) {
        const key = [kind, start, scopeKind, scopeId].join("|");
        let g = groups.get(key);
        if (!g) {
          g = {
            windowKind: kind,
            windowStart: start,
            windowEnd: end,
            scopeKind,
            scopeId,
            tokens: new Map(),
            share: new Map(),
            usdValues: [],
            eventCount: 0,
          };
          groups.set(key, g);
        }
        g.eventCount += 1;
        for (const [pool, v] of Object.entries(ev.tokensByPool ?? {})) addMeasure(g.tokens, pool, v);
        for (const [pool, v] of Object.entries(ev.sharePctByPool ?? {})) addMeasure(g.share, pool, v);
        if (typeof ev.paygUsd === "number") g.usdValues.push(ev.paygUsd);
      }
    }
  }
  const coverageFor = (g: Group): RollupCoverage => {
    // Compare coverage endpoints as instants, never as strings: ISO
    // representations differ ("2026-11-02T00:00:00Z" vs ".000Z", offsets)
    // for the same moment.
    const ws = Date.parse(g.windowStart);
    const we = Date.parse(g.windowEnd);
    const full = opts.coverage.some((s) => {
      if (s.status !== "ok") return false;
      const from = Date.parse(s.from);
      const to = Date.parse(s.to);
      return Number.isFinite(from) && Number.isFinite(to) && from <= ws && to >= we;
    });
    return full ? "complete" : "partial";
  };
  const out: SpendRollup[] = [];
  for (const g of groups.values()) {
    const coverage = coverageFor(g);
    const base = {
      kind: "spend-rollup/v1" as const,
      windowKind: g.windowKind,
      windowStart: g.windowStart,
      windowEnd: g.windowEnd,
      scopeKind: g.scopeKind,
      scopeId: g.scopeId,
      coverage,
      coverageStart: opts.coverageStart,
      attributionVersion: opts.attributionVersion,
    };
    for (const [pool, value] of [...g.tokens.entries()].sort()) out.push({ ...base, unit: "tokens", pool, value });
    for (const [pool, value] of [...g.share.entries()].sort()) out.push({ ...base, unit: "share_pct", pool, value });
    // USD is emitted only when every event in the group carried a matched
    // charge figure; a group with any unknown charge state emits no USD
    // record at all (unknown is not zero, and a partial sum would lie).
    if (g.usdValues.length === g.eventCount && g.eventCount > 0)
      out.push({ ...base, unit: "usd", pool: null, value: g.usdValues.reduce((a, b) => a + b, 0) });
  }
  out.sort((a, b) => rollupKey(a).localeCompare(rollupKey(b)));
  for (const r of out) validateSpendRollup(r);
  return out;
}

/** Append-only JSONL rollup store, mirroring the snapshots.ts pattern. */
export class SpendRollupStore {
  readonly file: string;
  constructor(private dir: string) {
    this.file = join(dir, "spend-rollups.jsonl");
  }

  async readAll(): Promise<SpendRollup[]> {
    let text: string;
    try {
      text = await readFile(this.file, "utf8");
    } catch (e) {
      // Only a missing ledger is an empty store. Any other read failure
      // must propagate: append's duplicate/conflict check runs against
      // this history, and silently treating an unreadable ledger as
      // empty would defeat the immutability guarantee.
      if ((e as NodeJS.ErrnoException).code === "ENOENT") return [];
      throw e;
    }
    const out: SpendRollup[] = [];
    for (const line of text.split("\n")) {
      if (!line.trim()) continue;
      const parsed: unknown = JSON.parse(line);
      validateSpendRollup(parsed);
      out.push(parsed);
    }
    return out;
  }

  /** Append records. Re-appending an identical record is a no-op;
   * appending a changed record under an existing (key, version) throws —
   * restate under a new attributionVersion instead. The duplicate check
   * and the append are serialized under the store lock (the same lock
   * the snapshot store uses), so overlapping appends cannot both pass
   * the check before either writes. */
  async append(records: readonly SpendRollup[]): Promise<{ appended: number; skippedIdentical: number }> {
    return withSnapshotStoreLock(this.dir, () => this.appendLocked(records));
  }

  private async appendLocked(records: readonly SpendRollup[]): Promise<{ appended: number; skippedIdentical: number }> {
    for (const r of records) validateSpendRollup(r);
    const existing = new Map((await this.readAll()).map((r) => [rollupKey(r), r]));
    const lines: string[] = [];
    let skippedIdentical = 0;
    for (const r of records) {
      const prev = existing.get(rollupKey(r));
      if (prev) {
        if (prev.value === r.value && prev.coverage === r.coverage && prev.coverageStart === r.coverageStart) {
          skippedIdentical += 1;
          continue;
        }
        throw new Error(
          `spend rollup: ${rollupKey(r)} already stored with a different value/coverage; rollups are immutable per attributionVersion — restate as a new version`,
        );
      }
      existing.set(rollupKey(r), r);
      lines.push(JSON.stringify(r));
    }
    if (lines.length > 0) {
      await mkdir(this.dir, { recursive: true });
      await appendFile(this.file, lines.join("\n") + "\n", "utf8");
    }
    return { appended: lines.length, skippedIdentical };
  }
}

/** Reader view: one CONSISTENT attribution version per window — the
 * newest version that has any record for that window (or the requested
 * version only). Selecting the newest version per key instead would
 * mix versions inside one window: a restatement that moves spend from
 * `unattributed` to a project would leave the old unattributed record
 * in place next to the new project record, double-counting the window.
 * Old versions stay stored and inspectable via an explicit version. */
export function selectRollups(records: readonly SpendRollup[], version?: string): SpendRollup[] {
  const pool = version !== undefined ? records.filter((r) => r.attributionVersion === version) : records;
  const newestByWindow = new Map<string, string>();
  for (const r of pool) {
    const w = [r.windowKind, r.windowStart].join("|");
    const prev = newestByWindow.get(w);
    if (prev === undefined || versionRank(r.attributionVersion) > versionRank(prev))
      newestByWindow.set(w, r.attributionVersion);
  }
  return pool
    .filter((r) => r.attributionVersion === newestByWindow.get([r.windowKind, r.windowStart].join("|")))
    .sort((a, b) => rollupKey(a).localeCompare(rollupKey(b)));
}
