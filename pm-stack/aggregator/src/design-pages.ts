// C7 leaf 3 — design pages and implementation rollups (#12353; design
// doc/2026-11-13T15-12-00Z_design_reporting-architecture-event-driven/
// c7-rollups-design.md, "Design pages, project spaces, and timeline").
//
// One canonical page model per design: origin and parent links, the
// subdesign tree, the status ladder, point-in-time member counts
// (done / in-progress / blocked / queued) with blocker reasons, and the
// daily/weekly change notes that the stored C7 report records already
// carry; long-running designs link their monthly/quarterly pages.
//
// What this module refuses to do, stated once:
//
// * It keeps no genealogy model of its own. Which design an item belongs
//   to, and which designs are subdesigns, come from `deriveGenealogy`
//   (#9060's read model): an item belongs to the design its genealogy
//   names as `spawned-by`.
// * It never guesses an issue's outcome. A merged PR, or a closed issue
//   whose close reason is `completed`, is implemented. `not_planned` is
//   cancelled and `duplicate` is a duplicate; both are counted apart and
//   never as done. A closed issue with no known reason is
//   `closed-unclassified`, shown as unknown, never as done.
// * It never claims a ladder stage it cannot see. Stages the evidence
//   proves are `reached`; the rest are `unknown`, not `not reached`.
//
// Pure module: no I/O, no clock. The caller passes the pool snapshot,
// the close reasons it read, and the stored report records.

import { deriveGenealogy } from "./genealogy.ts";
import type { ProjectPoolItem, RefKey } from "./model.ts";
import type { ReportRecord } from "./report-records.ts";

/** GitHub's issue close reasons (`stateReason`), lower-cased. */
export type CloseReason = "completed" | "not_planned" | "duplicate";

export type MemberClass =
  | "implemented"
  | "in-progress"
  | "blocked"
  | "queued"
  | "cancelled"
  | "duplicate"
  | "closed-unclassified";

export interface DesignMember {
  ref: RefKey;
  title: string;
  class: MemberClass;
  /** Open blockers named by the item's own `blockedBy` edges; empty unless blocked. */
  blockedBy: RefKey[];
}

/** The C7 status ladder, in order. */
export const DESIGN_LADDER = [
  "idea",
  "planned",
  "initial issue",
  "issue built out",
  "fully designed",
  "implementation issues",
] as const;
export type DesignLadderStage = (typeof DESIGN_LADDER)[number];

export interface LadderStep {
  stage: DesignLadderStage;
  state: "reached" | "unknown";
  evidence: string | null;
}

export interface SubdesignNode {
  ref: RefKey;
  title: string;
  children: SubdesignNode[];
}

export interface DesignChangeNote {
  periodKind: "daily" | "weekly";
  periodId: string;
  recordId: string;
  blockId: string;
  text: string;
}

export interface DesignPeriodLink {
  periodKind: "monthly" | "quarterly";
  periodId: string;
  recordId: string;
}

export interface DesignCounts {
  /** Implemented work only. Cancelled and duplicate closures never count here. */
  done: number;
  inProgress: number;
  blocked: number;
  queued: number;
  cancelled: number;
  duplicate: number;
  /** Closed with no known reason: shown, never folded into done. */
  unknown: number;
  total: number;
}

export interface DesignPage {
  ref: RefKey;
  title: string;
  issueUrl: string;
  /** Genealogy ancestors of the design itself (origin, parents, roadmap). */
  origin: { ref: RefKey; title: string; relation: string }[];
  ladder: LadderStep[];
  subdesigns: SubdesignNode[];
  members: DesignMember[];
  counts: DesignCounts;
  changeNotes: DesignChangeNote[];
  periodPages: DesignPeriodLink[];
  /** The pool snapshot time the counts are true at. */
  asOf: string;
}

const isDesign = (item: ProjectPoolItem): boolean =>
  item.kind === "Design" || (item.labels ?? []).includes("kind/design");

const IN_PROGRESS_STATUSES = new Set(["in progress", "in review"]);

