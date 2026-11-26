import { describe, expect, test } from "bun:test";
import {
  buildNeedsDrewCensus,
  createNeedsDrewSweeper,
  extractNeedsDrewHands,
  extractNeedsDrewReason,
  NEEDS_DREW_LABELS,
  needsDrewOwnerItems,
  needsDrewWait,
  needsDrewWaitCounts,
  needsDrewIncidentKey,
  needsDrewListUrl,
  needsDrewSweepTtlMs,
  type NeedsDrewFetchPage,
  shouldFetchNeedsDrewCensus,
} from "../src/needs-drew.ts";
import { REPOS } from "../src/model.ts";
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

/** GitHub's list of a repository's open issues carrying a label, keyed
 *  by (repo, label, page) from the URL asked. Anything that is not
 *  that list is answered as GitHub answers some tokens today (a
 *  GitHub App's): a search of issues that names neither `is:issue` nor
 *  `is:pull-request` is a 422 (#14103), and any other path is not
 *  found. */
function fakeFetch(handler: (repo: string, label: string, page: number) => unknown[] | Error): NeedsDrewFetchPage {
  return async (url: string) => {
    const u = new URL(url);
    if (u.pathname === "/search/issues") {
      const q = u.searchParams.get("q") ?? "";
      return { ok: false, json: { status: /\bis:(issue|pull-request)\b/.test(q) ? 501 : 422 } };
    }
    const repo = /^\/repos\/([^/]+\/[^/]+)\/issues$/.exec(u.pathname)?.[1];
    if (!repo || u.searchParams.get("state") !== "open") return { ok: false, json: { status: 404 } };
    const label = u.searchParams.get("labels") ?? "";
    const page = Number(u.searchParams.get("page") ?? "1");
    const r = handler(repo, label, page);
    if (r instanceof Error) return { ok: false, json: { error: r.message } };
    return { ok: true, json: r };
  };
}

