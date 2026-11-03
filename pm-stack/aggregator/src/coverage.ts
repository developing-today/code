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
// current, weekly, anchorAt }; rotation age is measured from anchorAt, a fixed
// anchor that does NOT move when `current` refreshes — measuring from
// the refreshed `current.generatedAt` would restart the week on every
// changed read and the baseline could stay null indefinitely during
// normal active use (Codex finding on PR #3309). All computation is
// pure — the store is a thin JSON file wrapper, and a missing/corrupt
// file reads as null. When the feed carries no census (collector had no
// token), backlog fields are null and the panel renders them as
// unavailable rather than inventing zeros; the same goes for the pool
// fields when the collector's Project read failed (pool null), so an
// unavailable pool never masquerades as zero coverage.
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
  /** Project #4 pool items whose linked issue/PR lives in this repo.
   * Null when the feed carried no projects section (pool unavailable). */
  poolItems: number | null;
  /** Pool items (this repo) with no Project Status set. */
  poolUnsetStatus: number | null;
  /** Pool items (this repo) with no Project Area set. */
  poolUnsetArea: number | null;
}

export interface CoverageReport {
  generatedAt: string;
  repos: RepoCoverage[];
}

export interface CoverageSnapshotFile {
  baseline: CoverageReport | null;
  current: CoverageReport;
  /** The frozen first report of the current week. Rotation promotes
   * THIS report to baseline - never current, which refreshes on every
   * changed read and would make the week-over-week delta compare
   * against a minutes-old observation (Codex finding on PR #3309).
   * Optional for snapshot files written before the field existed -
   * those fall back to current. */
  weekly?: CoverageReport | null;
  /** Fixed rotation anchor: when the current week began. Optional for
   * snapshot files written before the anchor existed — those fall back
   * to `current.generatedAt`. */
  anchorAt?: string;
}

export const COVERAGE_WEEK_MS = 7 * 24 * 60 * 60 * 1000;

function isRepositoryRef(value: string): boolean {
  const parts = value.split("/");
  if (parts.length !== 2) return false;
  const [owner, name] = parts;
  // GitHub owners are alphanumeric segments separated by single hyphens;
  // repository names additionally allow dots and underscores.
  return (
    /^[A-Za-z0-9]+(?:-[A-Za-z0-9]+)*$/.test(owner!) &&
    owner!.length <= 39 &&
    /^[A-Za-z0-9._-]{1,100}$/.test(name!) &&
    name !== "." &&
    name !== ".."
  );
}

/** Structural gate for one census row, shared by the feed reader. */
export function isRepoCensus(v: unknown): v is RepoCensus {
  if (!v || typeof v !== "object") return false;
  const r = v as Record<string, unknown>;
  return (
    typeof r.repo === "string" &&
    isRepositoryRef(r.repo) &&
    Number.isInteger(r.openIssues) &&
    (r.openIssues as number) >= 0 &&
    Number.isInteger(r.gateNow) &&
    (r.gateNow as number) >= 0 &&
    Number.isInteger(r.gateNext) &&
    (r.gateNext as number) >= 0 &&
    (r.gateNow as number) <= (r.openIssues as number) &&
    (r.gateNext as number) <= (r.openIssues as number)
  );
}

/**
 * Coverage from the collector's census plus the Project pool. Pure.
 * Repos are the union of what the census and the pool actually carry,
 * sorted by repo name. Backlog fields stay null for repos the census
 * did not cover; pool fields are real counts when the pool was read
 * and null when `pool` is null — the feed carried no projects section
 * (the collector's Project read failed or was never collected), which
 * must render as unavailable, never as zero coverage (Codex finding
 * on PR #3309).
 */
export function computeCoverage(
  census: RepoCensus[] | null,
  pool: ProjectPoolItem[] | null,
  now = new Date(),
): CoverageReport {
  const byRepo = new Map<string, RepoCoverage>();
  const row = (repo: string): RepoCoverage => {
    let r = byRepo.get(repo);
    if (!r) {
      r = {
        repo,
        openIssues: null,
        gateNow: null,
        gateNext: null,
        poolItems: pool ? 0 : null,
        poolUnsetStatus: pool ? 0 : null,
        poolUnsetArea: pool ? 0 : null,
      };
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
  for (const p of pool ?? []) {
    const r = row(p.repo);
    r.poolItems = (r.poolItems ?? 0) + 1;
    if (!p.status) r.poolUnsetStatus = (r.poolUnsetStatus ?? 0) + 1;
    if (!p.area) r.poolUnsetArea = (r.poolUnsetArea ?? 0) + 1;
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
 * stored baseline is untouched, `current` simply refreshes, and the
 * week's first report stays frozen in `weekly`; once the week anchored
 * at `anchorAt` has elapsed, the frozen weekly report becomes the
 * baseline (the delta the panel shows), the fresh report takes its
 * place as both `current` and the new week's frozen report, and a new
 * week anchors at `now`. The anchor is what makes the week a week:
 * `current` refreshes (new generatedAt) on every changed read, so its
 * timestamp can never be the rotation clock - and its values can never
 * be the baseline either, or the delta would measure only the time
 * since the last refresh instead of the week.
 */
export function nextCoverageSnapshot(
  stored: CoverageSnapshotFile | null,
  current: CoverageReport,
  now = new Date(),
): CoverageSnapshotFile {
  if (!stored) return { baseline: null, current, weekly: current, anchorAt: current.generatedAt };
  const anchor = stored.anchorAt ?? stored.current.generatedAt;
  const age = now.getTime() - new Date(anchor).getTime();
  if (Number.isFinite(age) && age >= COVERAGE_WEEK_MS) {
    return { baseline: stored.weekly ?? stored.current, current, weekly: current, anchorAt: now.toISOString() };
  }
  return { baseline: stored.baseline, current, weekly: stored.weekly ?? stored.current, anchorAt: anchor };
}

/** Thin JSON store for the coverage snapshot; corrupt reads as absent. */
export class CoverageSnapshotStore {
  private writeChain: Promise<void> = Promise.resolve();
  private writeSeq = 0;

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

  // Writes are serialized through a per-store promise chain and each
  // lands on a unique temp path, mirroring the comments/actions stores
  // (#1509): concurrent endpoint reads used to share one `.tmp` path,
  // so interleaved writes could rename a corrupted mix of two reports
  // into place (Codex finding on PR #3309). A failed write rejects its
  // own caller but does not poison the chain for later writes.
  write(snapshot: CoverageSnapshotFile): Promise<void> {
    const run = this.writeChain.then(async () => {
      await mkdir(dirname(this.file), { recursive: true });
      const tmp = `${this.file}.tmp-${process.pid}-${++this.writeSeq}`;
      await writeFile(tmp, `${JSON.stringify(snapshot, null, 2)}\n`, "utf8");
      await rename(tmp, this.file);
    });
    this.writeChain = run.catch(() => {});
    return run;
  }
}