/** Classify one member from its own state, its close reason and its open blockers. */
export function classifyMember(
  item: ProjectPoolItem,
  closeReason: CloseReason | undefined,
  openRefs: ReadonlySet<RefKey>,
): { class: MemberClass; blockedBy: RefKey[] } {
  if (item.state === "merged") return { class: "implemented", blockedBy: [] };
  if (item.state === "closed") {
    if (item.contentKind === "pr") return { class: "cancelled", blockedBy: [] };
    switch (closeReason) {
      case "completed":
        return { class: "implemented", blockedBy: [] };
      case "not_planned":
        return { class: "cancelled", blockedBy: [] };
      case "duplicate":
        return { class: "duplicate", blockedBy: [] };
      default:
        return { class: "closed-unclassified", blockedBy: [] };
    }
  }
  const blockedBy = (item.blockedBy ?? []).filter((ref) => openRefs.has(ref));
  if (blockedBy.length > 0) return { class: "blocked", blockedBy };
  const status = item.status?.trim().toLowerCase() ?? "";
  if (IN_PROGRESS_STATUSES.has(status) || (item.contentKind === "pr" && item.state === "open")) {
    return { class: "in-progress", blockedBy: [] };
  }
  return { class: "queued", blockedBy: [] };
}

function dedupe(pool: readonly ProjectPoolItem[]): ProjectPoolItem[] {
  const byRef = new Map<RefKey, ProjectPoolItem>();
  for (const entry of pool) if (!byRef.has(entry.ref)) byRef.set(entry.ref, entry);
  return [...byRef.values()];
}

function spawnedBy(item: ProjectPoolItem, pool: readonly ProjectPoolItem[]): RefKey | null {
  return deriveGenealogy(item, pool).ancestors.find((a) => a.relation === "spawned-by")?.ref ?? null;
}

function subdesignTree(
  root: RefKey,
  designs: readonly ProjectPoolItem[],
  parentOf: ReadonlyMap<RefKey, RefKey | null>,
  seen: Set<RefKey>,
): SubdesignNode[] {
  return designs
    .filter((d) => parentOf.get(d.ref) === root && !seen.has(d.ref))
    .sort((a, b) => a.ref.localeCompare(b.ref))
    .map((d) => {
      seen.add(d.ref);
      return {
        ref: d.ref,
        title: d.title,
        children: subdesignTree(d.ref, designs, parentOf, seen),
      };
    });
}

function ladderFor(
  design: ProjectPoolItem,
  members: readonly DesignMember[],
  subdesigns: readonly SubdesignNode[],
): LadderStep[] {
  const reached = new Map<DesignLadderStage, string>();
  // The design is a tracked GitHub issue, so the first three stages are proven by its existence.
  reached.set("idea", `${design.ref} exists`);
  reached.set("planned", `${design.ref} exists`);
  reached.set("initial issue", `${design.ref} is an issue`);
  if (subdesigns.length > 0 || (design.subIssuesSummary?.total ?? 0) > 0) {
    reached.set("issue built out", "has subdesigns or sub-issues");
  }
  if (members.length > 0) {
    reached.set("implementation issues", `${members.length} linked implementation item(s)`);
  }
  // A later stage implies the earlier ones only when the evidence is about the
  // same design: implementation issues imply the design was built out, but
  // "fully designed" is a human judgement this model cannot see, so it stays
  // unknown unless an explicit `design/fully-designed` label says so.
  if ((design.labels ?? []).includes("design/fully-designed")) {
    reached.set("fully designed", "labelled design/fully-designed");
  }
  if (reached.has("implementation issues") && !reached.has("issue built out")) {
    reached.set("issue built out", "has implementation items");
  }
  return DESIGN_LADDER.map((stage) => ({
    stage,
    state: reached.has(stage) ? "reached" : "unknown",
    evidence: reached.get(stage) ?? null,
  }));
}

/**
 * Build the canonical page for one design from a pool snapshot, the close
 * reasons the caller read, and stored C7 report records.
 */
