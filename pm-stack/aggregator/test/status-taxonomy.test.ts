import { describe, expect, test } from "bun:test";
import {
  CANONICAL_STATUSES,
  STATUS_TAXONOMY,
  canonicalStatus,
  canonicalStatusFor,
  ghItemDerivedStatus,
  statusDisagreement,
} from "../src/status-taxonomy.ts";
import { derivedProjectState, projectDisagreement } from "../src/projects.ts";
import { deriveToday, todayStatusNote } from "../src/derive.ts";
import { projectsBodyHtml, renderBoardBody, renderRoadmap } from "../src/render.ts";
import type { BoardItem, GhItem, ProjectPoolItem, Provenance, RoadmapRow } from "../src/model.ts";

// #1198: one canonical status taxonomy — per-source mapping, derived
// In Review, cross-page parity, and disagreement shown (not silently
// resolved) on Board / Roadmap / Today.

const REF = "developing-today/code#1198";

function poolItem(partial: Partial<ProjectPoolItem> & { number: number }): ProjectPoolItem {
  return {
    id: `PVTI_${partial.number}`,
    url: `https://github.com/orgs/developing-today/projects/4?pane=issue&itemId=${partial.number}`,
    projectUrl: "https://github.com/orgs/developing-today/projects/4",
    ref: `developing-today/code#${partial.number}`,
    repo: "developing-today/code",
    issueUrl: `https://github.com/developing-today/code/issues/${partial.number}`,
    title: `item ${partial.number}`,
    state: "open",
    contentKind: "issue",
    status: null,
    area: null,
    horizon: null,
    kind: "Task",
    ...partial,
  };
}

function ghItem(partial: Partial<GhItem>): GhItem {
  return {
    ref: REF,
    kind: "issue",
    url: `https://github.com/developing-today/code/issues/1198`,
    title: "taxonomy item",
    state: "open",
    labels: [],
    author: "dezren39",
    updatedAt: "2026-11-12T00:00:00Z",
    ...partial,
  };
}

const provenance: Provenance = {
  board: "fixture",
  roadmap: "fixture",
  items: "fixture",
  comments: "fixture",
  quota: "fixture",
  usage: "fixture",
  attention: "fixture",
  orchestration: "fixture",
  dns: "fixture",
  registry: "fixture",
  aa: "fixture",
};

describe("canonical taxonomy mapping", () => {
  test("the canonical set is the Project vocabulary, in lifecycle order", () => {
    expect(CANONICAL_STATUSES).toEqual(["Todo", "In Progress", "In Review", "Done"]);
  });

  test("every source maps its own In Review spelling to canonical In Review", () => {
    expect(canonicalStatusFor("project", "In Review")).toBe("In Review");
    expect(canonicalStatusFor("kaneo", "in-review")).toBe("In Review");
    expect(canonicalStatusFor("grist", "In Review")).toBe("In Review");
    expect(canonicalStatusFor("github", "open pr")).toBe("In Review");
  });

  test("mapping is separator/case-insensitive across sources", () => {
    expect(canonicalStatusFor("kaneo", "in_progress")).toBe("In Progress");
    expect(canonicalStatusFor("grist", "in progress")).toBe("In Progress");
    expect(canonicalStatusFor("orchestration", "in-progress")).toBe("In Progress");
    expect(canonicalStatus("IN-REVIEW")).toBe("In Review");
  });

  test("unmapped values land in the explicit unmapped bucket (null), never a guess", () => {
    expect(canonicalStatusFor("grist", "No longer used")).toBeNull();
    expect(canonicalStatusFor("grist", "bogus")).toBeNull();
    expect(canonicalStatusFor("project", "Doing")).toBeNull();
    expect(canonicalStatusFor("kaneo", "")).toBeNull();
    expect(canonicalStatus(null)).toBeNull();
  });

  test("every taxonomy row targets a canonical status", () => {
    for (const row of STATUS_TAXONOMY) {
      expect(CANONICAL_STATUSES).toContain(row.canonical);
      expect(canonicalStatusFor(row.source, row.raw)).toBe(row.canonical);
    }
  });

  test("statusDisagreement: agreement is null, unmapped always disagrees", () => {
    expect(statusDisagreement("kaneo", "in-review", "In Review")).toBeNull();
    expect(statusDisagreement("kaneo", null, "Todo")).toBeNull();
    expect(statusDisagreement("kaneo", "to-do", "In Progress")).toEqual({
      source: "kaneo",
      raw: "to-do",
      canonical: "Todo",
      derived: "In Progress",
    });
    expect(statusDisagreement("grist", "No longer used", "Done")?.canonical).toBeNull();
  });
});

