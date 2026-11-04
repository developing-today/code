import { describe, expect, test } from "bun:test";
import { buildItemBrief } from "../src/brief.ts";
import { attentionSections, renderBoardBody, renderItem, renderRoadmap, type ItemExtras } from "../src/render.ts";
import type { AttentionView } from "../src/derive.ts";
import type { BoardItem, ProjectPoolItem, RoadmapRow, TierItem } from "../src/model.ts";
import type { Provenance } from "../src/model.ts";

const ref = "developing-today/code#1279";

const poolItem: ProjectPoolItem = {
  id: "PVTI_1279",
  url: "https://github.com/orgs/developing-today/projects/4?pane=issue&itemId=1279",
  projectUrl: "https://github.com/orgs/developing-today/projects/4",
  ref,
  repo: "developing-today/code",
  number: 1279,
  issueUrl: "https://github.com/developing-today/code/issues/1279",
  title: "Per-item brief",
  state: "open",
  contentKind: "issue",
  status: null,
  area: null,
  horizon: null,
  kind: "Task",
};

const tierItem: TierItem = {
  id: ref,
  kind: "task",
  title: "Per-item brief",
  source: "github",
  reason: "Needs orchestration",
  since: "2026-11-02T00:00:00Z",
  lastSeenAt: "2026-11-02T01:00:00Z",
  ref,
};

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

const briefMap = new Map([[ref, buildItemBrief(poolItem)]]);

describe("derived briefs on attention cards", () => {
  test("renders the same pool brief on a matching tier-item card", () => {
    const view: AttentionView = {
      drew: [],
      orchestrator: [],
      watchdog: [],
      drewExtra: [],
      orchestratorExtra: [tierItem],
      all: [],
      expiredCount: 0,
      tierSource: { present: true, state: "live" },
    };

    const sections = attentionSections(view, [], undefined, undefined, briefMap);

    expect(sections.queues).toContain("Derived brief");
    expect(sections.queues).toContain("https://github.com/developing-today/code/issues/1279");
    expect(sections.queues).toContain("Per-item brief");
  });

  test("renders pool briefs on board cards, roadmap cards, and linked item pages", () => {
    const boardItem: BoardItem = {
      id: "kaneo:board/task-1",
      title: "Board task",
      repo: "developing-today/code",
      source: "kaneo",
      status: "in-progress",
      links: [{ ref, relation: "tracks" }],
    };
    const boardHtml = renderBoardBody([boardItem], new Map(), undefined, { briefs: briefMap });

    const roadmapRow: RoadmapRow = {
      id: 1,
      subject: "grist:doc/table/1",
      title: "Roadmap task",
      area: "pm-dashboard",
      status: "In Progress",
      horizon: "2026-Q4",
      source: "fixture",
      notes: "",
      links: [{ ref, relation: "tracks" }],
    };
    const roadmapHtml = renderRoadmap(
      [roadmapRow],
      new Map(),
      provenance,
      "quarter",
      undefined,
      undefined,
      undefined,
      undefined,
      undefined,
      "",
      briefMap,
    );

    const extras: ItemExtras = {
      dashComments: [],
      formRef: roadmapRow.subject,
      formOpts: { identities: [] },
      sessions: [],
      briefs: briefMap,
    };
    const itemHtml = renderItem(
      "Roadmap task",
      roadmapRow.subject,
      roadmapRow.links,
      new Map(),
      [],
      provenance,
      extras,
    );

    for (const html of [boardHtml, roadmapHtml, itemHtml]) {
      expect(html).toContain("Derived brief");
      expect(html).toContain("https://github.com/developing-today/code/issues/1279");
    }
  });
});
