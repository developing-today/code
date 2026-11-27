import { describe, expect, test } from "bun:test";
import type { GhItem, Provenance, ProvenanceDetail, RefKey, SessionRecord } from "../src/model.ts";
import { renderSearch } from "../src/render.ts";
import {
  paginateSearchResults,
  parseSearchPagination,
  searchWork,
  SEARCH_DEFAULT_LIMIT,
  SEARCH_MAX_LIMIT,
  type SearchResult,
} from "../src/search-view.ts";

// #14060 slice 2 oracle: a page is exactly results[offset, offset+limit),
// total is the unpaginated match count, and limit is clamped to the cap
// no matter what the URL spells.
const results: SearchResult[] = Array.from({ length: 320 }, (_, i) => ({
  kind: "board" as const,
  id: `item-${i}`,
  title: `Fleet item ${i}`,
  summary: "developing-today/code · open",
  refs: [],
  href: `/item/item-${i}`,
}));

describe("parseSearchPagination (#14060)", () => {
  test("defaults when params are absent", () => {
    expect(parseSearchPagination(new URLSearchParams())).toEqual({ limit: SEARCH_DEFAULT_LIMIT, offset: 0 });
  });
  test("limit is clamped to the max and offset is honoured", () => {
    expect(parseSearchPagination(new URLSearchParams("limit=9999&offset=40"))).toEqual({
      limit: SEARCH_MAX_LIMIT,
      offset: 40,
    });
  });
  test("non-decimal spellings fall back instead of prefix-parsing", () => {
    expect(parseSearchPagination(new URLSearchParams("limit=10junk&offset=1e2"))).toEqual({
      limit: SEARCH_DEFAULT_LIMIT,
      offset: 0,
    });
  });
});

describe("paginateSearchResults (#14060)", () => {
  test("first page is bounded and total reports every match", () => {
    const page = paginateSearchResults(results, { limit: 50, offset: 0 });
    expect(page.page).toHaveLength(50);
    expect(page.page[0].id).toBe("item-0");
    expect(page.total).toBe(320);
    expect(page.remainingAfterPage).toBe(270);
  });
  test("last page is partial and reports nothing remaining", () => {
    const page = paginateSearchResults(results, { limit: 50, offset: 300 });
    expect(page.page).toHaveLength(20);
    expect(page.remainingAfterPage).toBe(0);
  });
});

describe("renderSearch bounded page (#14060)", () => {
  test("heading shows the true total, not the page size", () => {
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
    const html = renderSearch("fleet", results.slice(0, 50), new Map<RefKey, GhItem>(), provenance, detail, 320);
    expect(html).toContain("320 results for");
    expect(html).toContain("Showing the first 50");
  });
});

describe("search registry candidate bound (#15785)", () => {
  test("repeated queries over a large registry visit only matching candidates", () => {
    const recordCount = 28_479;
    const matchingIds = [137, 9_001, 24_101];
    const registry: SessionRecord[] = Array.from({ length: recordCount }, (_, i) => ({
      id: matchingIds.includes(i) ? `needle-session-${i}` : `session-${i}`,
      source: "opencode-sandbox",
      label: matchingIds.includes(i) ? `needle result ${i}` : `ordinary session ${i}`,
      title: `session title ${i}`,
      agentId: "opencode",
      agentLabel: "OpenCode",
      status: "completed",
      model: "fixture-model",
      directory: `/workspace/session-${i}`,
      taskRefs: [],
    }));
    const args = [[], [], new Map<RefKey, GhItem>()] as const;

    // The first call builds any snapshot-level search data. The next
    // query must use that data instead of walking all 28,479 records.
    const expected = searchWork(...args, "needle", { registry });
    expect(expected.map((result) => result.id)).toEqual(matchingIds.map((i) => `needle-session-${i}`));

    let visited = 0;
    const originalIterator = registry[Symbol.iterator].bind(registry);
    Object.defineProperty(registry, Symbol.iterator, {
      configurable: true,
      value: function* () {
        for (const record of originalIterator()) {
          visited++;
          yield record;
        }
      },
    });

    const actual = searchWork(...args, "needle", { registry });
    expect(actual).toEqual(expected);
    expect(visited).toBeLessThanOrEqual(64);
  });
});
