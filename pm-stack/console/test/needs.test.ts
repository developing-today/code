// What waits on the owner, read from GitHub's labels (src/needs.ts).
//
// The oracle for most of this is the dashboard's own census
// (pm-stack/aggregator/src/needs-drew.ts): given the same issues, the
// list read here must be the list the dashboard makes, since the app
// shows whichever was read. The dashboard's module is loaded by a path
// the type check does not follow (it is another package's, with a
// machine's files in it).
import { describe, expect, test } from "bun:test";
import {
  type Census,
  censusOf,
  githubStatus,
  HANDS_KINDS,
  NEEDS_LABELS,
  NEEDS_PAGES,
  NEEDS_PER_PAGE,
  type NeedsItem,
  needsHands,
  needsItems,
  needsListPath,
  needsReason,
  readCensus,
} from "../src/needs.ts";
import { readConfig } from "../src/config.ts";
import { letters, pick, rng } from "./support.ts";

const DASHBOARD = "../../aggregator/src/needs-drew.ts";
const dashboard = (await import(DASHBOARD)) as {
  NEEDS_DREW_LABELS: readonly string[];
  HANDS_KINDS: readonly string[];
  extractNeedsDrewReason: (body: unknown) => string | null;
  extractNeedsDrewHands: (body: unknown) => { kind: string; action: string | null } | null;
  buildNeedsDrewCensus: (
    fetchPage: (url: string) => Promise<{ ok: boolean; json: unknown }>,
    opts: { repos: readonly string[] },
  ) => Promise<{ items: (Omit<NeedsItem, "kind"> & { kind: string })[]; complete: boolean }>;
};

const DASHBOARD_MODEL = "../../aggregator/src/model.ts";
const dashboardModel = (await import(DASHBOARD_MODEL)) as { REPOS: readonly string[] };

const REPOS = ["developing-today/code", "developing-today-forks/opencode", "dezren39/darwin-nix"];

interface Raw {
  number: number;
  title: string;
  body: string | null;
  state: string;
  html_url: string;
  updated_at: string;
  labels: ({ name: string } | string)[];
  pull_request?: unknown;
}

/** GitHub with these issues in each repository: a page of a
 * repository's list of open issues carrying a label, as the REST API
 * answers it. Every path asked for is noted. */
function github(issues: Record<string, Raw[]>, fails: (path: string) => string | null = () => null) {
  const asked: string[] = [];
  const get = async (path: string): Promise<unknown> => {
    asked.push(path);
    const why = fails(path);
    if (why !== null) throw new Error(why);
    const m = /^repos\/([^/]+\/[^/]+)\/issues\?state=open&labels=([^&]+)&per_page=(\d+)&page=(\d+)$/.exec(path);
    if (!m) throw new Error(`unexpected path ${path}`);
    const [, repo, label, per, page] = m;
    const all = issues[repo];
    if (!all) throw new Error('{"message":"Not Found","status":"404"}');
    const carrying = all.filter((i) =>
      i.labels.some((l) => (typeof l === "string" ? l : l.name) === decodeURIComponent(label)),
    );
    return carrying.slice((Number(page) - 1) * Number(per), Number(page) * Number(per));
  };
  return { asked, get };
}

/** The same GitHub for the dashboard's census, which asks by whole
 * address. It asks for the same lists as the reading here does (it
 * asked the search until #14103, which GitHub refuses in the form it
 * was asked); anything else it might ask is not answered, so that a
 * change in how it asks shows up here. */
function forDashboard(get: (path: string) => Promise<unknown>) {
  return async (url: string) => {
    const u = new URL(url);
    try {
      return { ok: true, json: await get(`${u.pathname.slice(1)}${u.search}`) };
    } catch (error) {
      return { ok: false, json: { error: (error as Error).message } };
    }
  };
}

