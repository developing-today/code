// Grist roadmap ↔ Project epic reconciliation (#1144).
//
// The dashboard has two roadmap sources that drift silently: Grist's
// "Program roadmap" rows (what renderRoadmap draws) and the org
// Project pool's Kind=Epic items (the fleet feed's projects section).
// This pure function matches them 1:1 — by linked ref first (a row's
// links naming the epic's issue), then by normalized title — and
// reports both unmatched sides. Drift = unmatched rows + unmatched
// epics; the /roadmap page flags it when > 0 and /api/roadmap.json
// publishes the counts and lists, so the check is a tracked output of
// every roadmap read, not a report somebody has to remember to run.
import type { ProjectPoolItem, RoadmapRow } from "./model.ts";

export interface RoadmapMatch {
  row: RoadmapRow;
  epic: ProjectPoolItem;
  via: "ref" | "title";
}

export interface RoadmapReconciliation {
  /** "unavailable" when a source read failed and no drift may be
   *  asserted (Codex P2 on PR 2428): an unread feed or a Grist
   *  fixtures fallback must never masquerade as an empty epic pool. */
  availability: "available" | "unavailable";
  unavailableReason?: string;
  matched: RoadmapMatch[];
  /** Grist rows no Project epic claims. */
  unmatchedRows: RoadmapRow[];
  /** Project epics (Kind=Epic) no Grist row claims. */
  unmatchedEpics: ProjectPoolItem[];
  /** unmatchedRows + unmatchedEpics — 0 means the two sources agree. */
  drift: number;
}

/** Case/punctuation-insensitive title key for the fallback match. */
export function normalizeTitle(title: string): string {
  return title
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, " ")
    .trim()
    .replace(/\s+/g, " ");
}

export function reconcileRoadmap(rows: RoadmapRow[], pool: ProjectPoolItem[]): RoadmapReconciliation {
  const epics = pool.filter((p) => p.kind === "Epic");
  const epicByRef = new Map(epics.map((e) => [e.ref, e] as const));
  const used = new Set<string>();
  const matched: RoadmapMatch[] = [];

  // Pass 1: linked ref — a row that links the epic's issue claims it.
  const rest: RoadmapRow[] = [];
  for (const row of rows) {
    const epic = row.links.map((l) => epicByRef.get(l.ref)).find((e) => e && !used.has(e.ref));
    if (epic) {
      used.add(epic.ref);
      matched.push({ row, epic, via: "ref" });
    } else {
      rest.push(row);
    }
  }

  // Pass 2: normalized title against the epics still unclaimed. First
  // epic wins a duplicated title; a claimed epic leaves the index so
  // one epic can never match two rows.
  const epicByTitle = new Map<string, ProjectPoolItem>();
  for (const e of epics) {
    if (used.has(e.ref)) continue;
    const key = normalizeTitle(e.title);
    if (key && !epicByTitle.has(key)) epicByTitle.set(key, e);
  }
  const unmatchedRows: RoadmapRow[] = [];
  for (const row of rest) {
    const key = normalizeTitle(row.title);
    const epic = epicByTitle.get(key);
    if (epic && !used.has(epic.ref)) {
      used.add(epic.ref);
      epicByTitle.delete(key);
      matched.push({ row, epic, via: "title" });
    } else {
      unmatchedRows.push(row);
    }
  }

  const unmatchedEpics = epics.filter((e) => !used.has(e.ref));
  return { availability: "available", matched, unmatchedRows, unmatchedEpics, drift: unmatchedRows.length + unmatchedEpics.length };
}

/** The reconciliation when either side was not successfully read:
 *  no matches, no unmatched lists, no drift — just the reason. */
export function unavailableRoadmapReconciliation(reason: string): RoadmapReconciliation {
  return { availability: "unavailable", unavailableReason: reason, matched: [], unmatchedRows: [], unmatchedEpics: [], drift: 0 };
}

/** The published shape for /api/roadmap.json: counts plus both lists. */
export function roadmapDriftReport(rec: RoadmapReconciliation) {
  if (rec.availability === "unavailable") {
    return {
      availability: rec.availability,
      unavailableReason: rec.unavailableReason ?? "a roadmap source could not be read",
      matched: 0,
      matchedByRef: 0,
      matchedByTitle: 0,
      unmatchedRows: [] as { subject: string; title: string }[],
      unmatchedEpics: [] as { ref: string; title: string }[],
      drift: null,
    };
  }
  return {
    availability: rec.availability,
    matched: rec.matched.length,
    matchedByRef: rec.matched.filter((m) => m.via === "ref").length,
    matchedByTitle: rec.matched.filter((m) => m.via === "title").length,
    unmatchedRows: rec.unmatchedRows.map((r) => ({ subject: r.subject, title: r.title })),
    unmatchedEpics: rec.unmatchedEpics.map((e) => ({ ref: e.ref, title: e.title })),
    drift: rec.drift,
  };
}
