// Project #4 coverage report (#1220): pool vs backlog reality.
//
// Backlog counts (open issues per repo, `now`/`next` gate labels) cannot
// come from the aggregator's item resolution — the Source only resolves
// curated refs, and the fleet gh token cannot read the Project at all
// (missing read:project). They come from the fleet-feed collector, which
// already holds a GitHub token for the Project read: it censuses the
// four fleet repos through the Search API and embeds a `coverage`
// section in the feed. Pool counts (items, unset Status/Area) are
// computed here from the Project pool the collector already embeds.
//
// A weekly snapshot is stored beside the other aggregator stores so the
// panel can show week-over-week delta: the file holds { baseline,
// current }; when `current` is a week old or older it rotates into
// `baseline` on the next read. All computation is pure — the store is a
// thin JSON file wrapper, and a missing/corrupt file reads as null.
// When the feed carries no census (collector had no token), backlog
// fields are null and the panel renders them as unavailable rather
// than inventing zeros.
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname } from "node:path";
import type { ProjectPoolItem } from "./model.ts";

/** One repo's backlog census, as embedded by the collector. */
export interface RepoCensus {
  repo: string;
  /** Open issues in the repo (Search API total_count, is:issue is:open). */
  openIssues: number;
  /** Open issues carrying the `now` gate label. */
  gateNow: number;
  /** Open issues carrying the `next` gate label. */
  gateNext: number;
}

export interface RepoCoverage extends Omit<RepoCensus, "openIssues" | "gateNow" | "gateNext"> {
  /** Null when the feed carried no census for this repo. */
  openIssues: number | null;
  gateNow: number | null;
  gateNext: number | null;
  /** Project #4 pool items whose linked issue/PR lives in this repo. */
  poolItems: number;
  /** Pool items (this repo) with no Project Status set. */
  poolUnsetStatus: number;
  /** Pool items (this repo) with no Project Area set. */
  poolUnsetArea: number;
}

export interface CoverageReport {
  generatedAt: string;
  repos: RepoCoverage[];
}

export interface CoverageSnapshotFile {
  baseline: CoverageReport | null;
  current: CoverageReport;
}

export const COVERAGE_WEEK_MS = 7 * 24 * 60 * 60 * 1000;

/** Structural gate for one census row, shared by the feed reader. */
export function isRepoCensus(v: unknown): v is RepoCensus {
  if (!v || typeof v !== "object") return false;
  const r = v as Record<string, unknown>;
  return (
    typeof r.repo === "string" &&
    r.repo.includes("/") &&
    Number.isInteger(r.openIssues) &&
    Number.isInteger(r.gateNow) &&
    Number.isInteger(r.gateNext)
  );
}

/**
 * Coverage from the collector's census plus the Project pool. Pure.
 * Repos are the union of what the census and the pool actually carry,
 * sorted by repo name. Pool-side counts are always real; backlog fields
 * stay null for repos the census did not cover.
 */
export function computeCoverage(census: RepoCensus[] | null, pool: ProjectPoolItem[], now = new Date()): CoverageReport {
  const byRepo = new Map<string, RepoCoverage>();
  const row = (repo: string): RepoCoverage => {
    let r = byRepo.get(repo);
    if (!r) {
      r = { repo, openIssues: null, gateNow: null, gateNext: null, poolItems: 0, poolUnsetStatus: 0, poolUnsetArea: 0 };
      byRepo.set(repo, r);
    }
    return r;
  };
  for (const c of census ?? []) {
    const r = row(c.repo);
    r.openIssues = c.openIssues;
    r.gateNow = c.gateNow;
    r.gateNext = c.gateNext;
  }
  for (const p of pool) {
    const r = row(p.repo);
    r.poolItems += 1;
    if (!p.status) r.poolUnsetStatus += 1;
    if (!p.area) r.poolUnsetArea += 1;
  }
  return {
    generatedAt: now.toISOString(),
    repos: [...byRepo.values()].sort((a, b) => a.repo.localeCompare(b.repo)),
  };
}

export type RepoCoverageDelta = {
  [K in "openIssues" | "gateNow" | "gateNext" | "poolItems" | "poolUnsetStatus" | "poolUnsetArea"]: number | null;
};

const deltaOrNull = (a: number | null, b: number | null | undefined): number | null =>
  a === null || b === null || b === undefined ? null : a - b;

/** Per-repo current-minus-baseline; null wherever either side is null. */
export function diffCoverage(current: CoverageReport, baseline: CoverageReport | null): Map<string, RepoCoverageDelta> {
  const out = new Map<string, RepoCoverageDelta>();
  if (!baseline) return out;
  const base = new Map(baseline.repos.map((r) => [r.repo, r]));
  for (const r of current.repos) {
    const b = base.get(r.repo);
    out.set(r.repo, {
      openIssues: deltaOrNull(r.openIssues, b?.openIssues),
      gateNow: deltaOrNull(r.gateNow, b?.gateNow),
      gateNext: deltaOrNull(r.gateNext, b?.gateNext),
      poolItems: deltaOrNull(r.poolItems, b?.poolItems),
      poolUnsetStatus: deltaOrNull(r.poolUnsetStatus, b?.poolUnsetStatus),
      poolUnsetArea: deltaOrNull(r.poolUnsetArea, b?.poolUnsetArea),
    });
  }
  return out;
}

/**
 * The next snapshot file after observing `current`. Within the week the
 * stored baseline is untouched and `current` simply refreshes; once the
 * stored current is a week old it becomes the baseline (the delta the
 * panel shows), and the fresh report takes its place.
 */
export function nextCoverageSnapshot(
  stored: CoverageSnapshotFile | null,
  current: CoverageReport,
  now = new Date(),
): CoverageSnapshotFile {
  if (!stored) return { baseline: null, current };
  const age = now.getTime() - new Date(stored.current.generatedAt).getTime();
  if (Number.isFinite(age) && age >= COVERAGE_WEEK_MS) {
    return { baseline: stored.current, current };
  }
  return { baseline: stored.baseline, current };
}

/** Thin JSON store for the coverage snapshot; corrupt reads as absent. */
export class CoverageSnapshotStore {
  constructor(private readonly file: string) {}

  async read(): Promise<CoverageSnapshotFile | null> {
    try {
      const parsed = JSON.parse(await readFile(this.file, "utf8")) as CoverageSnapshotFile;
      if (!parsed || typeof parsed !== "object" || !parsed.current || !Array.isArray(parsed.current.repos)) {
        return null;
      }
      return parsed;
    } catch {
      return null;
    }
  }

  async write(snapshot: CoverageSnapshotFile): Promise<void> {
    await mkdir(dirname(this.file), { recursive: true });
    const tmp = `${this.file}.tmp`;
    await writeFile(tmp, `${JSON.stringify(snapshot, null, 2)}\n`, "utf8");
    await rename(tmp, this.file);
  }
}
