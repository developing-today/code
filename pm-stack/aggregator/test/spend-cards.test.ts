import { describe, expect, test } from "bun:test";
import { projectsBodyHtml, renderRoadmap } from "../src/render.ts";
import type { Provenance, RoadmapRow } from "../src/model.ts";
import { buildSpendCards, spendCardHtml, unattributedSpendHtml } from "../src/spend-cards.ts";
import { buildSpendRollups, type SpendRollup, type SpendRollupEvent } from "../src/spend-rollups.ts";
import type { ProjectPoolItem, RefKey } from "../src/model.ts";

// #4824 acceptance: complete + spend present, forecast absent,
// unattributed present, partial-window labelling — and no view
// computes a merged dollar figure from tokens or share-of-window.
// Records are built through the real #4823 builder so the card view
// is tested against the store's actual record shape.

const EPIC = "developing-today/code#100";
const FULL_MONTH = [{ from: "2026-09-01T00:00:00Z", to: "2026-12-01T00:00:00Z", status: "ok" as const }];

function rollups(events: SpendRollupEvent[], coverage = FULL_MONTH): SpendRollup[] {
  return buildSpendRollups(events, {
    attributionVersion: "v1",
    coverageStart: "2026-09-01",
    coverage,
  });
}

const NOV = "2026-11-10T12:00:00Z";

describe("buildSpendCards (#4824)", () => {
  test("complete window: three unit families stay separate labelled measures", () => {
    const view = buildSpendCards(
      rollups([
        {
          ts: NOV,
          projectId: null,
          epicRef: EPIC,
          tokensByPool: { main: 1200 },
          paygUsd: 34.5,
          sharePctByPool: { main: 12.34 },
        },
      ]),
    );
    const card = view.byScope.get(EPIC);
    expect(card).toBeDefined();
    expect(card?.coverage).toBe("complete");
    expect(card?.measures).toEqual([
      { label: "Tokens · pool main", value: "1,200" },
      { label: "PAYG dollars", value: "$34.50" },
      { label: "Plan-pool share of window · pool main", value: "12.3%" },
    ]);
    const html = spendCardHtml(card as NonNullable<typeof card>);
    expect(html).toContain("Tokens · pool main");
    expect(html).toContain("PAYG dollars");
    expect(html).toContain("Plan-pool share of window · pool main");
    // No merged/total figure anywhere in the rendering.
    expect(html.toLowerCase()).not.toContain("total");
  });

  test("forecast absent renders unknown; supplied forecast renders value + confidence", () => {
    const records = rollups([{ ts: NOV, projectId: null, epicRef: EPIC, paygUsd: 10 }]);
    const absent = buildSpendCards(records).byScope.get(EPIC);
    expect(absent?.forecast).toBeNull();
    expect(spendCardHtml(absent as NonNullable<typeof absent>)).toContain("Forecast to complete: unknown");
    const supplied = buildSpendCards(records, {
      forecasts: new Map([[EPIC, { usd: 55.5, confidence: "medium" }]]),
    }).byScope.get(EPIC);
    const html = spendCardHtml(supplied as NonNullable<typeof supplied>);
    expect(html).toContain("Forecast to complete: $55.50 (confidence: medium)");
  });

  test("unknown charges render PAYG as unknown, never zero", () => {
    const view = buildSpendCards(
      rollups([{ ts: NOV, projectId: null, epicRef: EPIC, tokensByPool: { main: 5 }, paygUsd: null }]),
    );
    const card = view.byScope.get(EPIC);
    expect(card?.measures.find((m) => m.label === "PAYG dollars")?.value).toBe("unknown");
  });

  test("partial window is labelled partial with its coverage start", () => {
    const view = buildSpendCards(rollups([{ ts: NOV, projectId: null, epicRef: EPIC, tokensByPool: { main: 5 } }], []));
    const card = view.byScope.get(EPIC);
    expect(card?.coverage).toBe("partial");
    const html = spendCardHtml(card as NonNullable<typeof card>);
    expect(html).toContain("partial window");
    expect(html).toContain("2026-09-01");
  });

  test("unattributed bucket is its own card with the reason breakdown one click away", () => {
    const view = buildSpendCards(
      rollups([{ ts: NOV, projectId: null, epicRef: null, tokensByPool: { main: 77 }, paygUsd: 3 }]),
      { unattributedReasons: [{ reason: "no-binding", text: "77 tokens" }] },
    );
    expect(view.byScope.size).toBe(0);
    expect(view.unattributed).not.toBeNull();
    const html = unattributedSpendHtml(view.unattributed as NonNullable<typeof view.unattributed>);
    expect(html).toContain("Unattributed spend");
    expect(html).toContain("<details");
    expect(html).toContain("Reason-code breakdown");
    expect(html).toContain("no-binding: 77 tokens");
  });

  test("selected attribution version is preserved on the card and displayed beside the window label (#8854)", () => {
    // Oracle: the version string is the literal the records were built
    // under (buildSpendRollups stamps it), not a value re-derived from
    // the card under test.
    const view = buildSpendCards(
      rollups([{ ts: NOV, projectId: null, epicRef: EPIC, tokensByPool: { main: 5 }, paygUsd: 2 }]),
    );
    const card = view.byScope.get(EPIC);
    expect(card?.attributionVersion).toBe("v1");
    const html = spendCardHtml(card as NonNullable<typeof card>);
    expect(html).toContain("attribution v1");
    const unattr = buildSpendCards(rollups([{ ts: NOV, projectId: null, epicRef: null, tokensByPool: { main: 7 } }]));
    expect(unattr.unattributed?.attributionVersion).toBe("v1");
    expect(unattributedSpendHtml(unattr.unattributed as NonNullable<typeof unattr.unattributed>)).toContain(
      "attribution v1",
    );
  });

  test("a restatement displays the newer selected version, not the superseded one (#8854)", () => {
    const events: SpendRollupEvent[] = [
      { ts: NOV, projectId: null, epicRef: EPIC, tokensByPool: { main: 5 }, paygUsd: 2 },
    ];
    const v1 = buildSpendRollups(events, {
      attributionVersion: "v1",
      coverageStart: "2026-09-01",
      coverage: FULL_MONTH,
    });
    const v2 = buildSpendRollups(events, {
      attributionVersion: "v2",
      coverageStart: "2026-09-01",
      coverage: FULL_MONTH,
    });
    const card = buildSpendCards([...v1, ...v2]).byScope.get(EPIC);
    expect(card?.attributionVersion).toBe("v2");
    const html = spendCardHtml(card as NonNullable<typeof card>);
    expect(html).toContain("attribution v2");
    expect(html).not.toContain("attribution v1");
  });

  test("an empty store yields no cards at all", () => {
    const view = buildSpendCards([]);
    expect(view.windowKind).toBeNull();
    expect(view.byScope.size).toBe(0);
    expect(view.unattributed).toBeNull();
  });
});