const issue = (number: number, labels: string[], more: Partial<Raw> = {}): Raw => ({
  number,
  title: `Item ${number}`,
  body: null,
  state: "open",
  html_url: `https://github.com/x/y/issues/${number}`,
  updated_at: "2026-11-23T10:00:00Z",
  labels: labels.map((name) => ({ name })),
  ...more,
});

/** Bodies as issues have them: prose, with now and then a reason line,
 * a hands mark, or something that only looks like one. */
function bodies(seed: number, n: number): (string | null)[] {
  const next = rng(seed);
  const word = () => letters(next, "abcdefghij klmno", 1, 12);
  const kinds = [...HANDS_KINDS, "remote", "PHYSICAL", "physical2", ""];
  const out: (string | null)[] = [null, "", "Reason:", "reason - \u{1f600}".padEnd(400, "\u{1f600}")];
  for (let i = 0; i < n; i++) {
    const lines: string[] = [];
    const count = Math.floor(next() * 5);
    for (let l = 0; l < count; l++) {
      const which = next();
      if (which < 0.2)
        lines.push(
          `${pick(next, ["Escalated", "escalated to Drew", "Reason", "REASON", "Reasons"])}${pick(next, [":", " -", " —", "", " :"])} ${word()}`,
        );
      else if (which < 0.45) {
        const action = pick(next, [
          "",
          ` action="${word()}"`,
          ' action=""',
          ` action="${"x".repeat(300)}"`,
          ` action='${word()}'`,
        ]);
        lines.push(`<!-- hands:${pick(next, ["v1", "v2"])} kind=${pick(next, kinds)}${action} -->`);
      } else lines.push(word());
    }
    out.push(lines.join(pick(next, ["\n", "\n\n", " "])));
  }
  return out;
}

describe("where it is read from", () => {
  // The two lists were written apart and came to differ: the console
  // named two forks and a repository by names they no longer have.
  // GitHub answered those by redirecting, so nothing failed, but an
  // item was named one way by the dashboard and another by the console.
  test("the repositories the console looks in, unless told otherwise, are the ones the dashboard censuses", () => {
    expect(readConfig({ HOME: "/h" }).needsRepos).toEqual([...dashboardModel.REPOS]);
    // Told otherwise, it looks where it is told.
    expect(readConfig({ HOME: "/h", PM_CONSOLE_NEEDS_REPOS: "a/one, a/two" }).needsRepos).toEqual(["a/one", "a/two"]);
  });
});

describe("how a body is read", () => {
  test("the labels and the kinds of hands are the dashboard's", () => {
    expect([...NEEDS_LABELS] as string[]).toEqual([...dashboard.NEEDS_DREW_LABELS]);
    expect([...HANDS_KINDS] as string[]).toEqual([...dashboard.HANDS_KINDS]);
  });

  test("a reason and a hands mark are read as the dashboard reads them, for any body", () => {
    // Negative control first: the bodies do exercise both readers.
    const all = bodies(20261124, 600);
    expect(all.filter((b) => needsReason(b) !== null).length).toBeGreaterThan(40);
    expect(all.filter((b) => needsHands(b) !== null).length).toBeGreaterThan(40);
    expect(all.filter((b) => needsHands(b) === null && String(b).includes("hands:")).length).toBeGreaterThan(40);
    for (const body of all) {
      expect([body, needsReason(body)]).toEqual([body, dashboard.extractNeedsDrewReason(body)]);
      expect([body, needsHands(body) as unknown]).toEqual([body, dashboard.extractNeedsDrewHands(body)]);
    }
    // What is not text is not read.
    for (const body of [undefined, 3, {}, []]) expect([needsReason(body), needsHands(body)]).toEqual([null, null]);
  });

  test("cut at 240, and never through the middle of a character", () => {
    const long = `Reason: ${"a".repeat(239)}\u{1f600}tail`;
    const reason = needsReason(long) ?? "";
    expect([reason.length, reason.endsWith("a")]).toEqual([239, true]);
    expect(needsHands(`<!-- hands:v1 kind=payment action="${"b".repeat(300)}" -->`)?.action).toHaveLength(240);
  });
});