describe("derived In Review (#1198 acceptance 3)", () => {
  test("an open, ready PR derives In Review and agrees with Project Status In Review", () => {
    const pr = poolItem({ number: 7, contentKind: "pr", state: "open", draft: false, status: "In Review" });
    expect(derivedProjectState(pr, null)).toBe("In Review");
    expect(projectDisagreement(pr, null)).toBeNull();
  });

  test("a draft PR derives In Progress, not In Review", () => {
    const draft = poolItem({ number: 8, contentKind: "pr", state: "open", draft: true, status: "In Review" });
    expect(derivedProjectState(draft, null)).toBe("In Progress");
    expect(projectDisagreement(draft, null)).toEqual({ project: "In Review", derived: "In Progress" });
  });

  test("an issue with Project Status In Review but no review evidence still disagrees", () => {
    const issue = poolItem({ number: 9, contentKind: "issue", state: "open", status: "In Review" });
    expect(derivedProjectState(issue, null)).toBe("Todo");
    expect(projectDisagreement(issue, null)).toEqual({ project: "In Review", derived: "Todo" });
  });

  test("spelling variants of the Project Status do not fake a disagreement", () => {
    const done = poolItem({ number: 10, state: "closed", status: "done" });
    expect(projectDisagreement(done, null)).toBeNull();
  });

  test("ghItemDerivedStatus mirrors the same lifecycle rules", () => {
    expect(ghItemDerivedStatus({ state: "open", kind: "pr", draft: false })).toBe("In Review");
    expect(ghItemDerivedStatus({ state: "open", kind: "pr", draft: true })).toBe("In Progress");
    expect(ghItemDerivedStatus({ state: "merged", kind: "pr" })).toBe("Done");
    expect(ghItemDerivedStatus({ state: "open", kind: "issue" })).toBeNull();
  });
});

describe("cross-page parity (#1198 acceptance 2)", () => {
  test("the same inputs give the same canonical status on Project, Board, Roadmap, and Today", () => {
    // One logical item, four surfaces: Project Status "In Review",
    // Kaneo column "in-review", Grist row "In Review", and a Today
    // objective sourced from each — all canonicalize identically.
    const canonicals = [
      canonicalStatusFor("project", "In Review"),
      canonicalStatusFor("kaneo", "in-review"),
      canonicalStatusFor("grist", "In Review"),
      canonicalStatusFor("orchestration", "in-progress"),
    ];
    expect(new Set(canonicals.slice(0, 3))).toEqual(new Set(["In Review"]));
    expect(canonicals[3]).toBe("In Progress");

    // Agreement means NO disagreement note on any surface.
    const pr = ghItem({ kind: "pr", state: "open", draft: false, projectStatus: "In Review" });
    const items = new Map([[REF, pr]]);
    const links = [{ ref: REF, relation: "tracks" as const }];
    expect(todayStatusNote("grist", "In Review", links, items)).toBeUndefined();
    expect(todayStatusNote("kaneo", "in-review", links, items)).toBeUndefined();
  });

  test("Today carries the disagreement note when the sources disagree", () => {
    const pr = ghItem({ kind: "pr", state: "open", draft: false });
    const items = new Map([[REF, pr]]);
    const links = [{ ref: REF, relation: "tracks" as const }];
    expect(todayStatusNote("grist", "Planned", links, items)).toBe("⚠ grist: Planned · derived: In Review");

    const row: RoadmapRow = {
      id: 1,
      subject: "grist:doc/table/1",
      title: "Roadmap taxonomy row",
      area: "pm-stack",
      status: "Planned",
      horizon: "2026-11-12",
      source: "fixture",
      notes: "",
      links,
    };
    const board: BoardItem[] = [];
    const view = deriveToday(board, [row], items, null, new Date("2026-11-12T12:00:00Z"));
    const objective = view.objectives.find((o) => o.title === "Roadmap taxonomy row");
    expect(objective?.statusNote).toBe("⚠ grist: Planned · derived: In Review");
  });
});

describe("disagreement rendering (#1198 acceptances 1 + 4)", () => {
  test("the taxonomy table renders in the projects/program pool body", () => {
    const html = projectsBodyHtml([poolItem({ number: 11, status: "In Review" })], {});
    expect(html).toContain("Canonical status taxonomy (#1198)");
    expect(html).toContain("Kaneo board column");
    expect(html).toContain("Grist Roadmap Status");
    expect(html).toContain("In Review");
  });

  test("Board shows board-vs-project disagreement instead of silently picking the column", () => {
    const boardItem: BoardItem = {
      id: "kaneo:proj/1",
      title: "Board taxonomy card",
      repo: "developing-today/code",
      source: "kaneo",
      status: "in-progress",
      links: [{ ref: REF, relation: "tracks" }],
    };
    const pool = poolItem({ number: 1198, status: "In Review" });
    const html = renderBoardBody([boardItem], new Map(), undefined, {
      poolByRef: new Map([[REF, pool]]),
    });
    expect(html).toContain("⚠ board: in-progress · project: In Review");
  });

  test("Roadmap shows grist-vs-derived disagreement instead of silently picking Grist", () => {
    const row: RoadmapRow = {
      id: 2,
      subject: "grist:doc/table/2",
      title: "Roadmap taxonomy card",
      area: "pm-stack",
      status: "Planned",
      horizon: "2026-Q4",
      source: "fixture",
      notes: "",
      links: [{ ref: REF, relation: "tracks" }],
    };
    const items = new Map([[REF, ghItem({ kind: "pr", state: "open", draft: false })]]);
    const html = renderRoadmap([row], items, provenance, "quarter");
    expect(html).toContain("⚠ grist: Planned · derived: In Review");
  });
});
