// Registry-link ref filter (#6211): a stored pmp_links row whose ref fails
// isValidRef (unsafe-integer number, or not owner/repo#N at all) bypasses
// the projector's POST gate when it is legacy/seeded data, and used to flow
// verbatim through projectorLinks into board items and roadmap rows, where
// renderSearch / project-page rendering threw in refUrl and 500'd the page.
// projectorLinks now drops invalid refs at the read boundary (with a logged
// warning); these tests pin the drop for both subject kinds and the render
// survival of /search with the filtered links attached.
//
// Run: bun test test/registry-ref-filter-6211.test.ts
import { describe, expect, test } from "bun:test";
import type { BoardItem, GhItem, Provenance, ProvenanceDetail, RefKey, RoadmapRow } from "../src/model.ts";
import { renderSearch } from "../src/render.ts";
import { searchWork } from "../src/search-view.ts";
import { projectorLinks } from "../src/sources/live.ts";

const GOOD = "developing-today/code#6211";
const BAD_UNSAFE = "developing-today/code#99999999999999999999";
const BAD_SHAPE = "not-a-ref";

const rowsBySubject: Record<string, { ref: string; relation: string }[]> = {
  "kaneo:board-item": [
    { ref: BAD_UNSAFE, relation: "relates" },
    { ref: GOOD, relation: "implements" },
  ],
  "grist:roadmap-row": [
    { ref: BAD_SHAPE, relation: "relates" },
    { ref: BAD_UNSAFE, relation: "relates" },
    { ref: GOOD, relation: "tracks" },
  ],
};

const realFetch = globalThis.fetch;
globalThis.fetch = (async (input: unknown) => {
  const url = new URL(String(input));
  const subject = url.searchParams.get("subject") ?? "";
  return {
    ok: true,
    json: async () => ({ links: rowsBySubject[subject] ?? [] }),
  } as Response;
}) as typeof fetch;

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

describe("projectorLinks ref filter (#6211)", () => {
  test("board subject: unsafe-integer registry ref dropped, good ref kept", async () => {
    const links = await projectorLinks("http://projector", "kaneo:board-item");
    expect(links).toEqual([{ ref: GOOD, relation: "implements" }]);
  });

  test("roadmap subject: non-GitHub-shaped and unsafe refs dropped, good ref kept", async () => {
    const links = await projectorLinks("http://projector", "grist:roadmap-row");
    expect(links).toEqual([{ ref: GOOD, relation: "tracks" }]);
  });

  test("/search renders a board item and roadmap row enriched from filtered registry links", async () => {
    const boardLinks = await projectorLinks("http://projector", "kaneo:board-item");
    const roadmapLinks = await projectorLinks("http://projector", "grist:roadmap-row");
    const board: BoardItem[] = [
      {
        id: "kaneo:pm-stack/task-6211",
        title: "Registry ref filter",
        repo: "developing-today/code",
        source: "kaneo",
        status: "in-progress",
        links: boardLinks,
      },
    ];
    const rows: RoadmapRow[] = [
      {
        id: 6211,
        subject: "grist:program/roadmap/6211",
        title: "Registry ref filter",
        area: "Work",
        status: "In Progress",
        horizon: "November 2026",
        source: "Grist",
        notes: "",
        links: roadmapLinks,
      },
    ];
    const projected = new Map<RefKey, GhItem>([
      [
        GOOD as RefKey,
        {
          ref: GOOD as RefKey,
          kind: "issue",
          url: "https://github.com/developing-today/code/issues/6211",
          title: "Unsafe-integer registry refs",
          state: "open",
          labels: [],
          author: "octocat",
          updatedAt: "2026-11-05T00:00:00Z",
          resolution: "projector",
        },
      ],
    ]);
    const results = searchWork(board, rows, projected, "Registry ref filter");
    expect(results.map((r) => r.kind)).toEqual(["board", "roadmap"]);
    const html = renderSearch("Registry ref filter", results, projected, provenance, detail);
    expect(html).toContain(GOOD);
    expect(html).not.toContain(BAD_UNSAFE);
    expect(html).not.toContain(BAD_SHAPE);
    globalThis.fetch = realFetch;
  });
});
