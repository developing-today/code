import { describe, expect, test } from "bun:test";
import type { BoardItem, DnsSection, GhItem, Provenance, ProvenanceDetail, RefKey, RoadmapRow, SaasService, SessionRecord, TierItem } from "../src/model.ts";
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
    expect(html).toContain("Search board items, roadmap rows, registry sessions, DNS domains, services, and tier items");
    expect(html).toContain("Add global search");
    expect(html).toContain("developing-today/code#1212");
    expect(html).toContain("items · live");
  });
});

const registry: SessionRecord[] = [
  {
    id: "ses_searchfixture",
    source: "opencode-laptop",
    label: "Search fixture session",
    agentId: "opencode:main",
    agentLabel: "OpenCode main",
    parentId: null,
    status: "in-progress",
    taskRefs: [ref],
  },
];
const dns: DnsSection = {
  generatedAt: "2026-11-01T00:00:00Z",
  source: "fixture register",
  totalAnnualListUsd: 12,
  entries: [
    {
      domain: "searchfixture.example",
      registrar: "Fixture Registrar",
      dnsHost: "Fixture DNS",
      renewal: "2027-01-15",
      autoRenew: "ON",
      priceUsd: 12,
      priorYearCost: "$12",
      used: "fixture",
      plan: "active use",
      review: "no",
      status: "active",
    },
  ],
};
const services: SaasService[] = [
  {
    id: "searchfixture-db",
    name: "Searchfixture DB",
    group: "database",
    plan: "Free",
    status: "active",
    metrics: [],
    paygEstimate: null,
    measuredAt: "2026-11-01T00:00:00Z",
    sourceClass: "api",
    sourceNote: "fixture",
    cadenceDays: 7,
    conflicts: [],
    reads: [],
  },
];
const tierItems: TierItem[] = [
  {
    id: "tier-searchfixture",
    kind: "task",
    title: "Searchfixture tier task",
    source: "opencode-laptop",
    state: "blocked",
    reason: "fixture reason",
    since: "2026-11-01T00:00:00Z",
    lastSeenAt: "2026-11-01T00:00:00Z",
    ref,
  },
];

describe("global search slice 2 (#1212)", () => {
  const extras = { registry, dns, services, tierItems };

  test("finds registry sessions by label and task ref, linking to the session view", () => {
    const byLabel = searchWork(board, rows, projected, "Search fixture session", extras);
    expect(byLabel.map((r) => r.kind)).toEqual(["registry"]);
    expect(byLabel[0].href).toBe("/sessions?id=ses_searchfixture#ses_searchfixture");
    const byRef = searchWork([], [], projected, "developing-today/code#1212", extras);
    expect(byRef.map((r) => r.kind)).toContain("registry");
  });

  test("finds DNS entries by domain, linking to the register row anchor", () => {
    const results = searchWork([], [], projected, "searchfixture.example", extras);
    expect(results.map((r) => r.kind)).toEqual(["dns"]);
    expect(results[0].href).toBe("/dns#dns-searchfixture.example");
  });

  test("finds services by name, linking to the service detail page", () => {
    const results = searchWork([], [], projected, "Searchfixture DB", extras);
    expect(results.map((r) => r.kind)).toEqual(["service"]);
    expect(results[0].href).toBe("/quota/service/searchfixture-db");
  });

  test("finds tier items by title, linking to their object discussion page", () => {
    const results = searchWork([], [], projected, "Searchfixture tier task", extras);
    expect(results.map((r) => r.kind)).toEqual(["tier"]);
    expect(results[0].href).toBe("/discuss/gh%3Adeveloping-today%2Fcode%231212");
  });

  test("board and roadmap results keep their /item/ detail links", () => {
    const results = searchWork(board, rows, projected, "1212", extras);
    const work = results.filter((r) => r.kind === "board" || r.kind === "roadmap");
    expect(work.map((r) => r.href)).toEqual([
      "/item/kaneo%3Apm-stack%2Ftask-1",
      "/item/grist%3Aprogram%2Froadmap%2F7",
    ]);
  });

  test("renders the new kinds with their home freshness facets", () => {
    const results = searchWork([], [], projected, "searchfixture", extras);
    expect(results.map((r) => r.kind).sort()).toEqual(["dns", "registry", "service", "tier"]);
    const html = renderSearch("searchfixture", results, projected, provenance, detail);
    expect(html).toContain("/quota/service/searchfixture-db");
    expect(html).toContain("/dns#dns-searchfixture.example");
    expect(html).toContain("dns · live");
    expect(html).toContain("registry · live");
  });

  test("extras are optional: first-slice calls behave as before", () => {
    expect(searchWork(board, rows, projected, "1212").map((r) => r.kind)).toEqual(["board", "roadmap"]);
  });
});