describe("needs-drew census (#1226)", () => {
  test("the URL asked is the repository's own list: open-only, per-label, paginated, and not the search", () => {
    const u = new URL(needsDrewListUrl(REPO, "user/review", 2));
    expect([u.origin, u.pathname]).toEqual(["https://api.github.com", `/repos/${REPO}/issues`]);
    expect(Object.fromEntries(u.searchParams)).toEqual({
      state: "open",
      labels: "user/review",
      per_page: "100",
      page: "2",
    });
  });

  // #14103: the census asked the search for `repo:… state:open label:"…"`,
  // and GitHub answers a search of issues that names neither `is:issue`
  // nor `is:pull-request` with a 422 for some tokens. With such a token
  // every query would fail and the queue read as empty and incomplete
  // (the token the dashboard runs with is not one: there the search
  // answered). The fake above answers as GitHub does such a token.
  test("asked of GitHub as it answers today, the census gets its items", async () => {
    const fetchPage = fakeFetch((repo, label) =>
      repo === REPO && label === "user/review"
        ? [item(21, ["user/review"]), item(22, ["user/review"], { pull_request: { url: "x" } })]
        : [],
    );
    const census = await buildNeedsDrewCensus(fetchPage, { now: NOW });
    expect(census.complete).toBe(true);
    expect(census.sources.every((x) => x.ok)).toBe(true);
    expect(census.items.map((i) => [i.number, i.kind])).toEqual([
      [21, "issue"],
      [22, "pr"],
    ]);
    // The form it used to ask in is what that fake refuses, as GitHub does.
    const old = `https://api.github.com/search/issues?q=${encodeURIComponent(`repo:${REPO} state:open label:"user/review"`)}&per_page=100&page=1`;
    expect(await fetchPage(old)).toEqual({ ok: false, json: { status: 422 } });
  });

  test("both labels censused; an item carrying both appears once with both", async () => {
    const both = item(7, ["user/review", "user/escalated"]);
    const fetchPage = fakeFetch((repo, label) => {
      if (repo !== REPO) return [];
      if (label === "user/review") return [both, item(8, ["user/review"])];
      return [both];
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
      repo === REPO && label === "user/escalated" ? [item(9, ["user/escalated"], { pull_request: { url: "x" } })] : [],
    );
    const census = await buildNeedsDrewCensus(fetchPage, { now: NOW });
    expect(census.items).toHaveLength(1);
    expect(census.items[0].kind).toBe("pr");
  });

  test("closed items in a payload are excluded even though the query is open-only", async () => {
    const fetchPage = fakeFetch((repo, label) =>
      repo === REPO && label === "user/review"
        ? [item(10, ["user/review"]), item(11, ["user/review"], { state: "closed" })]
        : [],
    );
    const census = await buildNeedsDrewCensus(fetchPage, { now: NOW });
    expect(census.items.map((i) => i.number)).toEqual([10]);
  });

  test("pagination walks pages until one is not full", async () => {
    const page1 = Array.from({ length: 100 }, (_, i) => item(i + 1, ["user/review"]));
    const asked: number[] = [];
    const fetchPage = fakeFetch((repo, label, page) => {
      if (repo !== REPO || label !== "user/review") return [];
      asked.push(page);
      return page === 1 ? page1 : [item(101, ["user/review"])];
    });
    const census = await buildNeedsDrewCensus(fetchPage, { now: NOW });
    expect(census.items.filter((i) => i.repo === REPO)).toHaveLength(101);
    const src = census.sources.find((x) => x.repo === REPO && x.label === "user/review");
    expect(src?.count).toBe(101);
    expect(src?.truncated).toBeUndefined();
    expect(census.complete).toBe(true);
    expect(asked).toEqual([1, 2]);
  });

  test("a last page that is exactly full is followed by one more, which is empty: whole, not cut short", async () => {
    const asked: number[] = [];
    const fetchPage = fakeFetch((_repo, _label, page) => {
      asked.push(page);
      return page <= 2 ? Array.from({ length: 100 }, (_, i) => item((page - 1) * 100 + i + 1, ["user/review"])) : [];
    });
    const census = await buildNeedsDrewCensus(fetchPage, { now: NOW, repos: [REPO], labels: ["user/review"] });
    expect([census.items.length, census.complete, census.sources[0].truncated]).toEqual([200, true, undefined]);
    expect(asked).toEqual([1, 2, 3]);
  });

  // The cap is ten pages. A tenth page that is not full is the end of
  // the list like any other: everything was read, and saying "there
  // may be more" of it would mark a whole census as a floor.
  test("nine full pages and a tenth that is not full: whole, and not said to be cut short", async () => {
    const asked: number[] = [];
    const fetchPage = fakeFetch((_repo, _label, page) => {
      asked.push(page);
      return Array.from({ length: page < 10 ? 100 : 7 }, (_, i) => item((page - 1) * 100 + i + 1, ["user/review"]));
    });
    const census = await buildNeedsDrewCensus(fetchPage, { now: NOW, repos: [REPO], labels: ["user/review"] });
    expect([census.items.length, census.complete, census.sources[0].truncated]).toEqual([907, true, undefined]);
    expect(asked).toEqual([1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
  });

  test("more than ten full pages: the count is a floor and the census says so", async () => {
    const asked: number[] = [];
    const fetchPage = fakeFetch((_repo, _label, page) => {
      asked.push(page);
      return Array.from({ length: 100 }, (_, i) => item((page - 1) * 100 + i + 1, ["user/review"]));
    });
    const census = await buildNeedsDrewCensus(fetchPage, { now: NOW, repos: [REPO], labels: ["user/review"] });
    expect([census.items.length, census.complete, census.sources[0].truncated]).toEqual([1000, false, true]);
    expect(asked).toHaveLength(10);
    expect(needsDrewCensusHtml(census)).toContain("Label census incomplete");
  });

  test("a failed source is named, the census is incomplete, other sources still count", async () => {
    const fetchPage = fakeFetch((repo, label) => {
      if (repo === "dezren39/darwin-nix") return new Error("HTTP 403");
      if (repo === REPO && label === "user/review") return [item(12, ["user/review"])];
      return [];
    });
    const census = await buildNeedsDrewCensus(fetchPage, { now: NOW });
    expect(census.complete).toBe(false);
    expect(census.items.map((i) => i.number)).toEqual([12]);
    const nix = census.sources.filter((x) => x.repo === "dezren39/darwin-nix");
    expect(nix).toHaveLength(2);
    expect(nix.every((x) => !x.ok && x.error?.includes("403"))).toBe(true);
  });

  // The same worry as Codex P2 on PR #3970 (a 200 that is not the whole
  // answer must not read as an empty queue), for the list: an answer
  // under a 200 that is not a list is a failed query.
  test("an answer that is not a list is a failed source, never an empty queue", async () => {
    for (const json of [{ total_count: 0, items: [] }, { message: "Not Found" }, null, "[]", 0]) {
      const fetchPage: NeedsDrewFetchPage = async () => ({ ok: true, json });
      const census = await buildNeedsDrewCensus(fetchPage, { now: NOW, repos: [REPO], labels: ["user/review"] });
      expect(census.items).toHaveLength(0);
      expect(census.complete).toBe(false);
      expect([census.sources[0].ok, census.sources[0].error]).toEqual([false, "the answer was not a list"]);
      // The page must warn, not present an authoritative empty queue.
      expect(needsDrewCensusHtml(census)).toContain("Label census incomplete");
    }
  });

  // A census that carries the search API's old "incomplete results"
  // flag (the list never sets it) is still rendered as a floor.
  test("a source marked as having incomplete results is rendered as a floor", () => {
    const html = needsDrewCensusHtml({
      fetchedAt: NOW.toISOString(),
      items: [],
      sources: [
        { repo: REPO, label: "user/review", ok: true, count: 0, fetchedAt: NOW.toISOString(), incomplete: true },
      ],
      complete: false,
    });
    expect(html).toContain("Label census incomplete");
    expect(html).toContain("incomplete results");
    expect(html).toContain("in the sources that answered");
  });

  test("a clean sweep is complete, and nothing in it is marked a floor", async () => {
    const clean = await buildNeedsDrewCensus(
      fakeFetch(() => []),
      {
        now: NOW,
        repos: [REPO],
        labels: ["user/review"],
      },
    );
    expect(clean.complete).toBe(true);
    expect([clean.sources[0].incomplete, clean.sources[0].truncated]).toEqual([undefined, undefined]);
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

  // The oracle is GitHub's own allowance for a caller with no token:
  // sixty requests an hour from an address. A sweep is one request for
  // each repository and label the census covers, counted here from the
  // lists themselves: a repository added to them is weighed against the
  // allowance by this test, not found out in service. (Any interval
  // that fits passes; ten minutes rather than eight is margin, and is
  // not pinned.)
  test("with no token the sweep is made seldom enough to fit GitHub's hourly allowance for an address", () => {
    const perSweep = REPOS.length * NEEDS_DREW_LABELS.length;
    const anHour = 60 * 60_000;
    expect((anHour / needsDrewSweepTtlMs(false)) * perSweep).toBeLessThan(60);
    // With a token it stays as fresh as it was.
    expect(needsDrewSweepTtlMs(true)).toBe(300_000);
    // (At that pace a caller with no token would not fit: the two differ for a reason.)
    expect((anHour / needsDrewSweepTtlMs(true)) * perSweep).toBeGreaterThan(60);
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
      if (repo !== REPO) return [];
      if (label === "user/review") return [item(1, ["user/review"])];
      return [
        item(2, ["user/escalated"]),
        item(3, ["user/escalated"], { body: '<!-- hands:v1 kind=physical action="Plug in the rack PDU." -->' }),
        item(4, ["user/escalated"], { body: "Escalated: which encoding?" }),
      ];
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
