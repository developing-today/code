// Canonical status taxonomy (#1198): ONE status vocabulary for the
// PM stack, plus the per-source mapping table that translates every
// source's own vocabulary into it.
//
// The canonical set is the org Project's Status vocabulary — the set
// already published as PROJECT_STATUS_VALUES (render.ts) — adopted,
// not reinvented: Todo | In Progress | In Review | Done.
//
// Four sources speak four vocabularies:
//   - project:       GitHub Project #4 single-select Status strings
//                    ("In Review", "In Progress", …)
//   - kaneo:         Kaneo board column ids ("in-review", …), plus the
//                    synthetic orchestrator lane key "in-flight"
//   - grist:         the Roadmap table's free-Choice Status strings
//                    ("Planned", "Idea", "Shipped", …)
//   - orchestration: lined-up task statuses ("queued", "blocked", …)
//   - github:        the linked issue/PR's own lifecycle ("open pr", …)
//
// A raw value with NO row in its source's table maps to null — the
// explicit unmapped bucket. Callers must surface an unmapped value as
// a disagreement/unknown, never guess a canonical status for it:
// silently picking one source (or one spelling) is the defect #1198
// exists to remove.

/** The canonical status set, in lifecycle order. */
export type CanonicalStatus = "Todo" | "In Progress" | "In Review" | "Done";

export const CANONICAL_STATUSES: readonly CanonicalStatus[] = ["Todo", "In Progress", "In Review", "Done"];

export type StatusSource = "project" | "kaneo" | "grist" | "orchestration" | "github";

export const STATUS_SOURCE_LABELS: Record<StatusSource, string> = {
  project: "GitHub Project Status",
  kaneo: "Kaneo board column",
  grist: "Grist Roadmap Status",
  orchestration: "Orchestration lined-up status",
  github: "GitHub issue/PR lifecycle",
};

export interface StatusTaxonomyRow {
  source: StatusSource;
  /** The raw value as the source spells it. */
  raw: string;
  canonical: CanonicalStatus;
}

/** The mapping table, as data: rendered on /projects and /program
 * (#1198 acceptance 1) and compiled into the lookup maps below. */
export const STATUS_TAXONOMY: readonly StatusTaxonomyRow[] = [
  // GitHub Project #4 Status (single-select).
  { source: "project", raw: "Todo", canonical: "Todo" },
  { source: "project", raw: "Backlog", canonical: "Todo" },
  { source: "project", raw: "In Progress", canonical: "In Progress" },
  { source: "project", raw: "In Review", canonical: "In Review" },
  { source: "project", raw: "Done", canonical: "Done" },
  // Kaneo board column ids (+ the synthetic orchestrator lane key).
  { source: "kaneo", raw: "to-do", canonical: "Todo" },
  { source: "kaneo", raw: "in-progress", canonical: "In Progress" },
  { source: "kaneo", raw: "in-flight", canonical: "In Progress" },
  { source: "kaneo", raw: "in-review", canonical: "In Review" },
  { source: "kaneo", raw: "done", canonical: "Done" },
  // Grist "Program roadmap" Status (free Choice — unmapped values
  // exist in the wild and land in the unmapped bucket, e.g.
  // "No longer used", which is a retired row, not finished work).
  { source: "grist", raw: "Idea", canonical: "Todo" },
  { source: "grist", raw: "Planned", canonical: "Todo" },
  { source: "grist", raw: "Not started", canonical: "Todo" },
  { source: "grist", raw: "Todo", canonical: "Todo" },
  { source: "grist", raw: "In Progress", canonical: "In Progress" },
  { source: "grist", raw: "Active", canonical: "In Progress" },
  { source: "grist", raw: "In Review", canonical: "In Review" },
  { source: "grist", raw: "Done", canonical: "Done" },
  { source: "grist", raw: "Complete", canonical: "Done" },
  { source: "grist", raw: "Shipped", canonical: "Done" },
  // Orchestration lined-up task statuses. "blocked" and "stalled"
  // describe an in-progress item's blocker, not a fifth status: the
  // canonical status stays In Progress and the blocker is shown
  // separately, exactly as the board already renders it.
  { source: "orchestration", raw: "queued", canonical: "Todo" },
  { source: "orchestration", raw: "in-progress", canonical: "In Progress" },
  { source: "orchestration", raw: "blocked", canonical: "In Progress" },
  { source: "orchestration", raw: "stalled", canonical: "In Progress" },
  { source: "orchestration", raw: "done", canonical: "Done" },
  { source: "orchestration", raw: "done-recently", canonical: "Done" },
  // GitHub lifecycle of the linked item itself (the derived side).
  { source: "github", raw: "open issue", canonical: "Todo" },
  { source: "github", raw: "draft pr", canonical: "In Progress" },
  { source: "github", raw: "open pr", canonical: "In Review" },
  { source: "github", raw: "merged pr", canonical: "Done" },
  { source: "github", raw: "closed pr", canonical: "Done" },
  { source: "github", raw: "closed issue", canonical: "Done" },
];

