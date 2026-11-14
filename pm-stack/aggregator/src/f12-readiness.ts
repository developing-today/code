// Phase 12 readiness panel data (#9926, split from #1239): per-F-item
// state for F12.1–F12.18, joined live from three GitHub sources —
//   * the F-item list: #1235's Tier-2 line (the ordered-queue issue
//     whose body the phase12.ts feed already reads);
//   * design state: #534's checklist (the Phase 12 designs epic —
//     each design item is a D1-nnn entry naming its F-item, ticked
//     when the design is committed). F-items with no #534 entry
//     render "no design entry", never an invented design state;
//   * issue + PR state: the aggregator's GhItem states for the item
//     issues, and PR cross-references from each issue's timeline
//     (read by the live source; a merged cross-referenced PR is the
//     strongest PR signal GitHub exposes for an issue).
// Everything here is pure: parsers for both body grammars and the
// projection are fixture-tested, including a design-but-no-issue
// item and an item with a merged PR.
import type { GhItem, RefKey } from "./model.ts";
import { PHASE12_REPO } from "./phase12.ts";

export const F12_DESIGN_ISSUE_NUMBER = 534;
export const F12_DESIGN_ISSUE_URL = `https://github.com/${PHASE12_REPO}/issues/${F12_DESIGN_ISSUE_NUMBER}`;

/** One F-item from #1235's Tier-2 line. */
export interface F12Item {
  /** "F12.1" … "F12.18". */
  f: string;
  /** Numeric part, for ordering. */
  n: number;
  issueNumber: number;
}

// #1235 Tier-2 grammar (verified against its real body, 2026-11-14):
//   "#807 F12.1, #556 F12.2, #835 F12.3 remainder, …, #559 done F12.16, …"
const TIER2_PAIR = /#(\d+)\s+(?:done\s+)?F12\.(\d+)\b/g;

/** Parse the F12 item list from #1235's body. Returns the items in
 * F-number order, deduplicated; a body without the Tier-2 line
 * parses to an empty list (the panel renders unavailable). */
export function parseF12Items(phase12Body: string): F12Item[] {
  const byN = new Map<number, F12Item>();
  for (const m of phase12Body.matchAll(TIER2_PAIR)) {
    const issueNumber = Number(m[1]);
    const n = Number(m[2]);
    if (!Number.isSafeInteger(issueNumber) || !Number.isSafeInteger(n) || n <= 0) continue;
    if (!byN.has(n)) byN.set(n, { f: `F12.${n}`, n, issueNumber });
  }
  return [...byN.values()].sort((a, b) => a.n - b.n);
}

/** One design entry from #534's checklist. */
export interface F12DesignEntry {
  /** The D1-nnn design id, e.g. "D1-019". */
  designId: string;
  /** The checkbox state in #534's body. */
  done: boolean;
}

// #534 grammar (verified against its real body, 2026-11-14):
//   - [x] **D1-019 — F12.1 Declarative world manifest …**
const DESIGN_LINE = /^- \[( |x|X)\] \*\*(D1-\d+) — F12\.(\d+)\b/;

/** Parse #534's design checklist into a map by F-number. F-items
 * absent from the map have no design entry in #534. */
export function parseF12Designs(designBody: string): Map<number, F12DesignEntry> {
  const out = new Map<number, F12DesignEntry>();
  for (const line of designBody.split(/\r?\n/)) {
    const m = DESIGN_LINE.exec(line.trim());
    if (!m) continue;
    const n = Number(m[3]);
    if (!Number.isSafeInteger(n) || n <= 0 || out.has(n)) continue;
    out.set(n, { designId: m[2], done: m[1] !== " " });
  }
  return out;
}

/** A PR cross-referenced from an issue's timeline. */
export interface F12PrRef {
  number: number;
  state: "merged" | "open" | "closed";
  url: string;
}

/** Parse one issue-timeline response into its cross-referenced PRs.
 * Only cross-referenced events whose source is a pull request count;
 * a merged_at on the source PR is what marks a PR merged. */
export function parseTimelinePrRefs(json: unknown): F12PrRef[] {
  if (!Array.isArray(json)) throw new Error("timeline response is not an array");
  const byNumber = new Map<number, F12PrRef>();
  for (const raw of json) {
    if (!raw || typeof raw !== "object") continue;
    const ev = raw as { event?: unknown; source?: { issue?: Record<string, unknown> } };
    if (ev.event !== "cross-referenced") continue;
    const issue = ev.source?.issue;
    if (!issue || typeof issue.number !== "number" || typeof issue.html_url !== "string") continue;
    const pr = issue.pull_request as { merged_at?: unknown } | undefined;
    if (!pr || typeof pr !== "object") continue;
    const state: F12PrRef["state"] =
      typeof pr.merged_at === "string" ? "merged" : issue.state === "closed" ? "closed" : "open";
    byNumber.set(issue.number, { number: issue.number, state, url: issue.html_url });
  }
  return [...byNumber.values()].sort((a, b) => a.number - b.number);
}

/** One readiness row: an F-item joined with design, issue, PR state. */
export interface F12ReadinessRow extends F12Item {
  ref: RefKey;
  url: string;
  /** Live issue title when resolved. */
  title: string | null;
  /** GhItem state; null = the source could not resolve the issue. */
  issueState: GhItem["state"] | null;
  /** The #534 design entry; null = no design entry in #534. */
  design: F12DesignEntry | null;
  /** Cross-referenced PRs (empty when the timeline read is absent
   * or the issue has none). */
  prs: F12PrRef[];
}

/** Join the F-item list with design entries, GhItem states, and PR
 * cross-references. Missing inputs stay null/empty — the panel
 * renders them as unavailable, never as invented states. */
export function projectF12Readiness(
  items: F12Item[],
  designs: ReadonlyMap<number, F12DesignEntry>,
  ghItems: ReadonlyMap<RefKey, GhItem>,
  prRefsByIssue: ReadonlyMap<number, F12PrRef[]> = new Map(),
): F12ReadinessRow[] {
  return items.map((item) => {
    const ref: RefKey = `${PHASE12_REPO}#${item.issueNumber}`;
    const gh = ghItems.get(ref);
    return {
      ...item,
      ref,
      url: gh?.url ?? `https://github.com/${PHASE12_REPO}/issues/${item.issueNumber}`,
      title: gh ? gh.title : null,
      issueState: gh ? gh.state : null,
      design: designs.get(item.n) ?? null,
      prs: prRefsByIssue.get(item.issueNumber) ?? [],
    };
  });
}