export function buildDesignPage(
  designRef: RefKey,
  poolInput: readonly ProjectPoolItem[],
  closeReasons: ReadonlyMap<RefKey, CloseReason>,
  records: readonly ReportRecord[],
  asOf: string,
): DesignPage {
  const pool = dedupe(poolInput);
  const design = pool.find((i) => i.ref === designRef);
  if (!design) throw new Error(`design-pages: ${designRef} is not in the pool snapshot`);
  if (!isDesign(design)) throw new Error(`design-pages: ${designRef} is not a design`);

  const openRefs = new Set(pool.filter((i) => i.state === "open").map((i) => i.ref));
  const parentOf = new Map(pool.map((i) => [i.ref, spawnedBy(i, pool)] as const));
  const designs = pool.filter(isDesign);

  const members: DesignMember[] = pool
    .filter((i) => !isDesign(i) && parentOf.get(i.ref) === designRef)
    .sort((a, b) => a.ref.localeCompare(b.ref))
    .map((i) => {
      const c = classifyMember(i, closeReasons.get(i.ref), openRefs);
      return {
        ref: i.ref,
        title: i.title,
        class: c.class,
        blockedBy: c.blockedBy,
      };
    });

  const count = (cls: MemberClass) => members.filter((m) => m.class === cls).length;
  const counts: DesignCounts = {
    done: count("implemented"),
    inProgress: count("in-progress"),
    blocked: count("blocked"),
    queued: count("queued"),
    cancelled: count("cancelled"),
    duplicate: count("duplicate"),
    unknown: count("closed-unclassified"),
    total: members.length,
  };

  const subdesigns = subdesignTree(designRef, designs, parentOf, new Set([designRef]));
  const related = new Set<string>([designRef, ...members.map((m) => m.ref)]);
  const mentions = (links: readonly string[] | undefined) => (links ?? []).some((l) => related.has(l));

  const changeNotes: DesignChangeNote[] = [];
  const periodPages: DesignPeriodLink[] = [];
  for (const record of records) {
    const kind = record.period.kind;
    const hits = record.blocks.filter((b) => mentions(b.links));
    if (hits.length === 0) continue;
    if (kind === "daily" || kind === "weekly") {
      for (const b of hits) {
        changeNotes.push({
          periodKind: kind,
          periodId: record.period.id,
          recordId: record.id,
          blockId: b.id,
          text: b.text,
        });
      }
    } else if (kind === "monthly" || kind === "quarterly") {
      periodPages.push({
        periodKind: kind,
        periodId: record.period.id,
        recordId: record.id,
      });
    }
  }
  const byPeriod = (a: { periodId: string }, b: { periodId: string }) => a.periodId.localeCompare(b.periodId);
  changeNotes.sort((a, b) => byPeriod(a, b) || a.blockId.localeCompare(b.blockId));
  periodPages.sort(byPeriod);

  return {
    ref: design.ref,
    title: design.title,
    issueUrl: design.issueUrl,
    origin: deriveGenealogy(design, pool).ancestors.map((a) => ({
      ref: a.ref,
      title: a.title,
      relation: a.relation,
    })),
    ladder: ladderFor(design, members, subdesigns),
    subdesigns,
    members,
    counts,
    changeNotes,
    periodPages,
    asOf,
  };
}

function renderTree(nodes: readonly SubdesignNode[], depth: number): string[] {
  return nodes.flatMap((n) => [`${"  ".repeat(depth)}- ${n.title} (${n.ref})`, ...renderTree(n.children, depth + 1)]);
}

/** Render the page as Markdown. Every count is the page model's; nothing is recomputed here. */
export function renderDesignPage(page: DesignPage): string {
  const c = page.counts;
  const lines: string[] = [
    `# ${page.title}`,
    "",
    `[${page.ref}](${page.issueUrl}) · counts as of ${page.asOf}`,
    "",
    "## Origin",
    "",
    ...(page.origin.length ? page.origin.map((o) => `- ${o.relation}: ${o.title} (${o.ref})`) : ["- none recorded"]),
    "",
    "## Status ladder",
    "",
    ...page.ladder.map((s) => `- ${s.stage}: ${s.state}${s.evidence ? ` (${s.evidence})` : ""}`),
    "",
    "## Subdesigns",
    "",
    ...(page.subdesigns.length ? renderTree(page.subdesigns, 0) : ["- none"]),
    "",
    "## Implementation",
    "",
    `${c.done} done · ${c.inProgress} in progress · ${c.blocked} blocked · ${c.queued} queued (of ${c.total})`,
    "",
    `Closed apart from done: ${c.cancelled} cancelled · ${c.duplicate} duplicate · ${c.unknown} closed with unknown reason`,
    "",
    ...page.members.map(
      (m) =>
        `- ${m.class}: ${m.title} (${m.ref})${m.blockedBy.length ? ` — blocked by ${m.blockedBy.join(", ")}` : ""}`,
    ),
    "",
    "## Changes",
    "",
    ...(page.changeNotes.length
      ? page.changeNotes.map((n) => `- ${n.periodKind} ${n.periodId}: ${n.text}`)
      : ["- no stored report mentions this design yet"]),
  ];
  if (page.periodPages.length) {
    lines.push("", "## Longer periods", "", ...page.periodPages.map((p) => `- ${p.periodKind} ${p.periodId}`));
  }
  return `${lines.join("\n")}\n`;
}
