import { describe, expect, test } from "bun:test";
import {
  buildNeedsDrewCensus,
  extractNeedsDrewReason,
  needsDrewSearchUrl,
  type NeedsDrewFetchPage,
} from "../src/needs-drew.ts";
import { needsDrewCensusHtml } from "../src/render.ts";

const NOW = new Date("2026-11-01T12:00:00Z");
const REPO = "developing-today/code";

function item(number: number, labels: string[], extra: Record<string, unknown> = {}) {
  return {
    number,
    title: `item ${number}`,
    html_url: `https://github.com/${REPO}/issues/${number}`,
    state: "open",
    updated_at: "2026-11-01T10:00:00Z",
    body: "",
    labels: labels.map((name) => ({ name })),
    ...extra,
  };
}

/** Fake search API keyed by (repo, label, page) from the query URL. */
function fakeFetch(
  handler: (repo: string, label: string, page: number) => { total: number; items: unknown[] } | Error,
): NeedsDrewFetchPage {
  return async (url: string) => {
    const u = new URL(url);
    const q = u.searchParams.get("q") ?? "";
    const repo = /repo:(\S+)/.exec(q)?.[1] ?? "";
    const label = /label:"([^"]+)"/.exec(q)?.[1] ?? "";
    const page = Number(u.searchParams.get("page") ?? "1");
    const r = handler(repo, label, page);
    if (r instanceof Error) return { ok: false, json: { error: r.message } };
    return { ok: true, json: { total_count: r.total, items: r.items } };
  };
}

describe("needs-drew census (#1226)", () => {
  test("search URL is open-only, per-label, paginated", () => {
    const url = needsDrewSearchUrl(REPO, "user/review", 2);
    expect(url).toContain("page=2");
    const q = new URL(url).searchParams.get("q") ?? "";
    expect(q).toContain("state:open");
    expect(q).toContain('label:"user/review"');
    expect(q).toContain(`repo:${REPO}`);
  });

  test("both labels censused; an item carrying both appears once with both", async () => {
    const both = item(7, ["user/review", "user/escalated"]);
    const fetchPage = fakeFetch((repo, label) => {
      if (repo !== REPO) return { total: 0, items: [] };
      if (label === "user/review") return { total: 2, items: [both, item(8, ["user/review"])] };
      return { total: 1, items: [both] };
    });
    const census = await buildNeedsDrewCensus(fetchPage, { now: NOW });
    expect(census.complete).toBe(true);
    expect(census.fetchedAt).toBe(NOW.toISOString());
    const code = census.items.filter((i) => i.repo === REPO);
    expect(code.map((i) => i.number)).toEqual([7, 8]);
    expect(code[0].needsLabels).toEqual(["user/review", "user/escalated"]);
    expect(code[1].needsLabels).toEqual(["user/review"]);
  });

  test("PRs are included and distinguished from issues", async () => {
    const fetchPage = fakeFetch((repo, label) =>
      repo === REPO && label === "user/escalated"
        ? { total: 1, items: [item(9, ["user/escalated"], { pull_request: { url: "x" } })] }
        : { total: 0, items: [] },
    );
    const census = await buildNeedsDrewCensus(fetchPage, { now: NOW });
    expect(census.items).toHaveLength(1);
    expect(census.items[0].kind).toBe("pr");
  });

  test("closed items in a payload are excluded even though the query is open-only", async () => {
    const fetchPage = fakeFetch((repo, label) =>
      repo === REPO && label === "user/review"
        ? { total: 2, items: [item(10, ["user/review"]), item(11, ["user/review"], { state: "closed" })] }
        : { total: 0, items: [] },
    );
    const census = await buildNeedsDrewCensus(fetchPage, { now: NOW });
    expect(census.items.map((i) => i.number)).toEqual([10]);
  });

  test("pagination walks pages until total_count is covered", async () => {
    const page1 = Array.from({ length: 100 }, (_, i) => item(i + 1, ["user/review"]));
    const fetchPage = fakeFetch((repo, label, page) => {
      if (repo !== REPO || label !== "user/review") return { total: 0, items: [] };
      return page === 1 ? { total: 101, items: page1 } : { total: 101, items: [item(101, ["user/review"])] };
    });
    const census = await buildNeedsDrewCensus(fetchPage, { now: NOW });
    expect(census.items.filter((i) => i.repo === REPO)).toHaveLength(101);
    const src = census.sources.find((x) => x.repo === REPO && x.label === "user/review");
    expect(src?.count).toBe(101);
    expect(src?.truncated).toBeUndefined();
    expect(census.complete).toBe(true);
  });

  test("a failed source is named, the census is incomplete, other sources still count", async () => {
    const fetchPage = fakeFetch((repo, label) => {
      if (repo === "dezren39/nix") return new Error("HTTP 403");
      if (repo === REPO && label === "user/review") return { total: 1, items: [item(12, ["user/review"])] };
      return { total: 0, items: [] };
    });
    const census = await buildNeedsDrewCensus(fetchPage, { now: NOW });
    expect(census.complete).toBe(false);
    expect(census.items.map((i) => i.number)).toEqual([12]);
    const nix = census.sources.filter((x) => x.repo === "dezren39/nix");
    expect(nix).toHaveLength(2);
    expect(nix.every((x) => !x.ok && x.error?.includes("403"))).toBe(true);
  });

  test("reason comes from an explicit escalation line, else null (honest fallback)", () => {
    expect(extractNeedsDrewReason("context\nEscalated: needs a product call\nmore")).toBe("needs a product call");
    expect(extractNeedsDrewReason("Reason: token expired")).toBe("token expired");
    expect(extractNeedsDrewReason("no stated reason here")).toBeNull();
    expect(extractNeedsDrewReason(null)).toBeNull();
    expect(extractNeedsDrewReason(42)).toBeNull();
  });
});

describe("needs-drew census rendering", () => {
  const base = {
    fetchedAt: NOW.toISOString(),
    complete: true,
    sources: [{ repo: REPO, label: "user/review" as const, ok: true, count: 1, fetchedAt: NOW.toISOString() }],
    items: [
      {
        ref: `${REPO}#7`,
        repo: REPO,
        number: 7,
        kind: "issue" as const,
        title: "decide the thing",
        url: `https://github.com/${REPO}/issues/7`,
        labels: ["user/escalated"],
        needsLabels: ["user/escalated" as const],
        reason: null,
        updatedAt: "2026-11-01T10:00:00Z",
      },
    ],
  };

  test("undefined omits the section; null renders unavailable, not zero", () => {
    expect(needsDrewCensusHtml(undefined)).toBe("");
    expect(needsDrewCensusHtml(null)).toContain("Unavailable");
  });

  test("items render with the honest no-reason fallback and the API link", () => {
    const html = needsDrewCensusHtml(base);
    expect(html).toContain("Needs Drew — GitHub label census (1)");
    expect(html).toContain(`${REPO}#7`);
    expect(html).toContain("no reason recorded on the item");
    expect(html).toContain("/api/needs-drew.json");
    expect(html).not.toContain("incomplete");
  });

  test("failed sources render a banner naming repo and label", () => {
    const html = needsDrewCensusHtml({
      ...base,
      complete: false,
      sources: [
        { repo: "dezren39/nix", label: "user/review" as const, ok: false, count: 0, fetchedAt: NOW.toISOString(), error: "HTTP 403" },
      ],
    });
    expect(html).toContain("Label census incomplete");
    expect(html).toContain("dezren39/nix");
    expect(html).toContain("HTTP 403");
  });
});
