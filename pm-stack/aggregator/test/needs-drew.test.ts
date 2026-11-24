import { describe, expect, test } from "bun:test";
import {
  buildNeedsDrewCensus,
  createNeedsDrewSweeper,
  extractNeedsDrewHands,
  extractNeedsDrewReason,
  needsDrewOwnerItems,
  needsDrewWait,
  needsDrewWaitCounts,
  needsDrewIncidentKey,
  needsDrewListUrl,
  type NeedsDrewFetchPage,
  shouldFetchNeedsDrewCensus,
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

/** Fake list endpoint keyed by (repo, label, page) from the request URL. */
function fakeFetch(
  handler: (repo: string, label: string, page: number) => { items: unknown[] } | Error,
): NeedsDrewFetchPage {
  return async (url: string) => {
    const u = new URL(url);
    const repo = /^\/repos\/([^/]+\/[^/]+)\/issues$/.exec(u.pathname)?.[1] ?? "";
    const label = u.searchParams.get("labels") ?? "";
    const page = Number(u.searchParams.get("page") ?? "1");
    const r = handler(repo, label, page);
    if (r instanceof Error) return { ok: false, json: { error: r.message } };
    return { ok: true, json: r.items };
  };
}

describe("needs-drew census (#1226)", () => {
  // #14103: the census reads the list endpoint, not the search API —
  // a search query with neither is:issue nor is:pull-request is a 422.
  test("list URL is open-only, per-label, paginated", () => {
    const url = needsDrewListUrl(REPO, "user/review", 2);
    const u = new URL(url);
    expect(u.pathname).toBe(`/repos/${REPO}/issues`);
    expect(u.searchParams.get("state")).toBe("open");
    expect(u.searchParams.get("labels")).toBe("user/review");
    expect(u.searchParams.get("per_page")).toBe("100");
    expect(u.searchParams.get("page")).toBe("2");
  });

  test("both labels censused; an item carrying both appears once with both", async () => {
    const both = item(7, ["user/review", "user/escalated"]);
    const fetchPage = fakeFetch((repo, label) => {
      if (repo !== REPO) return { items: [] };
      if (label === "user/review") return { items: [both, item(8, ["user/review"])] };
      return { items: [both] };
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
        ? { items: [item(9, ["user/escalated"], { pull_request: { url: "x" } })] }
        : { items: [] },
    );
    const census = await buildNeedsDrewCensus(fetchPage, { now: NOW });
    expect(census.items).toHaveLength(1);
    expect(census.items[0].kind).toBe("pr");
  });

  test("closed items in a payload are excluded even though the query is open-only", async () => {
    const fetchPage = fakeFetch((repo, label) =>
      repo === REPO && label === "user/review"
        ? { items: [item(10, ["user/review"]), item(11, ["user/review"], { state: "closed" })] }
        : { items: [] },
    );
    const census = await buildNeedsDrewCensus(fetchPage, { now: NOW });
    expect(census.items.map((i) => i.number)).toEqual([10]);
  });

  test("pagination walks pages until a short page ends the list", async () => {
    const page1 = Array.from({ length: 100 }, (_, i) => item(i + 1, ["user/review"]));
    const fetchPage = fakeFetch((repo, label, page) => {
      if (repo !== REPO || label !== "user/review") return { items: [] };
      return page === 1 ? { items: page1 } : { items: [item(101, ["user/review"])] };
    });
    const census = await buildNeedsDrewCensus(fetchPage, { now: NOW });
    expect(census.items.filter((i) => i.repo === REPO)).toHaveLength(101);
    const src = census.sources.find((x) => x.repo === REPO && x.label === "user/review");
    expect(src?.count).toBe(101);
    expect(src?.truncated).toBeUndefined();
    expect(census.complete).toBe(true);
  });

  test("a full page at the cap is truncated, not complete", async () => {
    const full = Array.from({ length: 100 }, (_, i) => item(i + 1, ["user/review"]));
    const fetchPage = fakeFetch((repo, label) => {
      if (repo !== REPO || label !== "user/review") return { items: [] };
      return { items: full };
    });
    const census = await buildNeedsDrewCensus(fetchPage, { now: NOW, repos: [REPO], labels: ["user/review"] });
    expect(census.items).toHaveLength(1000);
    expect(census.complete).toBe(false);
    expect(census.sources[0].count).toBe(1000);
    expect(census.sources[0].truncated).toBe(true);
  });

  test("a failed source is named, the census is incomplete, other sources still count", async () => {
    const fetchPage = fakeFetch((repo, label) => {
      if (repo === "dezren39/darwin-nix") return new Error("HTTP 403");
      if (repo === REPO && label === "user/review") return { items: [item(12, ["user/review"])] };
      return { items: [] };
    });
    const census = await buildNeedsDrewCensus(fetchPage, { now: NOW });
    expect(census.complete).toBe(false);
    expect(census.items.map((i) => i.number)).toEqual([12]);
    const nix = census.sources.filter((x) => x.repo === "dezren39/darwin-nix");
    expect(nix).toHaveLength(2);
    expect(nix.every((x) => !x.ok && x.error?.includes("403"))).toBe(true);
  });

  // #14103: a query GitHub refuses outright (a 4xx that is not a rate
  // limit) is a fault of the query, not a transient failure to wait out.
  test("a refused query is named as a fault of the query", async () => {
    const fetchPage: NeedsDrewFetchPage = async () => ({ ok: false, json: { status: 422 } });
    const census = await buildNeedsDrewCensus(fetchPage, { now: NOW, repos: [REPO], labels: ["user/review"] });
    expect(census.items).toHaveLength(0);
    expect(census.complete).toBe(false);
    expect(census.sources[0].ok).toBe(false);
    expect(census.sources[0].error).toContain("query refused by GitHub (HTTP 422)");
    // The page names the refusal, not just an incomplete census.
    const html = needsDrewCensusHtml(census);
    expect(html).toContain("Label census incomplete");
    expect(html).toContain("query refused by GitHub (HTTP 422)");
  });

  test("only a flagged page marks the source incomplete; a clean sweep stays complete", async () => {
    const clean = await buildNeedsDrewCensus(
      fakeFetch(() => ({ items: [] })),
      {
        now: NOW,
        repos: [REPO],
        labels: ["user/review"],
      },
    );
    expect(clean.complete).toBe(true);
    expect(clean.sources[0].incomplete).toBeUndefined();
  });

  // Codex P2 on PR #3970: contexts that discard the census must not pay
  // for it. Opting in is explicit, so a new caller cannot inherit the
  // external dependency by accident.
  test("the census is opt-in for attention contexts", () => {
    expect(shouldFetchNeedsDrewCensus()).toBe(false);
    expect(shouldFetchNeedsDrewCensus({})).toBe(false);
    expect(shouldFetchNeedsDrewCensus({ census: false })).toBe(false);
    expect(shouldFetchNeedsDrewCensus({ census: true })).toBe(true);
  });

  // Codex P2 on PR #3970: three concurrent renders must share one sweep.
  test("concurrent callers share one in-flight sweep (single-flight)", async () => {
    let calls = 0;
    let release: (() => void) | undefined;
    const gate = new Promise<void>((r) => {
      release = r;
    });
    const fetchPage: NeedsDrewFetchPage = async () => {
      calls++;
      await gate;
      return { ok: true, json: [] };
    };
    const sweep = createNeedsDrewSweeper(fetchPage, { repos: [REPO], labels: ["user/review"] });
    const a = sweep();
    const b = sweep();
    const c = sweep();
    expect(a).toBe(b);
    expect(b).toBe(c);
    release?.();
    const [ra, rb, rc] = await Promise.all([a, b, c]);
    expect(calls).toBe(1);
    expect(ra).toBe(rb);
    expect(rb).toBe(rc);
    // A fresh call inside the TTL is served from cache, not re-fetched.
    const again = await sweep();
    expect(again).toBe(ra);
    expect(calls).toBe(1);
  });

  test("the sweep is reused within the TTL and re-runs after it", async () => {
    let now = 1_000;
    let calls = 0;
    const fetchPage: NeedsDrewFetchPage = async () => {
      calls++;
      return { ok: true, json: [] };
    };
    const sweep = createNeedsDrewSweeper(
      fetchPage,
      { repos: [REPO], labels: ["user/review"] },
      { ttlMs: 100, now: () => now },
    );
    const first = await sweep();
    expect(calls).toBe(1);
    now += 50;
    expect(await sweep()).toBe(first);
    expect(calls).toBe(1);
    now += 200;
    const second = await sweep();
    expect(calls).toBe(2);
    expect(second).not.toBe(first);
  });

  // A sweep that fails every query is honest data (complete:false), and
  // keeping it cached for the TTL is what stops a rate-limited API from
  // being hammered on every render.
  test("a wholly failed sweep is marked incomplete and cached for the TTL", async () => {
    let calls = 0;
    const fetchPage: NeedsDrewFetchPage = async () => {
      calls++;
      return { ok: false, json: { status: 403 } };
    };
    const sweep = createNeedsDrewSweeper(fetchPage, { repos: [REPO], labels: ["user/review"] });
    const first = await sweep();
    expect(first.complete).toBe(false);
    expect(first.sources[0].ok).toBe(false);
    expect(await sweep()).toBe(first);
    expect(calls).toBe(1);
  });

  test("reason comes from an explicit escalation line, else null (honest fallback)", () => {
    expect(extractNeedsDrewReason("context\nEscalated: needs a product call\nmore")).toBe("needs a product call");
    expect(extractNeedsDrewReason("Reason: token expired")).toBe("token expired");
    expect(extractNeedsDrewReason("no stated reason here")).toBeNull();
    expect(extractNeedsDrewReason(null)).toBeNull();
    expect(extractNeedsDrewReason(42)).toBeNull();
  });
});

// FDR-0013 (doc/decisions): only a hands item may wait on a person.
// Oracle: the marker grammar documented on extractNeedsDrewHands and
// the four kinds `fleet queue block --hands` accepts.
describe("how an item waits", () => {
  test("the hands marker gives a kind and an optional action", () => {
    expect(
      extractNeedsDrewHands('text\n<!-- hands:v1 kind=interactive-login action="Run gh auth refresh." -->\nmore'),
    ).toEqual({ kind: "interactive-login", action: "Run gh auth refresh." });
    expect(extractNeedsDrewHands("<!-- hands:v1 kind=payment -->")).toEqual({ kind: "payment", action: null });
    // An empty action is no action.
    expect(extractNeedsDrewHands('<!-- hands:v1 kind=signature action="  " -->')).toEqual({
      kind: "signature",
      action: null,
    });
  });

  test("an unknown kind, another marker or no body is not a hands marker", () => {
    expect(extractNeedsDrewHands("<!-- hands:v1 kind=decision -->")).toBeNull();
    expect(extractNeedsDrewHands("<!-- decision:v1 -->")).toBeNull();
    expect(extractNeedsDrewHands("hands: please decide")).toBeNull();
    expect(extractNeedsDrewHands(null)).toBeNull();
    expect(extractNeedsDrewHands(42)).toBeNull();
  });

  test("a hands marker wins, then a review request, else an agent parked it", () => {
    const hands = { kind: "physical" as const, action: null };
    expect(needsDrewWait({ needsLabels: ["user/escalated"], hands })).toBe("hands");
    expect(needsDrewWait({ needsLabels: ["user/review"], hands })).toBe("hands");
    expect(needsDrewWait({ needsLabels: ["user/review", "user/escalated"], hands: null })).toBe("review");
    expect(needsDrewWait({ needsLabels: ["user/escalated"], hands: null })).toBe("agent-decides");
    // An item censused before the marker existed has no `hands` field.
    expect(needsDrewWait({ needsLabels: ["user/escalated"] })).toBe("agent-decides");
  });

  test("the census reads the marker from the item body and counts each kind of wait", async () => {
    const fetchPage = fakeFetch((repo, label) => {
      if (repo !== REPO) return { items: [] };
      if (label === "user/review") return { items: [item(1, ["user/review"])] };
      return {
        items: [
          item(2, ["user/escalated"]),
          item(3, ["user/escalated"], { body: '<!-- hands:v1 kind=physical action="Plug in the rack PDU." -->' }),
          item(4, ["user/escalated"], { body: "Escalated: which encoding?" }),
        ],
      };
    });
    const census = await buildNeedsDrewCensus(fetchPage, { now: NOW });
    expect(census.items.find((i) => i.number === 3)?.hands).toEqual({
      kind: "physical",
      action: "Plug in the rack PDU.",
    });
    expect(census.items.find((i) => i.number === 2)?.hands).toBeNull();
    expect(needsDrewWaitCounts(census)).toEqual({ hands: 1, review: 1, "agent-decides": 2 });
    expect(needsDrewOwnerItems(census).map((i) => i.number)).toEqual([1, 3]);
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

  test("an agent-parked item renders as fleet work, not in the owner's count", () => {
    // `base` holds one user/escalated item with no hands marker.
    const html = needsDrewCensusHtml(base);
    expect(html).toContain("Needs Drew — GitHub label census (0)");
    expect(html).toContain("Parked on the owner by an agent — fleet work (1)");
    expect(html).toContain("Needs a person's hands (0)");
    expect(html).toContain("Review requested (0)");
    expect(html).toContain(`${REPO}#7`);
    expect(html).toContain("no reason recorded on the item");
    expect(html).toContain("/api/needs-drew.json");
    expect(html).not.toContain("incomplete");
    // The parked item sits under the fleet-work heading, after it.
    expect(html.indexOf(`${REPO}#7`)).toBeGreaterThan(html.indexOf('id="needs-drew-parked"'));
  });

  test("hands items and review requests are the owner's count, each under its own heading", () => {
    const html = needsDrewCensusHtml({
      ...base,
      items: [
        ...base.items,
        {
          ...base.items[0],
          ref: `${REPO}#8`,
          number: 8,
          title: "sign in for the codespace surface",
          url: `https://github.com/${REPO}/issues/8`,
          hands: { kind: "interactive-login" as const, action: "Run gh auth refresh <now> and approve." },
        },
        {
          ...base.items[0],
          ref: `${REPO}#9`,
          number: 9,
          title: "review the release notes",
          url: `https://github.com/${REPO}/issues/9`,
          labels: ["user/review"],
          needsLabels: ["user/review" as const],
          reason: "release notes are ready",
        },
      ],
    });
    expect(html).toContain("Needs Drew — GitHub label census (2)");
    expect(html).toContain("Needs a person's hands (1)");
    expect(html).toContain("Review requested (1)");
    expect(html).toContain("Parked on the owner by an agent — fleet work (1)");
    // The one action is shown, escaped, with its kind.
    expect(html).toContain("<strong>interactive-login</strong>: Run gh auth refresh &lt;now&gt; and approve.");
    const at = (needle: string) => html.indexOf(needle);
    expect(at(`${REPO}#8`)).toBeGreaterThan(at('id="needs-drew-hands"'));
    expect(at(`${REPO}#8`)).toBeLessThan(at('id="needs-drew-review"'));
    expect(at(`${REPO}#9`)).toBeGreaterThan(at('id="needs-drew-review"'));
    expect(at(`${REPO}#9`)).toBeLessThan(at('id="needs-drew-parked"'));
  });

  test("failed sources render a banner naming repo and label", () => {
    const html = needsDrewCensusHtml({
      ...base,
      complete: false,
      sources: [
        {
          repo: "dezren39/darwin-nix",
          label: "user/review" as const,
          ok: false,
          count: 0,
          fetchedAt: NOW.toISOString(),
          error: "HTTP 403",
        },
      ],
    });
    expect(html).toContain("Label census incomplete");
    expect(html).toContain("dezren39/darwin-nix");
    expect(html).toContain("HTTP 403");
  });
});

describe("needsDrewIncidentKey (#8605)", () => {
  test("spells the incident key the raiser posts under", () => {
    // The fleet-queue poster files the question's alert incident with
    // key `needs-drew:<ledgerId>`; the answer path closes it by the
    // same spelling, so the two must never drift.
    expect(needsDrewIncidentKey("ledger-42")).toBe("needs-drew:ledger-42");
    expect(needsDrewIncidentKey("proof-8568-needs-drew")).toBe("needs-drew:proof-8568-needs-drew");
  });
});
