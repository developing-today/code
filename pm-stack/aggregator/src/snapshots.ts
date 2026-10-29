// The usage snapshot store (hub design §4): append-only history for
// usage metrics, one JSONL file per month (`usage-YYYY-MM.jsonl`) in a
// durable directory beside the comments/registry stores. Writers are
// the owning jobs (fleet-feed collector, stewardship-cycle, the Jules
// keeper) and the one-shot backfill script; the aggregator only reads.
//
// Records keep their original observation timestamp and source class —
// backfilled prose observations stay dated 2026-10-20 even when the
// backfill runs later. Estimates are stored as a low/high pair with
// unit "usd-est"; the 1.2×high counting rule applies at render, never
// baked into history.
import { appendFile, mkdir, readdir, readFile } from "node:fs/promises";
import { join } from "node:path";
import type { SnapshotRecord, UsageSourceClass } from "./model.ts";

const SOURCES: ReadonlySet<string> = new Set(["api", "console", "manual", "estimate", "invoice"]);
const UNITS: ReadonlySet<string> = new Set(["pct", "usd", "count", "gb", "tokens", "usd-est", "minutes", "hours", "gb-hours"]);
const FILE_RE = /^usage-(\d{4}-\d{2})\.jsonl$/;

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

export class SnapshotStore {
  constructor(private readonly dir: string) {}

  /** Every valid record in the store, oldest first. Malformed lines are skipped. */
  async readAll(): Promise<SnapshotRecord[]> {
    let names: string[];
    try {
      names = (await readdir(this.dir)).filter((n) => FILE_RE.test(n)).sort();
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
      for (const line of text.split("\n")) {
        if (!line.trim()) continue;
        try {
          const parsed: unknown = JSON.parse(line);
          if (isSnapshotRecord(parsed)) out.push(parsed);
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

  /**
   * Append records, skipping any already present (same dedupe key), so
   * writers and the backfill are safe to re-run. Returns how many were
   * actually appended. Records land in their own month's file.
   */
  async append(records: SnapshotRecord[]): Promise<number> {
    const valid = records.filter(isSnapshotRecord);
    if (!valid.length) return 0;
    const seen = new Set((await this.readAll()).map(snapshotKey));
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
}

/** Source classes the store accepts, for writer-side validation messages. */
export const SNAPSHOT_SOURCES: readonly UsageSourceClass[] = ["api", "console", "manual", "estimate", "invoice"];
