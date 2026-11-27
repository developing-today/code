import { describe, expect, test } from "bun:test";
import type { GhItem, Provenance, ProvenanceDetail, RefKey, SessionRecord } from "../src/model.ts";
import { renderSearch } from "../src/render.ts";
import {
  paginateSearchResults,
  parseSearchPagination,
  registrySearchIndexStats,
  registrySearchRefs,
  REGISTRY_SEARCH_MAX_FIELD_CHARS,
  REGISTRY_SEARCH_MAX_GRAM_KEYS,
  REGISTRY_SEARCH_MAX_POSTING_ENTRIES,
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
      parentId: null,
      status: "completed",
      model: "fixture-model",
      directory: `/workspace/session-${i}`,
      taskRefs: [],
    }));
    let visited = 0;
    const indexedRegistry = new Proxy(registry, {
      get(target, property, receiver) {
        if (typeof property === "string" && /^\d+$/.test(property)) visited++;
        return Reflect.get(target, property, receiver);
      },
    });
    // The first call builds any snapshot-level search data. The next
    // query must use that data instead of walking all 28,479 records.
    const expected = searchWork([], [], new Map<RefKey, GhItem>(), "needle", { registry: indexedRegistry });
    expect(expected.map((result) => result.id)).toEqual(matchingIds.map((i) => `needle-session-${i}`));

    const stats = registrySearchIndexStats(indexedRegistry);
    expect(stats.postingEntries).toBeLessThanOrEqual(REGISTRY_SEARCH_MAX_POSTING_ENTRIES);
    expect(stats.gramKeys).toBeLessThanOrEqual(REGISTRY_SEARCH_MAX_GRAM_KEYS);
    expect(stats.estimatedPostingBytes).toBeLessThanOrEqual(
      REGISTRY_SEARCH_MAX_POSTING_ENTRIES * Uint32Array.BYTES_PER_ELEMENT,
    );
    expect(stats.overflowRecords).toBe(0);
    visited = 0;
    const actual = searchWork([], [], new Map<RefKey, GhItem>(), "needle", { registry: indexedRegistry });
    expect(actual).toEqual(expected);
    expect(visited).toBeLessThanOrEqual(64);
  });

  test("overlong fields use exact overflow fallback within the index budget", () => {
    const registry: SessionRecord[] = [
      {
        id: "long-session",
        source: "opencode-sandbox",
        label: `session ${"x".repeat(REGISTRY_SEARCH_MAX_FIELD_CHARS + 1)} unique-marker`,
        agentId: "opencode:main",
        agentLabel: "OpenCode",
        parentId: null,
        status: "complete",
        taskRefs: [],
      },
    ];
    const stats = registrySearchIndexStats(registry);
    expect(stats.overflowRecords).toBe(1);
    expect(stats.estimatedPostingBytes).toBeLessThanOrEqual(
      REGISTRY_SEARCH_MAX_POSTING_ENTRIES * Uint32Array.BYTES_PER_ELEMENT,
    );
    const results = searchWork([], [], new Map<RefKey, GhItem>(), "unique-marker", { registry });
    expect(results.map((result) => result.id)).toEqual(["long-session"]);
  });

  test("indexed results equal the full-scan oracle across generated field values", () => {
    const terms = ["fleet search", "ordinary", "unicode 🛰 record", "registry", "dashboard"];
    const registry: SessionRecord[] = Array.from({ length: 256 }, (_, i) => ({
      id: `session-${i}`,
      source: "opencode-sandbox",
      label: `${terms[i % terms.length]} label ${i}`,
      title: i % 3 === 0 ? `Fleet task ${i}` : `title ${i}`,
      agentId: i % 2 === 0 ? "opencode:main" : "opencode:subagent",
      agentLabel: "OpenCode",
      parentId: null,
      status: i % 4 === 0 ? "in-progress" : "complete",
      model: i % 5 === 0 ? "gpt-fleet-model" : "other-model",
      directory: `/workspace/${terms[(i + 2) % terms.length]}/${i}`,
      taskRefs: i % 7 === 0 ? [`developing-today/code#${i}`] : [],
    }));
    registry.push(
      {
        id: "split-grams",
        source: "opencode-sandbox",
        label: "abc",
        title: "bcd",
        agentId: "cde",
        agentLabel: "def",
        parentId: null,
        status: "complete",
        taskRefs: [],
      },
      {
        id: "repeat-gram-only",
        source: "opencode-sandbox",
        label: "aaa",
        agentId: "opencode:main",
        agentLabel: "OpenCode",
        parentId: null,
        status: "complete",
        taskRefs: [],
      },
    );
    const queries = ["f", "fleet", "fle", "opencode:sub", "code#", "🛰", "in-progress", "abcdef", "aaaa", "missing"];
    const matches = (record: SessionRecord, query: string) =>
      [
        record.id,
        record.label,
        record.title,
        record.agentId,
        record.agentLabel,
        record.status,
        record.model,
        record.directory,
        ...record.taskRefs,
      ].some((value) => typeof value === "string" && value.toLowerCase().includes(query));

    for (const rawQuery of queries) {
      const query = rawQuery.trim().toLowerCase();
      const matched = registry.filter((record) => matches(record, query));
      const actual = searchWork([], [], new Map<RefKey, GhItem>(), rawQuery, { registry })
        .filter((result) => result.kind === "registry")
        .map((result) => result.id);
      expect(actual).toEqual(matched.map((record) => record.id));
      expect(registrySearchRefs(registry, rawQuery)).toEqual([
        ...new Set(matched.flatMap((record) => record.taskRefs)),
      ]);
    }
    expect(registrySearchRefs(registry, "   ")).toEqual([]);
  });
});
