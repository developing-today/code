import { describe, expect, test } from "bun:test";
import type { GhItem, OrchestrationSection, ProjectPoolItem, Provenance } from "../src/model.ts";
import { PAGES, pageByRoute } from "../src/pages.ts";
import { buildProgramData, buildProgramLive, PROGRAM, programRefs, TRACKED_REFS } from "../src/program.ts";
import { renderProgram } from "../src/render.ts";
import { fixtureSource } from "../src/sources/fixtures.ts";

const PROV: Provenance = {
  board: "x",
  roadmap: "x",
  items: "x",
  comments: "x",
  quota: "x",
  usage: "x",
  attention: "x",
  orchestration: "x",
  dns: "x",
  registry: "x",
};

function poolItem(partial: Partial<ProjectPoolItem>): ProjectPoolItem {
  return {
    id: "PVTI_x",
    url: "https://github.com/orgs/developing-today/projects/4?pane=issue&itemId=1",
    projectUrl: "https://github.com/orgs/developing-today/projects/4",
    ref: "developing-today/code#1",
    repo: "developing-today/code",
    number: 1,
    issueUrl: "https://github.com/developing-today/code/issues/1",
    title: "item",
    state: "open",
    contentKind: "issue",
    status: null,
    area: null,
    horizon: null,
    kind: null,
    ...partial,
  };
}

describe("page manifest (#1046)", () => {
  test("PAGES contains program with the right route + api twin", () => {
    const page = PAGES.find((p) => p.id === "program");
    expect(page).toBeDefined();
    expect(page?.title).toBe("Program status");
    expect(page?.route).toBe("/program");
    expect(page?.api).toBe("/api/program.json");
    expect(pageByRoute("/program")?.id).toBe("program");
  });
});

describe("program data builder", () => {
  test("handles empty pool, absent items, and null orchestration", () => {
    const live = buildProgramLive(new Map(), [], null);
    expect(live.pool.total).toBe(0);
    expect(live.pool.byStatus).toEqual([]);
    expect(live.pool.byArea).toEqual([]);
    expect(live.orchestration).toBeNull();
    // Every tracked ref is present and unresolved — never dropped,
    // never given an invented state.
    expect(live.tracked.map((t) => t.ref)).toEqual(TRACKED_REFS);
    expect(live.tracked.every((t) => t.item === null)).toBe(true);
  });

  test("derives pool counts by status and area, and orchestration freshness", () => {
    const pool = [
      poolItem({ status: "Done", area: "pm-dashboard" }),
      poolItem({ status: "Done", area: "fleet-ops" }),
      poolItem({ status: "Todo", area: "pm-dashboard" }),
      poolItem({ status: null, area: null }),
    ];
    const section = {
      generatedAt: "2026-10-26T17:00:00Z",
      runId: "orch-2026-10-26T17:00:00Z",
    } as OrchestrationSection;
    const live = buildProgramLive(new Map(), pool, section);
    expect(live.pool.total).toBe(4);
    expect(live.pool.byStatus).toContainEqual({ name: "Done", count: 2 });
    expect(live.pool.byStatus).toContainEqual({ name: "(unset)", count: 1 });
    expect(live.pool.byArea).toContainEqual({ name: "pm-dashboard", count: 2 });
    expect(live.orchestration).toEqual({
      generatedAt: "2026-10-26T17:00:00Z",
      runId: "orch-2026-10-26T17:00:00Z",
    });
  });

  test("programRefs covers the tracked refs and every entry ref", () => {
    const refs = programRefs();
    for (const r of TRACKED_REFS) expect(refs).toContain(r);
    expect(refs).toContain("developing-today/code#911");
    expect(new Set(refs).size).toBe(refs.length); // de-duplicated
  });
});

describe("renderProgram", () => {
  test("renders every section, a tracked ref, and next/long-term items from fixtures", async () => {
    const src = fixtureSource();
    const items = await src.items(programRefs());
    const data = buildProgramData(items, await src.projects(), await src.orchestration());
    const html = renderProgram(data, items, PROV);
    for (const heading of [
      "Program status",
      "What this is — architecture",
      "Live status",
      "What's built",
      "What's next",
      "Long-term roadmap",
      "Known gaps",
    ]) {
      expect(html).toContain(heading);
    }
    // A tracked-issue ref renders (resolved or honestly unresolved).
    expect(html).toContain("developing-today/code#910");
    expect(html).toContain("unresolved");
    // Next + long-term editorial items, each with its tracker named.
    expect(html).toContain("Wire the work-picking jobs to the orchestrator");
    expect(html).toContain("Cozystack");
    // Fixtures carry no pool and no orchestration: both say so.
    expect(html).toContain("No Project pool in the current feed");
    expect(html).toContain("no orchestration section in the current feed yet");
    // The design record + Project links are present.
    expect(html).toContain("2026-10-24T11-05-38Z_design_project-management-stack");
    expect(html).toContain("https://github.com/orgs/developing-today/projects/4");
  });

  test("renders resolved states, pool counts, and freshness when data exists", () => {
    const item: GhItem = {
      ref: "developing-today/code#910",
      kind: "issue",
      url: "https://github.com/developing-today/code/issues/910",
      title: "Org Project layer",
      state: "closed",
      labels: [],
      author: "dezren39",
      updatedAt: "2026-10-26T17:00:00Z",
    };
    const items = new Map([[item.ref, item]]);
    const pool = [poolItem({ status: "Done", area: "pm-dashboard" })];
    const section = {
      generatedAt: "2026-10-26T17:00:00Z",
      runId: "orch-2026-10-26T17:00:00Z",
    } as OrchestrationSection;
    const html = renderProgram(buildProgramData(items, pool, section), items, PROV);
    expect(html).toContain("Org Project layer");
    expect(html).toContain("1 items");
    expect(html).toContain("Done: 1");
    expect(html).toContain("orch-2026-10-26T17:00:00Z");
    expect(PROGRAM.done.length).toBeGreaterThan(0);
  });
});
