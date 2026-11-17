// Release milestones (#9067; design doc/2026-11-12T22-50-00Z_design_
// pm-board-hierarchy-genealogy/note.md section 4): close-out accounting
// over the milestone ingestion the collector now carries on every pool
// item (projects.ts: milestone { title, dueOn, closedAt }).
//
// Two milestone kinds (design D3):
// - program milestones: the existing rolling standing homes — dueOn is
//   null, they never close; accounting is computed for them
//   retroactively (counts only, no closures).
// - release milestones: bounded containers titled
//   "<Program> — release <version or date>" whose dueOn is the release
//   date; when one closes, its snapshot is the close-out record.
//
// A snapshot counts the items a milestone covers, split closed vs
// carried-over (open at snapshot time — for an epic, carried-over is
// the pinned D5 meaning: the epic spans releases and moves on), in
// three categories (design section 4): issues, designs (items carrying
// a design/<name> label or the kind/design label), and epics (Project
// Kind "Epic", or — for REST-collected items with no Project kind —
// the epic marker labels (`epic`, `meta/epic`) — never `epic/<name>`, which marks membership in an epic, not epic-ness). Categories are disjoint: an item is
// counted exactly once, epic first, then design, then issue. PRs are
// not milestone coverage and are never counted.
//
// Pure functions, no I/O — the collector/server/script callers own
// fetching, exactly like projects.ts.
import type { MilestoneRef, ProjectPoolItem } from "./model.ts";

export type MilestoneKind = "program" | "release";

export interface CategoryCounts {
  closed: number;
  carriedOver: number;
  total: number;
}

export interface ReleaseSnapshot {
  milestone: MilestoneRef;
  kind: MilestoneKind;
  /** The release date: a closed release milestone's closedAt date,
   * else its dueOn. Null for program milestones (they never release). */
  releaseDate: string | null;
  counts: {
    issues: CategoryCounts;
    designs: CategoryCounts;
    epics: CategoryCounts;
    total: CategoryCounts;
  };
  /** Refs of epics still open at snapshot time — the carried-over
   * list a closing milestone's snapshot must name (design D5). */
  carriedOverEpics: string[];
  /** The one-line close-out text, in the shape the design gives
   * Drew's example ("closed with the release covering 123 issues,
   * 23 designs, 4 epics") — this is the text written to a milestone
   * description at close-out and served verbatim by the API. */
  descriptionLine: string;
}

/** The minimal item shape the accounting needs — ProjectPoolItem
 * satisfies it, and so does a REST issue mapped by
 * scripts/release-accounting.ts. */
export interface MilestoneItemLike {
  ref?: string;
  state: "open" | "closed" | "merged";
  contentKind?: "issue" | "pr";
  kind?: string | null;
  labels?: string[];
}

export function classifyMilestoneItem(item: MilestoneItemLike): "epic" | "design" | "issue" {
  const labels = item.labels ?? [];
  if (item.kind === "Epic" || labels.includes("epic") || labels.includes("meta/epic")) return "epic";
  if (labels.includes("kind/design") || labels.some((l) => l.startsWith("design/"))) return "design";
  return "issue";
}

/** Release milestones are the bounded kind: a dueOn (the release
 * date), a closure, or the "<Program> — release <...>" title shape.
 * Everything else is a rolling program milestone. */
export function milestoneKind(milestone: MilestoneRef): MilestoneKind {
  if (milestone.dueOn !== null || milestone.closedAt !== null) return "release";
  return / — release /.test(milestone.title) ? "release" : "program";
}

function emptyCounts(): CategoryCounts {
  return { closed: 0, carriedOver: 0, total: 0 };
}

function snapshotDescriptionLine(
  kind: MilestoneKind,
  releaseDate: string | null,
  counts: ReleaseSnapshot["counts"],
): string {
  const covered = `${counts.total.total} items: ${counts.issues.total} issues, ${counts.designs.total} designs, ${counts.epics.total} epics`;
  const split = `${counts.total.closed} closed, ${counts.total.carriedOver} carried over`;
  if (kind === "program") return `Program milestone (rolling, never closes) — retroactive accounting: ${covered} (${split}).`;
  if (releaseDate) return `Closed with the release on ${releaseDate} covering ${covered} (${split}).`;
  return `Release milestone (open) — accounting so far: ${covered} (${split}).`;
}

/** Compute one milestone's close-out snapshot from the items the
 * collector ingested for it. Oracle: every counted item is an issue
 * (never a PR) in exactly one category, and closed + carriedOver
 * equals total in every category. */
export function computeMilestoneSnapshot(milestone: MilestoneRef, items: MilestoneItemLike[]): ReleaseSnapshot {
  const counts: ReleaseSnapshot["counts"] = {
    issues: emptyCounts(),
    designs: emptyCounts(),
    epics: emptyCounts(),
    total: emptyCounts(),
  };
  const carriedOverEpics: string[] = [];
  for (const item of items) {
    if (item.contentKind === "pr") continue;
    const category = classifyMilestoneItem(item);
    const bucket = category === "epic" ? counts.epics : category === "design" ? counts.designs : counts.issues;
    const isClosed = item.state === "closed" || item.state === "merged";
    bucket.total += 1;
    counts.total.total += 1;
    if (isClosed) {
      bucket.closed += 1;
      counts.total.closed += 1;
    } else {
      bucket.carriedOver += 1;
      counts.total.carriedOver += 1;
      if (category === "epic" && item.ref) carriedOverEpics.push(item.ref);
    }
  }
  carriedOverEpics.sort();
  const kind = milestoneKind(milestone);
  const releaseDate = kind === "release" ? (milestone.closedAt ?? milestone.dueOn) : null;
  return {
    milestone,
    kind,
    releaseDate,
    counts,
    carriedOverEpics,
    descriptionLine: snapshotDescriptionLine(kind, releaseDate, counts),
  };
}

/** Snapshots for every milestone represented in the pool, unioned
 * with `knownMilestones` — the retroactive pass: a known milestone
 * with no pool items still gets a (zero-count) snapshot, so the
 * existing program milestones all appear in /api/releases.json even
 * before their items are Project-board members. Sorted by title for
 * a stable served shape. */
export function buildReleaseSnapshots(items: ProjectPoolItem[], knownMilestones: MilestoneRef[] = []): ReleaseSnapshot[] {
  const byTitle = new Map<string, { milestone: MilestoneRef; items: ProjectPoolItem[] }>();
  for (const known of knownMilestones) {
    if (!byTitle.has(known.title)) byTitle.set(known.title, { milestone: known, items: [] });
  }
  for (const item of items) {
    if (!item.milestone) continue;
    const entry = byTitle.get(item.milestone.title) ?? { milestone: item.milestone, items: [] };
    // Prefer the freshest milestone stamps the collector saw.
    entry.milestone = item.milestone;
    entry.items.push(item);
    byTitle.set(item.milestone.title, entry);
  }
  return [...byTitle.values()]
    .map(({ milestone, items: milestoneItems }) => computeMilestoneSnapshot(milestone, milestoneItems))
    .sort((a, b) => a.milestone.title.localeCompare(b.milestone.title));
}
