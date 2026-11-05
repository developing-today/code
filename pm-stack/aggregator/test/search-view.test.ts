import { describe, expect, test } from "bun:test";
import type { BoardItem, GhItem, Provenance, ProvenanceDetail, RefKey, RoadmapRow } from "../src/model.ts";
import { renderSearch } from "../src/render.ts";
import { searchWork } from "../src/search-view.ts";

const ref = "developing-today/code#1212" as RefKey;
const board: BoardItem[] = [
  {
    id: "kaneo:pm-stack/task-1",
    title: "Ship dashboard search",
    repo: "developing-today/code",
    source: "kaneo",
    status: "in-progress",
    links: [{ ref, relation: "implements" }],
  },
];
const rows: RoadmapRow[] = [
  {
    id: 4,
    subject: "grist:program/roadmap/7",
    title: "Aggregator findability",
    area: "Work",
    status: "In Progress",
    horizon: "November 2026",
    source: "Grist",
    notes: "Search across tracked issues",
    links: [{ ref, relation: "tracks" }],
  },
];
const projected = new Map<RefKey, GhItem>([
  [
    ref,
    {
      ref,
      kind: "issue",
      url: "https://github.com/developing-today/code/issues/1212",
      title: "Add global search",
      state: "open",
      labels: [],
      author: "octocat",
      updatedAt: "2026-11-01T00:00:00Z",
      resolution: "projector",
    },
  ],
]);
const provenance = {
  board: "live",
  roadmap: "live",
  items: "live",
  comments: "live",
  quota: "live",
  usage: "live",
  attention: "live",
  orchestration: "live",
  dns: "live",
  registry: "live",
  aa: "live",
} as Provenance;
const detail = Object.fromEntries(
  Object.entries(provenance).map(([key, text]) => [key, { text, state: "live" }]),
) as ProvenanceDetail;

describe("global search first slice", () => {
  test("finds canonical work by linked ref and projected GitHub title", () => {
    expect(searchWork(board, rows, projected, "developing-today/code#1212").map((r) => r.kind)).toEqual([
      "board",
      "roadmap",
    ]);
    expect(searchWork(board, rows, projected, "Add global search").map((r) => r.title)).toEqual([
      "Ship dashboard search",
      "Aggregator findability",
    ]);
  });

  test("returns an empty result for an unknown or blank query", () => {
    expect(searchWork(board, rows, projected, "nothing matches")).toEqual([]);
    expect(searchWork(board, rows, projected, "  ")).toEqual([]);
  });

  test("renders linked item state, global search, and freshness chips", () => {
    const results = searchWork(board, rows, projected, "1212");
    const html = renderSearch("1212", results, projected, provenance, detail);
    expect(html).toContain('action="/search"');
    expect(html).toContain("Search board and roadmap work");
    expect(html).toContain("Add global search");
    expect(html).toContain("developing-today/code#1212");
    expect(html).toContain("items · live");
  });
});