describe("the three lists", () => {
  const item = (ref: string, more: Partial<NeedsItem> = {}): NeedsItem => ({
    ref,
    kind: "issue",
    title: `Title of ${ref}`,
    url: `https://github.com/${ref.replace("#", "/issues/")}`,
    needsLabels: ["user/escalated"],
    reason: null,
    updatedAt: "2026-11-23T10:00:00Z",
    hands: null,
    ...more,
  });

  test("a hands mark wins, then a review that was asked for; everything else is listed as the fleet's to decide", () => {
    const census = censusOf(
      [
        item("a/b#1", { hands: { kind: "payment", action: "Pay the invoice" }, needsLabels: ["user/review"] }),
        item("a/b#2", { needsLabels: ["user/review"], kind: "pr" }),
        item("a/b#3", { needsLabels: ["user/review", "user/escalated"] }),
        item("a/b#4", { reason: "an agent could not choose" }),
        item("a/b#5", { hands: { kind: "signature", action: null } }),
      ],
      "2026-11-23T11:00:00Z",
      true,
    );
    expect(census.hands.map((i) => [i.ref, i.kind, i.action])).toEqual([
      ["a/b#1", "payment", "Pay the invoice"],
      ["a/b#5", "signature", ""],
    ]);
    expect(census.reviews.map((i) => [i.ref, i.pr])).toEqual([
      ["a/b#2", true],
      ["a/b#3", false],
    ]);
    expect(census.other.map((i) => [i.ref, i.why])).toEqual([["a/b#4", "an agent could not choose"]]);
    // Every item is in one list and one only.
    expect(census.hands.length + census.reviews.length + census.other.length).toBe(5);
    expect([census.at, census.complete, "unread" in census]).toEqual(["2026-11-23T11:00:00Z", true, false]);
  });

  test("what is shown of each is kept short, and the time to the second", () => {
    const census = censusOf(
      [
        item("a/b#1", { title: "t".repeat(500), updatedAt: "2026-11-23T10:00:00.123Z", reason: "r".repeat(500) }),
        item("a/b#2", { hands: { kind: "physical", action: "x".repeat(900) } }),
      ],
      null,
      false,
      ["a/c"],
    );
    expect([census.other[0].title.length, census.other[0].why.length, census.other[0].updated]).toEqual([
      200,
      300,
      "2026-11-23T10:00:00",
    ]);
    expect(census.hands[0].action).toHaveLength(400);
    expect([census.at, census.complete, census.unread]).toEqual([null, false, ["a/c"]]);
  });
});

