import { expect, test } from "bun:test";
import { fetchProjectSnapshot, PROJECT_QUERY } from "../src/projects.ts";
import { designBoardsFromFeed } from "../src/feed.ts";
import {
  boardFamilyPanels,
  renderDesignBoards,
  isBoardSnapshot,
  DESIGN_VIEWS,
  ROADMAP_VIEWS,
} from "../src/design-boards.ts";

const date = "2026-11-01T10:00:00.000Z";
const page = (number: number, nodes: any[], next = false) => ({
  data: {
    node: {
      number,
      title: `Project ${number}`,
      url: `https://github.com/orgs/developing-today/projects/${number}`,
      fields: { nodes: [] },
      views: {
        nodes:
          number === 5
            ? [{ id: "ideas", name: "Ideas board", layout: "BOARD_LAYOUT", filter: "" }]
            : [...DESIGN_VIEWS, ...ROADMAP_VIEWS].map((name) => ({
                id: name,
                name,
                layout: "BOARD_LAYOUT",
                filter: name.startsWith("Roadmap") ? "repo:developing-today/code" : "label:design",
              })),
      },
      items: {
        totalCount: number === 5 ? 2 : nodes.length,
        nodes,
        pageInfo: { hasNextPage: next, endCursor: next ? "cursor2" : null },
      },
    },
  },
});
const item = (id: string, draft = false) => ({
  id,
  databaseId: Number(id),
  type: draft ? "DRAFT_ISSUE" : "ISSUE",
  content: {
    title: `<idea ${id}>`,
    ...(draft
      ? {}
      : {
          number: Number(id),
          url: `https://github.com/developing-today/code/issues/${id}`,
          repository: { nameWithOwner: "developing-today/code" },
          labels: { nodes: [{ name: "design" }] },
        }),
  },
  fieldValues: {
    nodes: [
      { name: "Parked", field: { name: "Status" } },
      { name: "Atlas", field: { name: "Area" } },
      { name: "Security", field: { name: "Theme" } },
      { date: "2026-11-01", field: { name: "Start date" } },
      { date: "2026-12-01", field: { name: "Target date" } },
    ],
  },
});

test("#8864 oracle: paginated draft ideas survive without a linked issue; dates and titles retained", async () => {
  let calls = 0;
  const fetcher = (async (_url: unknown, init: any) => {
    const vars = JSON.parse(init.body).variables;
    expect(vars.id).toBe("ideas-id");
    expect(vars.cursor).toBe(calls === 0 ? null : "cursor2");
    return Response.json(page(5, [item(String(++calls), true)], calls === 1));
  }) as unknown as typeof fetch;
  const snapshot = await fetchProjectSnapshot("not-a-real-token", fetcher, date, "ideas-id");
  expect(calls).toBe(2);
  expect(snapshot.items).toHaveLength(0);
  expect(snapshot.board.items).toHaveLength(2);
  expect(snapshot.board.items[0]).toMatchObject({
    title: "<idea 1>",
    theme: "Security",
    status: "Parked",
    start: "2026-11-01",
    target: "2026-12-01",
  });
  expect(snapshot.board.items[0].url).toContain("projects/5?pane=issue&itemId=1");
  expect(isBoardSnapshot(snapshot.board)).toBe(true);
  expect(JSON.stringify(snapshot.board)).not.toContain("not-a-real-token");
});

test("#8864 oracle: thirteen named panels, exact wire item counts, escaped text, no client GitHub fetch", async () => {
  const a = await fetchProjectSnapshot(
    "x",
    (async () => Response.json(page(4, [item("1")]))) as unknown as typeof fetch,
    date,
  );
  const b = await fetchProjectSnapshot(
    "x",
    (async () => Response.json(page(5, [item("2", true), item("3", true)]))) as unknown as typeof fetch,
    date,
    "ideas-id",
  );
  const panels = boardFamilyPanels([a.board, b.board]);
  expect(panels).toHaveLength(13);
  expect(panels.find((p) => p.name === "Design Register")?.items).toHaveLength(1);
  expect(panels.find((p) => p.name === "Ideas board")?.items).toHaveLength(2);
  expect(panels.find((p) => p.name === "Roadmap — code")?.items).toHaveLength(1);
  const html = renderDesignBoards([a.board, b.board]);
  expect(html).toContain("Board data as of");
  expect(html).toContain(date);
  expect(html).toContain("Security");
  expect(html).toContain("Parked");
  expect(html).toContain("&lt;idea 1&gt;");
  expect(html).not.toContain("<idea 1>");
  expect(html).not.toContain("fetch(");
  expect(renderDesignBoards([])).toContain("unavailable");
});

test("#8864 query requests date fields and snapshot gate refuses malformed items", () => {
  expect(PROJECT_QUERY).toContain("ProjectV2ItemFieldDateValue");
  expect(isBoardSnapshot({ number: 5, generatedAt: date, items: [{}] })).toBe(false);
});

test("satellite board snapshots are accepted by registered URL; unrelated number-1 boards are not", () => {
  const base = { generatedAt: date, totalItems: 0, views: [], items: [] };
  expect(
    isBoardSnapshot({ ...base, number: 1, url: "https://github.com/orgs/developing-today-forks/projects/1" }),
  ).toBe(true);
  expect(isBoardSnapshot({ ...base, number: 1, url: "https://github.com/orgs/someone-else/projects/1" })).toBe(false);
});

test("#8864 exact design label, area-only panels, date-only roadmaps and unsupported filters", async () => {
  const wire = page(4, [item("1"), item("2"), item("3")]);
  wire.data.node.items.nodes[1].content.labels = { nodes: [{ name: "kind/design" }] };
  wire.data.node.items.nodes[2].fieldValues.nodes = [];
  const views = wire.data.node.views.nodes;
  views.find((v) => v.name === "Designs — Atlas")!.filter = 'area:"Atlas"';
  views.find((v) => v.name === "Designs — PM stack")!.filter = 'area:"PM stack"';
  views.find((v) => v.name === "Designs — Turso")!.filter = "is:issue";
  const { board } = await fetchProjectSnapshot("x", (async () => Response.json(wire)) as unknown as typeof fetch, date);
  const panels = boardFamilyPanels([board]);
  expect(panels.find((p) => p.name === "Design Register")?.items.map((i) => i.id)).toEqual(["1", "3"]);
  expect(panels.find((p) => p.name === "Designs — Atlas")?.items.map((i) => i.id)).toEqual(["1", "2"]);
  expect(panels.find((p) => p.name === "Designs — PM stack")?.items).toHaveLength(0);
  expect(panels.find((p) => p.name === "Roadmap — code")?.items).toHaveLength(2);
  expect(panels.find((p) => p.name === "Designs — Turso")?.problem).toContain("unsupported");
  expect(panels.find((p) => p.name === "Designs — Turso")?.items).toHaveLength(0);
  expect(renderDesignBoards([board])).toContain("stale (over 15 minutes)");
  expect(designBoardsFromFeed({ designBoards: [board] })).toEqual([board]);
  expect(
    designBoardsFromFeed({ designBoards: [{ ...board, items: [{ ...board.items[0], url: "javascript:alert(1)" }] }] }),
  ).toEqual([]);
  expect(isBoardSnapshot({ ...board, items: [board.items[0], board.items[0]] })).toBe(false);
  expect(isBoardSnapshot({ ...board, items: [{ ...board.items[0], start: "2026-02-30" }] })).toBe(false);
  expect(designBoardsFromFeed({})).toEqual([]);
});
