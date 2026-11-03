import { describe, expect, test } from "bun:test";
import type { GhItem, ProjectPoolItem, Provenance, RefKey } from "../src/model.ts";
import { buildEpicRollups } from "../src/epics.ts";
import { renderItem, renderProgramProject, renderProjects } from "../src/render.ts";
import { buildProjectDetail, projectConfigById } from "../src/project-config.ts";

const R = (n: number): RefKey => `developing-today/code#${n}`;

function item(partial: Partial<ProjectPoolItem> & { number: number }): ProjectPoolItem {
  return {
    id: `PVTI_${partial.number}`,
    url: "https://github.com/orgs/developing-today/projects/4?pane=issue&itemId=1",
    projectUrl: "https://github.com/orgs/developing-today/projects/4",
    ref: R(partial.number),
    repo: "developing-today/code",
    issueUrl: `https://github.com/developing-today/code/issues/${partial.number}`,
    title: `item ${partial.number}`,
    state: "open",
    contentKind: "issue",
    status: null,
    area: "pm-dashboard",
    horizon: null,
    kind: null,
    ...partial,
  };
}

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
  aa: "x",
};

function fixturePool(): ProjectPoolItem[] {
  return [
    item({ number: 1, title: "Epic one", kind: "Epic" }),
    item({ number: 2, epicRef: R(1), status: "In Progress", blockedBy: [R(99)] }),
    item({ number: 3, epicRef: R(1), status: "Done", state: "open" }),
    item({ number: 4, epicRef: R(1), status: "Todo", blockedBy: [R(5)] }),
    item({ number: 5, state: "closed" }),
    item({ number: 6, epicRef: R(1), state: "merged", status: "No longer used" }),
    item({ number: 7, title: "Epic without children", kind: "Epic" }),
    item({ number: 8 }),
  ];
}

describe("epic rollups", () => {
  test("groups only explicit epicRef children and counts done, status mix, and unresolved blockers", () => {
    const rollups = buildEpicRollups(fixturePool());
    expect(rollups.get(R(1))).toEqual({
      total: 4,
      done: 2,
      blocked: 1,
      statusMix: [
        { status: "Done", count: 1 },
        { status: "In Progress", count: 1 },
        { status: "No longer used", count: 1 },
        { status: "Todo", count: 1 },
      ],
    });
    expect(rollups.get(R(7))).toEqual({ total: 0, done: 0, blocked: 0, statusMix: [] });
  });

  test("renders rollups on project cards, the program epic list, and at the top of epic item pages", () => {
    const pool = fixturePool();
    const projectsHtml = renderProjects(pool, PROV);
    expect(projectsHtml).toContain("Epic rollup");
    expect(projectsHtml).toContain("2/4 done");
    expect(projectsHtml).toContain("Blocked: 1");
    expect(projectsHtml).toContain("0 resolved</strong> · completion unknown");
    expect(projectsHtml).toContain("Status mix: none");
    const nonEpicStart = projectsHtml.indexOf("<strong>item 8</strong>");
    const nonEpicCard = projectsHtml.slice(nonEpicStart, projectsHtml.indexOf("</div>", nonEpicStart));
    expect(nonEpicCard).not.toContain("Epic rollup");

    const config = projectConfigById("pm-dashboard")!;
    const detail = buildProjectDetail(config, new Map<RefKey, GhItem>(), pool, []);
    const programHtml = renderProgramProject(detail, new Map(), PROV, undefined, pool);
    expect(programHtml).toContain("Epic one");
    expect(programHtml).toContain("2/4 done");
    expect(programHtml).toContain("Blocked: 1");

    const itemHtml = renderItem("Epic one", R(1), [], [], new Map(), [], PROV, {
      dashComments: [],
      formRef: R(1),
      formOpts: { identities: [] },
      sessions: [],
      epicRollup: buildEpicRollups(pool).get(R(1)),
    });
    expect(itemHtml.indexOf("Epic rollup")).toBeLessThan(itemHtml.indexOf("Linked issues / PRs"));
    expect(itemHtml).toContain("2/4 done");
  });
});