describe("read from GitHub", () => {
  /** The census, less what only one of the two readers says. */
  const lists = (c: Census) => ({ hands: c.hands, reviews: c.reviews, other: c.other });

  test("the list read here is the list the dashboard makes of the same issues", async () => {
    const next = rng(77);
    const issues: Record<string, Raw[]> = {};
    const texts = bodies(5, 200);
    for (const repo of REPOS) {
      issues[repo] = [];
      const n = 20 + Math.floor(next() * 150);
      for (let i = 0; i < n; i++) {
        const labels = [pick(next, ["bug", "kind/feature", "next"])];
        const which = next();
        if (which < 0.4) labels.push("user/review");
        else if (which < 0.8) labels.push("user/escalated");
        else if (which < 0.9) labels.push("user/escalated", "user/review");
        issues[repo].push(
          issue(1000 - i * 3, labels, {
            title: letters(next, "abc def", 1, 260),
            body: pick(next, texts),
            ...(next() < 0.3 ? { pull_request: { url: "x" } } : {}),
            updated_at: `2026-11-${String(10 + Math.floor(next() * 13)).padStart(2, "0")}T10:00:00Z`,
          }),
        );
      }
    }
    const here = await readCensus(REPOS, github(issues).get);
    const asked = github(issues);
    const theirs = await dashboard.buildNeedsDrewCensus(forDashboard(asked.get), { repos: REPOS });
    // Negative control: there is something of each kind, and more than a page of something.
    expect([here.hands.length > 5, here.reviews.length > 20, here.other.length > 20]).toEqual([true, true, true]);
    expect(Math.max(...REPOS.map((repo) => issues[repo].length))).toBeGreaterThan(NEEDS_PER_PAGE);
    expect([theirs.complete, here.complete, here.unread]).toEqual([true, true, []]);
    // Both asked GitHub for the same lists, page for page.
    const mine = github(issues);
    await readCensus(REPOS, mine.get);
    expect(asked.asked).toEqual(mine.asked);
    // The console makes its three lists of the dashboard's items with the same function.
    const made = censusOf(
      theirs.items.map((i) => ({ ...i, kind: i.kind === "pr" ? ("pr" as const) : ("issue" as const) })),
      null,
      true,
    );
    expect(lists(here)).toEqual(lists(made));
  });

  test("in the order the repositories are named and by number within one, each item once whichever labels it carries", async () => {
    const { get, asked } = github({
      "a/one": [issue(9, ["user/escalated"]), issue(3, ["user/review", "user/escalated"]), issue(5, ["user/review"])],
      "a/two": [issue(1, ["user/escalated"], { pull_request: {} })],
    });
    const census = await readCensus(["a/two", "a/one"], get);
    expect(census.other.map((i) => i.ref)).toEqual(["a/two#1", "a/one#9"]);
    expect(census.reviews.map((i) => i.ref)).toEqual(["a/one#3", "a/one#5"]);
    expect(census.other[0].pr).toBe(true);
    // Each repository's list of each label, one page: nothing was a full page.
    expect(asked).toEqual(
      ["a/two", "a/one"].flatMap((repo) => NEEDS_LABELS.map((label) => needsListPath(repo, label, 1))),
    );
    // Nothing in it changes from one read to the next when GitHub has not.
    expect(await readCensus(["a/two", "a/one"], get)).toEqual(census);
    expect(census.at).toBeNull();
  });

  test("a list longer than a page is read to its end, and one longer than is read at all is said to be cut short", async () => {
    const many = (n: number) => Array.from({ length: n }, (_, i) => issue(i + 1, ["user/review"]));
    const exact = github({ "a/one": many(2 * NEEDS_PER_PAGE) });
    const whole = await readCensus(["a/one"], exact.get);
    expect([whole.reviews.length, whole.complete]).toEqual([2 * NEEDS_PER_PAGE, true]);
    // A last page that is full is followed by one more, which is empty.
    expect(
      exact.asked.filter((path) => path.includes("user%2Freview")).map((path) => path.split("page=").pop()),
    ).toEqual(["1", "2", "3"]);
    const over = github({ "a/one": many(NEEDS_PAGES * NEEDS_PER_PAGE + 1) });
    const cut = await readCensus(["a/one"], over.get);
    expect([cut.reviews.length, cut.complete, cut.unread]).toEqual([
      NEEDS_PAGES * NEEDS_PER_PAGE,
      false,
      [`a/one (user/review: only the first ${NEEDS_PAGES * NEEDS_PER_PAGE})`],
    ]);
    expect(over.asked.filter((path) => path.includes("user%2Freview"))).toHaveLength(NEEDS_PAGES);
  });

  test("only what is open and has a number is taken from a page", () => {
    const page = [
      issue(1, ["user/review"]),
      issue(2, ["user/review"], { state: "closed" }),
      { ...issue(3, ["user/review"]), number: "3" },
      null,
      "text",
      issue(4, ["user/review", "other"], { labels: ["user/review", { name: "other" }] }),
    ];
    expect(needsItems("a/b", page).map((i) => [i.ref, i.needsLabels])).toEqual([
      ["a/b#1", ["user/review"]],
      ["a/b#4", ["user/review"]],
    ]);
    expect(needsItems("a/b", { items: [] })).toEqual([]);
  });

  test("a repository this caller cannot read is named, and the others are read all the same", async () => {
    // Not there for this caller; not this caller's to read; no issues kept. Each in the words
    // one of the two ways of asking gives.
    for (const said of [
      '{"message":"Not Found","documentation_url":"https://docs.github.com/rest","status":"404"}',
      "gh: Not Found (HTTP 404)",
      '{"message":"Not Found"}',
      '{"message":"Resource not accessible by personal access token","status":"403"}',
      "gh: Issues are disabled for this repo (HTTP 410)",
      "GitHub answered http 404",
    ]) {
      const { get, asked } = github(
        { "a/one": [issue(1, ["user/review"])], "a/three": [issue(2, ["user/escalated"])] },
        (path) => (path.startsWith("repos/a/two/") ? said : null),
      );
      const census = await readCensus(["a/one", "a/two", "a/three"], get);
      expect([said, census.complete, census.unread]).toEqual([said, false, ["a/two"]]);
      expect([census.reviews.map((i) => i.ref), census.other.map((i) => i.ref)]).toEqual([["a/one#1"], ["a/three#2"]]);
      expect(asked).toHaveLength(6);
    }
    // One of a repository's two lists: said with the label.
    const { get } = github({ "a/one": [issue(1, ["user/review"])] }, (path) =>
      path.includes("user%2Fescalated") ? "gh: Not Found (HTTP 404)" : null,
    );
    expect((await readCensus(["a/one"], get)).unread).toEqual(["a/one (user/escalated)"]);
  });

  test("no repository that could be read is a failed read, not an empty list", async () => {
    const { get } = github({});
    await expect(readCensus(["a/one", "a/two"], get)).rejects.toThrow(/Not Found/);
    // With no repository named there is nothing to fail: the list is empty, and whole.
    expect(await readCensus([], get)).toEqual({
      at: null,
      complete: true,
      unread: [],
      hands: [],
      reviews: [],
      other: [],
    });
  });

  test("turned away for calling too often, out of reach, or GitHub in trouble: the read ends there and nothing more is asked", async () => {
    for (const said of [
      '{"message":"API rate limit exceeded for user ID 1.","status":"403"}',
      '{"message":"You have exceeded a secondary rate limit.","status":"429"}',
      "GitHub says to ask again at 2026-11-24T06:00:00.000Z: not asked again since GitHub answered: API rate limit exceeded",
      "GitHub could not be reached: The operation timed out.",
      '{"message":"Server Error","status":"500"}',
      "gh: Bad credentials (HTTP 401)",
    ]) {
      // The third list asked for is the one that fails.
      let n = 0;
      const { get, asked } = github({ "a/one": [issue(1, ["user/review"])], "a/two": [], "a/three": [] }, () =>
        ++n === 3 ? said : null,
      );
      await expect(readCensus(["a/one", "a/two", "a/three"], get)).rejects.toThrow(said.slice(0, 40));
      expect([said, asked.length]).toEqual([said, 3]);
    }
  });

  test("the status a failure names, in each of the ways it is worded", () => {
    expect(githubStatus("fleet gh failed (exit 1): gh: Not Found (HTTP 404)")).toBe(404);
    expect(githubStatus('fleet gh failed (exit 1): {"message":"x","status":"422"}')).toBe(422);
    expect(githubStatus('{"message":"x","status": 403}')).toBe(403);
    expect(githubStatus("GitHub answered http 502")).toBe(502);
    expect(githubStatus('{"message":"Not Found"}')).toBe(404);
    expect(githubStatus("GitHub could not be reached: socket hang up")).toBeNull();
    expect(githubStatus("")).toBeNull();
  });
});