/** Separator/case-insensitive key: "In Review", "in-review" and
 * "in_review" are the same status spelled by different sources. */
export function normalizeStatusKey(raw: string): string {
  return raw
    .trim()
    .toLowerCase()
    .replace(/[-_]+/g, " ")
    .replace(/\s+/g, " ");
}

const LOOKUPS: Record<StatusSource, Map<string, CanonicalStatus>> = (() => {
  const out = {} as Record<StatusSource, Map<string, CanonicalStatus>>;
  for (const row of STATUS_TAXONOMY) {
    (out[row.source] ??= new Map()).set(normalizeStatusKey(row.raw), row.canonical);
  }
  return out;
})();

const SOURCE_ORDER: readonly StatusSource[] = ["project", "kaneo", "grist", "orchestration", "github"];

/**
 * The canonical status for one source's raw value, or null when the
 * value is in the unmapped bucket (unknown vocabulary — surface it,
 * never guess).
 */
export function canonicalStatusFor(source: StatusSource, raw: string | null | undefined): CanonicalStatus | null {
  if (!raw) return null;
  return LOOKUPS[source].get(normalizeStatusKey(raw)) ?? null;
}

/** Canonical status when the source is unknown: the first source (in
 * SOURCE_ORDER) whose table knows the value, else null. */
export function canonicalStatus(raw: string | null | undefined): CanonicalStatus | null {
  if (!raw) return null;
  for (const source of SOURCE_ORDER) {
    const hit = canonicalStatusFor(source, raw);
    if (hit) return hit;
  }
  return null;
}

/**
 * The status GitHub's own lifecycle derives for an item, without any
 * other source: closed/merged is Done; an open ready PR is In Review
 * (GitHub's draft/ready distinction IS the review boundary); an open
 * draft PR is In Progress. An open issue derives nothing here (null)
 * — its derived state needs the orchestration evidence that
 * derivedProjectState (projects.ts) weighs.
 */
export function ghItemDerivedStatus(item: {
  state: string;
  kind: string;
  draft?: boolean;
}): CanonicalStatus | null {
  if (item.state === "closed" || item.state === "merged") return "Done";
  if (item.kind === "pr" && item.state === "open") return item.draft ? "In Progress" : "In Review";
  return null;
}

/** A source-vs-derived disagreement to display: both sides, neither
 * preferred. `canonical` is null when the source value is unmapped. */
export interface StatusDisagreement {
  source: StatusSource;
  /** The source's raw value, verbatim. */
  raw: string;
  canonical: CanonicalStatus | null;
  derived: CanonicalStatus;
}

/**
 * Compare one source's raw status against the derived canonical
 * status. Returns null when there is nothing to disagree about (no
 * raw value, or both sides agree canonically); a StatusDisagreement
 * otherwise — including when the raw value is unmapped, because an
 * unknown vocabulary silently treated as agreement is how the pages
 * drifted apart.
 */
export function statusDisagreement(
  source: StatusSource,
  raw: string | null | undefined,
  derived: CanonicalStatus,
): StatusDisagreement | null {
  if (!raw) return null;
  const canonical = canonicalStatusFor(source, raw);
  return canonical === derived ? null : { source, raw, canonical, derived };
}