const R = (n: number): RefKey => `developing-today/code#${n}` as RefKey;

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

describe("Projects pool cards render spend beside progress (#4824)", () => {
  test("epic card shows its rollup and its spend; unattributed row is visible", () => {
    const pool = [
      item({ number: 100, title: "Epic one", kind: "Epic" }),
      item({ number: 101, epicRef: R(100), state: "closed" }),
      item({ number: 102, epicRef: R(100) }),
    ];
    const spend = buildSpendCards(
      rollups([
        { ts: NOV, projectId: null, epicRef: "developing-today/code#100", tokensByPool: { main: 42 }, paygUsd: 9.5 },
        { ts: NOV, projectId: null, epicRef: null, tokensByPool: { main: 7 } },
      ]),
      { unattributedReasons: [{ reason: "no-worktree", text: "7 tokens" }] },
    );
    const html = projectsBodyHtml(pool, { spend });
    expect(html).toContain("Epic rollup");
    expect(html).toContain("Spend");
    expect(html).toContain("Tokens · pool main");
    expect(html).toContain("$9.50");
    expect(html).toContain("Unattributed spend");
    expect(html).toContain("no-worktree: 7 tokens");
    // Without spend data the cards render exactly as before.
    const bare = projectsBodyHtml(pool, {});
    expect(bare).not.toContain("Unattributed spend");
  });

  test("roadmap view renders the unattributed row too (both views)", () => {
    const row: RoadmapRow = {
      id: 1,
      subject: "grist:doc/table/1",
      title: "Roadmap row",
      area: "pm-dashboard",
      status: "In progress",
      horizon: "Q4 2026",
      source: "fixture",
      notes: "",
      links: [],
    };
    const prov = {
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
    } as Provenance;
    const spend = buildSpendCards(rollups([{ ts: NOV, projectId: null, epicRef: null, tokensByPool: { main: 7 } }]));
    const html = renderRoadmap(
      [row],
      new Map(),
      prov,
      "quarter",
      undefined,
      undefined,
      undefined,
      undefined,
      undefined,
      "",
      undefined,
      undefined,
      undefined,
      spend,
    );
    expect(html).toContain("Unattributed spend");
    const bare = renderRoadmap([row], new Map(), prov, "quarter");
    expect(bare).not.toContain("Unattributed spend");
  });
});
