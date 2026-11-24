// Trends: what was opened, merged, closed and finished, by day.
//
// Oracles:
//   - what a title says a pull request is: real titles merged on
//     2026-11-24, with what each is written out by hand;
//   - GitHub's "list repository issues" is stood in for by a list that
//     answers as GitHub does: everything changed at or after `since`
//     (checked on GitHub itself on 2026-11-24: an item is in the answer
//     to its own time), oldest change first, a hundred to a page, a pull
//     request carrying `pull_request.merged_at`;
//   - the queue's store is a real SQLite with the store's tables and
//     indexes, answering over the same wire as the store, and the query
//     plan is asked of it;
//   - every expected count is worked out by hand from the items a test
//     puts in.
import { describe, expect, test } from "bun:test";
import { Database } from "bun:sqlite";
import { readConfig } from "../src/config.ts";
import { type Fetch, Ops, type Run } from "../src/ops.ts";
import {
  daysBack,
  type Day,
  fold,
  HISTORY_DAYS,
  median,
  memoryShelf,
  projectOf,
  TASKS_BY_DAY_SQL,
  titleParts,
  Trends,
} from "../src/trends.ts";
import { asThePlatformFetch } from "./support.ts";

const CFG = readConfig({ HOME: "/h" });
const NOW = new Date("2026-11-24T12:00:00Z");

interface Item {
  number: number;
  title: string;
  created_at: string;
  updated_at: string;
  closed_at: string | null;
  state: "open" | "closed";
  labels: { name: string }[];
  pull_request?: { merged_at: string | null };
}

const issue = (number: number, created: string, more: Partial<Item> = {}): Item => ({
  number,
  title: `Issue ${number}`,
  created_at: created,
  updated_at: more.closed_at ?? created,
  closed_at: null,
  state: more.closed_at ? "closed" : "open",
  labels: [],
  ...more,
});
const pull = (
  number: number,
  title: string,
  created: string,
  merged: string | null,
  more: Partial<Item> = {},
): Item => ({
  number,
  title,
  created_at: created,
  updated_at: merged ?? more.closed_at ?? created,
  closed_at: merged ?? null,
  state: merged || more.closed_at ? "closed" : "open",
  labels: [],
  pull_request: { merged_at: merged },
  ...more,
});

/** A value as the queue's store sends it over HTTP. */
const asTheStore = (value: unknown) =>
  value === null || value === undefined
    ? { type: "null" }
    : typeof value === "number" && Number.isInteger(value)
      ? { type: "integer", value: String(value) }
      : { type: "text", value: String(value) };

function queueDb() {
  const db = new Database(":memory:");
  db.run(
    "CREATE TABLE tasks (id INTEGER PRIMARY KEY, queue TEXT NOT NULL, title TEXT, payload TEXT NOT NULL DEFAULT '{}', priority INTEGER, status TEXT NOT NULL DEFAULT 'queued', created_at TEXT)",
  );
  db.run(
    "CREATE TABLE events (id INTEGER PRIMARY KEY AUTOINCREMENT, worker_id TEXT, task_id INTEGER, kind TEXT NOT NULL, detail TEXT NOT NULL DEFAULT '{}', created_at TEXT NOT NULL)",
  );
  db.run("CREATE INDEX events_task ON events(task_id, id)");
  db.run("CREATE INDEX events_kind_ts ON events(kind, created_at)");
  const task = (id: number, kind: string | null, status = "done", queue = CFG.queue) =>
    db.run("INSERT INTO tasks (id, queue, payload, status) VALUES (?, ?, ?, ?)", [
      id,
      queue,
      kind === null ? "not json" : JSON.stringify({ kind }),
      status,
    ]);
  const event = (taskId: number, kind: string, at: string) =>
    db.run("INSERT INTO events (task_id, kind, created_at) VALUES (?, ?, ?)", [taskId, kind, at]);
  return { db, task, event };
}

/** GitHub and the queue store, as the classes reach them. */
function world(items: Item[], db?: Database) {
  const asked: string[] = [];
  const state = { fail: null as null | ((path: string, n: number) => string | null), queueDown: false };
  const run: Run = async (argv) => {
    if (argv[1] === "secrets") return { code: 0, stdout: "token\n", stderr: "" };
    if (argv[1] !== "gh" || argv[2] !== "api") return { code: 1, stdout: "", stderr: "not a command the test knows" };
    const path = argv[3];
    asked.push(path);
    const why = state.fail?.(path, asked.length);
    if (why) return { code: 1, stdout: "", stderr: why };
    const url = new URL(`https://api.github.com/${path}`);
    if (url.pathname !== `/repos/${CFG.repo}/issues`) return { code: 1, stdout: "", stderr: "Not Found" };
    const q = url.searchParams;
    // What the reader must ask for, or the answer would be another list.
    expect([q.get("state"), q.get("sort"), q.get("direction")]).toEqual(["all", "updated", "asc"]);
    const since = q.get("since") ?? "";
    const per = Math.min(100, Number(q.get("per_page") ?? 30));
    const at = Number(q.get("page") ?? 1);
    const page = items
      .filter((i) => i.updated_at >= since)
      .sort((a, b) => (a.updated_at < b.updated_at ? -1 : a.updated_at > b.updated_at ? 1 : a.number - b.number))
      .slice((at - 1) * per, at * per);
    return { code: 0, stdout: JSON.stringify(page), stderr: "" };
  };
  const fetchImpl: Fetch = async (_url, init) => {
    if (state.queueDown || !db) return { ok: false, status: 503, json: async () => ({}) };
    const requests = (
      JSON.parse(init.body ?? "{}").requests as {
        type: string;
        stmt?: { sql: string; args: { value: string; type: string }[] };
      }[]
    ).filter((r) => r.type === "execute");
    return {
      ok: true,
      status: 200,
      json: async () => ({
        results: requests.map((r) => {
          const args = (r.stmt?.args ?? []).map((a) => (a.type === "integer" ? Number(a.value) : a.value));
          const out = db.query(r.stmt?.sql ?? "").all(...args) as Record<string, unknown>[];
          const cols = Object.keys(out[0] ?? {}).map((name) => ({ name }));
          return {
            type: "ok",
            response: { result: { cols, rows: out.map((row) => cols.map((c) => asTheStore(row[c.name]))) } },
          };
        }),
      }),
    };
  };
  const ops = new Ops(CFG, run, asThePlatformFetch(fetchImpl) as Fetch);
  const shelf = memoryShelf();
  const clock = { now: NOW };
  const trends = new Trends(ops, shelf, () => clock.now);
  const day = async (date: string) => JSON.parse((await shelf.get(`day:${date}`)) ?? "null") as Day | null;
  const meta = async () =>
    JSON.parse((await shelf.get("meta")) ?? "null") as {
      from: string;
      cursor: string;
      page: number;
      liveFrom: string;
      live: boolean;
      back: { from: string; cursor: string; page: number; until: string } | null;
      edges: string[];
      whole: boolean;
      readAt: string | null;
    };
  return { ops, shelf, trends, asked, state, clock, day, meta, items };
}

describe("what a pull request's title says it is", () => {
  test("real titles merged on 2026-11-24: its kind, its scope, and the project that scope belongs to", () => {
    const said = (title: string) => {
      const p = titleParts(title);
      return [p.kind, p.scope, projectOf(p.scope)];
    };
    expect(
      said("feat(pm-stack): the app's look is the owner's to choose: themes, typefaces, size and spacing"),
    ).toEqual(["feat", "pm-stack", "pm-stack"]);
    expect(said("fix(laptop-agent): report a failed final meta write (#12195)")).toEqual([
      "fix",
      "laptop-agent",
      "fleet",
    ]);
    expect(said("feat(id-import-notion): row notes, property claims, relations (#13523)")).toEqual([
      "feat",
      "id-import-notion",
      "id",
    ]);
    expect(said("feat(aggregator): C4.3 day-type + spike-day forecast adjustments as shadow")).toEqual([
      "feat",
      "aggregator",
      "pm-stack",
    ]);
    // A scope and no kind: counted under its project, and not as a feature.
    expect(said("id-pg: Postgres-over-HTTP (PostgREST) query translator with allowlist")).toEqual([
      "other",
      "id-pg",
      "id",
    ]);
    expect(said("fleet-secrets: bound every fallback subprocess wait (#14040 Residual B)")).toEqual([
      "other",
      "fleet-secrets",
      "fleet",
    ]);
    // Two kinds joined is the first; a scope nobody claims is everything else.
    expect(said("test+docs(bedside): BA-W07 second pass — external/gateway suite")).toEqual([
      "test",
      "bedside",
      "other",
    ]);
    // Only words.
    expect(said("Add scheduled five-copy freshness probe + Cloudflare rehost evaluation for #8584")).toEqual([
      "other",
      "",
      "other",
    ]);
    expect(said("S1 of #14056: feature catalogue + manifest + hash as design-of-record")).toEqual([
      "other",
      "",
      "other",
    ]);
    expect(said("🎨 Palette: label lab-ops-hub inputs, announce async output, visible focus")).toEqual([
      "other",
      "",
      "other",
    ]);
    expect(said("Record the approved chat map as FDR-RA-001 (#9569)")).toEqual(["other", "", "other"]);
  });

  test("the forms a title takes", () => {
    expect(titleParts("feat!: drop the old way")).toEqual({ kind: "feat", scope: "", words: "drop the old way" });
    expect(titleParts("Feat(ID): Caps")).toEqual({ kind: "feat", scope: "id", words: "Caps" });
    expect(titleParts("feature(atlas-search): ranked results")).toEqual({
      kind: "feat",
      scope: "atlas-search",
      words: "ranked results",
    });
    expect(titleParts("docs(fleet): how a lane is claimed").kind).toBe("docs");
    expect(titleParts("chore(deps): bump a thing").kind).toBe("other");
    expect(titleParts("refactor(mcpx): one reader")).toEqual({ kind: "other", scope: "mcpx", words: "one reader" });
    // A single word before the colon that is not a kind is a scope.
    expect(titleParts("mcpx: one reader")).toEqual({ kind: "other", scope: "mcpx", words: "one reader" });
    expect(titleParts("Refine(queue): the scan")).toEqual({
      kind: "other",
      scope: "",
      words: "Refine(queue): the scan",
    });
    // A word before a colon that is not a kind and has a space is no scope.
    expect(titleParts("Decision record: use the queue")).toEqual({
      kind: "other",
      scope: "",
      words: "Decision record: use the queue",
    });
    // A colon with nothing after it, or no space, is not the form.
    expect(titleParts("feat(x):nospace").scope).toBe("");
    expect(titleParts("")).toEqual({ kind: "other", scope: "", words: "" });
    expect(titleParts(undefined as never)).toEqual({ kind: "other", scope: "", words: "" });
    // Projects by scope: the name itself, and what is under it.
    for (const [scope, project] of [
      ["id", "id"],
      ["id/web", "id"],
      ["id-store", "id"],
      ["idle", "other"],
      ["fleet", "fleet"],
      ["fleet-queue", "fleet"],
      ["queue", "fleet"],
      ["claimant", "fleet"],
      ["opencode", "fleet"],
      ["atlas", "atlas"],
      ["atlas-core", "atlas"],
      ["atlases", "other"],
      ["pm", "pm-stack"],
      ["pm-console", "pm-stack"],
      ["pm-service", "pm-stack"],
      ["android", "pm-stack"],
      ["pmx", "other"],
      ["mcpx", "mcpx"],
      ["ci", "other"],
      ["", "other"],
      ["  PM-Stack ", "pm-stack"],
    ]) {
      expect([scope, projectOf(scope)]).toEqual([scope, project]);
    }
  });
});

describe("an issue or pull request put into the days it belongs to", () => {
  const FROM = "2026-10-01";
  const into = (items: Item[]) => {
    const days = new Map<string, Day>();
    const day = (date: string) => {
      if (!days.has(date)) days.set(date, { io: [], ic: [], bo: [], bc: [], po: [], px: [], pm: {} });
      return days.get(date) as Day;
    };
    for (const item of items) fold(item, FROM, day);
    return Object.fromEntries([...days.entries()].sort());
  };

  test("each thing is counted on the day it happened", () => {
    const days = into([
      issue(1, "2026-11-01T10:00:00Z"),
      issue(2, "2026-11-01T11:00:00Z", { closed_at: "2026-11-03T09:00:00Z", labels: [{ name: "kind/bug" }] }),
      pull(3, "feat(id): a thing", "2026-11-01T23:50:00Z", "2026-11-02T00:20:00Z"),
      pull(4, "fix(fleet): another", "2026-11-02T08:00:00Z", null, { closed_at: "2026-11-02T09:00:00Z" }),
      pull(5, "docs: words", "2026-11-03T08:00:00Z", null),
    ]);
    expect(days).toEqual({
      "2026-11-01": { io: [1, 2], ic: [], bo: [2], bc: [], po: [3], px: [], pm: {} },
      "2026-11-02": {
        io: [],
        ic: [],
        bo: [],
        bc: [],
        po: [4],
        px: [4],
        pm: { "3": ["feat", "id", 30, "00:20", "a thing"] },
      },
      "2026-11-03": { io: [], ic: [2], bo: [], bc: [2], po: [5], px: [], pm: {} },
    });
  });

  test("read again it changes nothing; read after it changed it adds what is new", () => {
    const open = pull(7, "fix(pm-console): a fix", "2026-11-05T10:00:00Z", null);
    const merged = pull(7, "fix(pm-console): a fix", "2026-11-05T10:00:00Z", "2026-11-06T10:00:00Z");
    expect(into([open, open, merged, merged])).toEqual({
      "2026-11-05": { io: [], ic: [], bo: [], bc: [], po: [7], px: [], pm: {} },
      // A day and no minutes more: 1440. Its words are not kept: only a feature's are listed.
      "2026-11-06": { io: [], ic: [], bo: [], bc: [], po: [], px: [], pm: { "7": ["fix", "pm-stack", 1440, "10:00"] } },
    });
  });

  test("what happened before the history starts is not counted, and what is not an issue is passed over", () => {
    const days = into([
      // Opened before, closed within: its closing counts and its opening does not.
      issue(8, "2026-09-20T10:00:00Z", { closed_at: "2026-10-02T10:00:00Z" }),
      // Merged before: nothing.
      pull(9, "feat(id): old", "2026-09-01T10:00:00Z", "2026-09-02T10:00:00Z"),
      { number: 0 } as Item,
      { number: Number.NaN } as Item,
      // An issue said to be open with a closing time left on it (it was opened again) is not closed.
      issue(10, "2026-10-05T10:00:00Z", { closed_at: "2026-10-06T10:00:00Z", state: "open" }),
    ]);
    expect(days).toEqual({
      "2026-10-02": { io: [], ic: [8], bo: [], bc: [], po: [], px: [], pm: {} },
      "2026-10-05": { io: [10], ic: [], bo: [], bc: [], po: [], px: [], pm: {} },
    });
  });

  test("the middle of some numbers, and the days of a window", () => {
    expect(median([])).toBeNull();
    expect(median([5])).toBe(5);
    expect(median([9, 1, 5])).toBe(5);
    expect(median([4, 1, 3, 2])).toBe(2.5);
    expect(daysBack("2026-11-24", 3)).toEqual(["2026-11-22", "2026-11-23", "2026-11-24"]);
    // Across a month's end and a leap day.
    expect(daysBack("2028-03-01", 3)).toEqual(["2028-02-28", "2028-02-29", "2028-03-01"]);
  });
});

describe("the history is read from GitHub newest first, a page at a time, and kept", () => {
  /** 250 pull requests, one merged every ten minutes from the 20th: the last at 17:30 on the 21st. */
  const many = () =>
    Array.from({ length: 250 }, (_, i) => {
      const at = new Date(Date.parse("2026-11-20T00:00:00Z") + i * 600_000).toISOString().replace(".000Z", "Z");
      return pull(1000 + i, `feat(id): thing ${i}`, at, at);
    });
  const since = (asked: string[]) =>
    asked.map((path) => /since=([^&]+)(?:&per_page=100)(&page=\d+)?/.exec(path)?.slice(1).join("") ?? path);
  /** Every merged pull request kept, whatever day it is under. */
  const kept = async (w: ReturnType<typeof world>) => {
    const numbers = new Set<string>();
    for (const key of w.shelf.keys()) {
      if (key.startsWith("day:")) for (const n of Object.keys(((await w.day(key.slice(4))) as Day).pm)) numbers.add(n);
    }
    return numbers;
  };

  test("the last day first, to the end of the list; then each older stretch up to where the one after it began", async () => {
    // It is noon on the 24th. The stretches start 1, 3, 7, 14, 30 and 60 days back, and with the history itself.
    const w = world(many());
    const first = await w.trends.advance(2);
    expect(HISTORY_DAYS).toBe(90);
    // Asked for exactly this: everything, by when it changed, oldest change first, from a day back.
    expect(w.asked[0]).toBe(
      `repos/${CFG.repo}/issues?state=all&sort=updated&direction=asc&since=2026-11-23T12:00:00Z&per_page=100`,
    );
    // Nothing changed in the last day, so that stretch is read at once; the next goes back to noon on the 21st.
    expect(since(w.asked)).toEqual(["2026-11-23T12:00:00Z", "2026-11-21T12:00:00Z"]);
    // From noon on the 21st there are 34 (12:00 to 17:30, one every ten minutes): fewer than a page, so that stretch is read too.
    expect((await kept(w)).size).toBe(34);
    expect([first.live, first.whole]).toEqual([true, false]);
    expect(await w.meta()).toEqual({
      from: "2026-08-26",
      cursor: "2026-11-23T12:00:00Z",
      page: 1,
      liveFrom: "2026-11-23T12:00:00Z",
      live: true,
      // The stretch being read now: a week back, up to where the last one began.
      back: { from: "2026-11-17T12:00:00Z", cursor: "2026-11-17T12:00:00Z", page: 1, until: "2026-11-21T12:00:00Z" },
      edges: ["2026-11-10T12:00:00Z", "2026-10-25T12:00:00Z", "2026-09-25T12:00:00Z", "2026-08-26T00:00:00Z"],
      whole: false,
      readAt: NOW.toISOString(),
    });
    expect(await w.trends.due()).toBe(true);
    // Read on: one look at what is new (nothing), then the stretch from a week back takes three pages
    // (100, 100, and the rest up to where the next stretch began), and the four older stretches a page each,
    // each ending at once: its first page already reaches past where it stops.
    const written: string[] = [];
    const put = w.shelf.put;
    w.shelf.put = async (key, text) => {
      if (key.startsWith("day:")) written.push(key);
      return put(key, text);
    };
    const second = await w.trends.advance(20);
    // A stretch counts only what changed before the one after it began: the rest was counted by that
    // one. So the week-back stretch writes the 20th, then the 20th and 21st, then the 21st, and the
    // four older stretches (whose first page is all from after they stop) write no day at all.
    expect(written).toEqual(["day:2026-11-20", "day:2026-11-20", "day:2026-11-21", "day:2026-11-21"]);
    expect(since(w.asked).slice(2)).toEqual([
      "2026-11-23T12:00:00Z",
      "2026-11-17T12:00:00Z",
      "2026-11-20T16:30:00Z",
      "2026-11-21T09:00:00Z",
      "2026-11-10T12:00:00Z",
      "2026-10-25T12:00:00Z",
      "2026-09-25T12:00:00Z",
      "2026-08-26T00:00:00Z",
    ]);
    expect([second.whole, second.back, second.edges]).toEqual([true, null, []]);
    expect((await kept(w)).size).toBe(250);
    // The 20th has the first 144 (one every ten minutes for a day), the 21st the rest.
    expect(Object.keys(((await w.day("2026-11-20")) as Day).pm).length).toBe(144);
    expect(Object.keys(((await w.day("2026-11-21")) as Day).pm).length).toBe(106);
    expect(await w.trends.due()).toBe(false);
  });

  test("kept up afterwards by asking what changed since: one page, and only what is new is added", async () => {
    const w = world(many());
    await w.trends.advance(20);
    expect((await w.meta()).whole).toBe(true);
    const before = w.asked.length;
    // Nothing changed: one question, the same days.
    const was = JSON.stringify(await w.day("2026-11-21"));
    await w.trends.advance(20);
    expect(w.asked.length).toBe(before + 1);
    expect(JSON.stringify(await w.day("2026-11-21"))).toBe(was);
    // A pull request opened long ago merges now, and an issue is filed.
    w.items.push(pull(77, "feat(fleet): late", "2026-11-10T00:00:00Z", "2026-11-24T11:00:00Z"));
    w.items.push(issue(78, "2026-11-24T11:30:00Z", { labels: [{ name: "kind/bug" }] }));
    await w.trends.advance(20);
    expect(w.asked.length).toBe(before + 2);
    expect(since(w.asked.slice(-1))).toEqual(["2026-11-23T12:00:00Z"]);
    expect(await w.day("2026-11-24")).toEqual({
      io: [78],
      ic: [],
      bo: [78],
      bc: [],
      po: [],
      px: [],
      // Opened at midnight on the 10th, merged at 11:00 on the 24th: 14 days and 11 hours, 20,820 minutes.
      pm: { "77": ["feat", "fleet", 20820, "11:00", "late"] },
    });
    // The next look starts from the last change seen.
    await w.trends.advance(20);
    expect(since(w.asked.slice(-1))).toEqual(["2026-11-24T11:30:00Z"]);
    // It is due again only after a while.
    expect(await w.trends.due()).toBe(false);
    w.clock.now = new Date(NOW.getTime() + 5 * 60_000);
    expect(await w.trends.due()).toBe(true);
    expect(await w.trends.due(30 * 60_000)).toBe(false);
    expect(await w.trends.dueAt(30 * 60_000)).toBe(NOW.getTime() + 30 * 60_000);
  });

  test("a busy last day takes several pages, and nothing older is read until it has been read to its end", async () => {
    // 230 changed in the last day, one a minute from 13:00 on the 23rd.
    const busy = Array.from({ length: 230 }, (_, i) => {
      const at = new Date(Date.parse("2026-11-23T13:00:00Z") + i * 60_000).toISOString().replace(".000Z", "Z");
      return pull(5000 + i, "fix(id): busy", at, at);
    });
    const w = world([...many(), ...busy]);
    const first = await w.trends.advance(2);
    // Two pages of the last day, each from the last change of the one before: not at its end yet.
    expect(since(w.asked)).toEqual(["2026-11-23T12:00:00Z", "2026-11-23T14:39:00Z"]);
    expect([first.live, first.back, first.readAt]).toEqual([false, null, null]);
    // Until then the screen cannot say any day is whole (GitHub is out of reach for this one look,
    // so the screen shows what is kept and reads nothing on).
    w.state.fail = () => "GitHub could not be reached: timed out";
    const early = await w.trends.read("7");
    expect([early.history.covered, early.history.whole, early.partial]).toEqual([null, false, true]);
    w.state.fail = null;
    const done = await w.trends.advance(1);
    expect(since(w.asked).slice(-1)).toEqual(["2026-11-23T16:18:00Z"]);
    expect(done.live).toBe(true);
    // Now the last day is whole, and the screen says how far back that goes.
    expect((await w.meta()).back?.until).toBe("2026-11-23T12:00:00Z");
    expect((await kept(w)).size).toBe(230);
  });

  test("something that changes while the history is being read is still counted once", async () => {
    const items = many();
    const w = world(items);
    // The last day (nothing), the stretch from noon on the 21st (34), and the first page of the one before it.
    await w.trends.advance(3);
    expect((await kept(w)).size).toBe(134);
    // One already read is changed now (a comment on it): it moves to the end of the list.
    items[10].updated_at = "2026-11-24T12:00:05Z";
    // One not yet read is changed too: it leaves the stretch it was in.
    items[150].updated_at = "2026-11-24T12:00:06Z";
    await w.trends.advance(20);
    expect((await kept(w)).size).toBe(250);
    const meta = await w.meta();
    expect([meta.whole, meta.cursor]).toEqual([true, "2026-11-24T12:00:06Z"]);
  });

  test("a hundred and more changed in the same second are read a page of that second at a time", async () => {
    // 130 swept in one second on the 22nd, and one an hour after.
    const same = Array.from({ length: 130 }, (_, i) =>
      pull(2000 + i, "fix(id): swept", "2026-11-22T00:00:00Z", "2026-11-22T00:00:00Z"),
    );
    const after = pull(3000, "feat(id): after the sweep", "2026-11-22T01:00:00Z", "2026-11-22T01:00:00Z");
    const w = world([...same, after]);
    await w.trends.advance(3);
    // The last day; then the stretch from noon on the 21st: its first page is all one second, and so is the page asked from that second.
    expect(since(w.asked)).toEqual(["2026-11-23T12:00:00Z", "2026-11-21T12:00:00Z", "2026-11-22T00:00:00Z"]);
    expect((await w.meta()).back).toEqual({
      from: "2026-11-21T12:00:00Z",
      cursor: "2026-11-22T00:00:00Z",
      page: 2,
      until: "2026-11-23T12:00:00Z",
    });
    // Stopped there and started again, it goes on with the second page of that second: nothing is skipped.
    const meta = await w.trends.advance(20);
    expect(since(w.asked)[4]).toBe("2026-11-22T00:00:00Z&page=2");
    expect(meta.whole).toBe(true);
    const day = (await w.day("2026-11-22")) as Day;
    expect(Object.keys(day.pm).length).toBe(131);
    expect(Object.keys(day.pm)).toContain("3000");
  });

  test("a page that cannot be read stops it there with what came before kept, and the next read goes on", async () => {
    const w = world(many());
    w.state.fail = (_path, n) => (n === 4 ? "GitHub could not be reached: timed out" : null);
    await expect(w.trends.advance(20)).rejects.toThrow("GitHub could not be reached");
    const meta = await w.meta();
    // The last day, the stretch from the 21st and one page of the stretch before it were read; the fourth ask failed.
    expect([meta.whole, meta.live, meta.back?.cursor]).toEqual([false, true, "2026-11-20T16:30:00Z"]);
    expect((await kept(w)).size).toBe(134);
    w.state.fail = null;
    expect((await w.trends.advance(20)).whole).toBe(true);
    // One look at what is new, then on from where it stopped.
    expect(since(w.asked).slice(4, 6)).toEqual(["2026-11-23T12:00:00Z", "2026-11-20T16:30:00Z"]);
    expect((await kept(w)).size).toBe(250);
    // An answer that is not a list is no answer.
    const odd = world([]);
    (odd.ops as unknown as { gh: () => Promise<unknown> }).gh = async () => ({ message: "Bad credentials" });
    await expect(odd.trends.advance(1)).rejects.toThrow("was not a list");
  });

  test("two readers at once are one reading", async () => {
    const w = world(many());
    const [a, b] = await Promise.all([w.trends.advance(20), w.trends.advance(20)]);
    expect(a).toBe(b);
    expect(w.asked.length).toBe(9);
  });

  test("what is kept that is not what was written is read again from the start, not trusted", async () => {
    const w = world(many());
    await w.shelf.put("meta", "{not json");
    await w.shelf.put("day:2026-11-20", "also not json");
    expect((await w.trends.advance(20)).whole).toBe(true);
    expect(since(w.asked)[0]).toBe("2026-11-23T12:00:00Z");
    expect(Object.keys(((await w.day("2026-11-20")) as Day).pm).length).toBe(144);
    // The same for a note of where it had got to that is JSON but not the note.
    const other = world(many());
    await other.shelf.put("meta", JSON.stringify({ from: "2026-08-26", cursor: "2026-11-01T00:00:00Z", whole: true }));
    expect((await other.trends.advance(20)).whole).toBe(true);
    expect(since(other.asked)[0]).toBe("2026-11-23T12:00:00Z");
    expect((await kept(other)).size).toBe(250);
  });
});

describe("tasks finished, by day and kind, from the queue's store", () => {
  test("each task once, on the day it was last finished, while it stands as done; and the store's index is used", async () => {
    const q = queueDb();
    q.task(1, "feature slice");
    q.event(1, "completed", "2026-11-22T10:00:00Z");
    q.task(2, "feature slice");
    q.event(2, "completed", "2026-11-22T11:00:00Z");
    // Finished, taken up again, finished the next day: counted on the next day only.
    q.event(2, "claimed", "2026-11-22T12:00:00Z");
    q.event(2, "completed", "2026-11-23T09:00:00Z");
    q.task(3, "small fix");
    q.event(3, "staleness_autocomplete", "2026-11-23T10:00:00Z");
    // No kind, and a payload that is not JSON: counted, with no kind.
    q.task(4, null);
    q.event(4, "completed", "2026-11-23T11:00:00Z");
    // Not done any more, another queue's, before the time asked, and an event that is not a finishing.
    q.task(5, "small fix", "queued");
    q.event(5, "completed", "2026-11-23T12:00:00Z");
    q.task(6, "small fix", "done", "another-queue");
    q.event(6, "completed", "2026-11-23T12:00:00Z");
    q.task(7, "small fix");
    q.event(7, "completed", "2026-11-01T12:00:00Z");
    q.task(8, "small fix");
    q.event(8, "claimed", "2026-11-23T12:00:00Z");
    const rows = q.db.query(TASKS_BY_DAY_SQL).all("2026-11-20T00:00:00Z", CFG.queue);
    expect(rows).toEqual([
      { day: "2026-11-22", kind: "feature slice", n: 1 },
      { day: "2026-11-23", kind: "", n: 1 },
      { day: "2026-11-23", kind: "feature slice", n: 1 },
      { day: "2026-11-23", kind: "small fix", n: 1 },
    ]);
    const plan = (
      q.db.query(`EXPLAIN QUERY PLAN ${TASKS_BY_DAY_SQL}`).all("2026-11-20T00:00:00Z", CFG.queue) as {
        detail: string;
      }[]
    )
      .map((r) => r.detail)
      .join(" | ");
    // The events by their kind and time, each task by its key: nothing is read from end to end.
    expect(plan).toContain("SEARCH e USING INDEX events_kind_ts (kind=? AND created_at>?)");
    expect(plan).toContain("SEARCH t USING INTEGER PRIMARY KEY (rowid=?)");
    expect(plan).not.toContain("SCAN e");
    expect(plan).not.toContain("SCAN t");
  });
});

describe("the document the Trends screen draws", () => {
  const items = () => [
    // The window (7 days to the 24th) is the 18th to the 24th; the 7 before it, the 11th to the 17th.
    pull(1, "feat(id): alpha", "2026-11-23T08:00:00Z", "2026-11-23T10:00:00Z"),
    pull(2, "feat(id): beta", "2026-11-24T08:00:00Z", "2026-11-24T08:30:00Z"),
    pull(3, "fix(fleet-queue): gamma", "2026-11-24T09:00:00Z", "2026-11-24T10:00:00Z"),
    pull(4, "feat(pm-console): delta", "2026-11-12T09:00:00Z", "2026-11-12T10:00:00Z"),
    pull(5, "words only", "2026-11-24T09:00:00Z", "2026-11-24T09:10:00Z"),
    pull(6, "docs(atlas): closed unmerged", "2026-11-24T09:00:00Z", null, { closed_at: "2026-11-24T09:30:00Z" }),
    issue(7, "2026-11-24T07:00:00Z", { labels: [{ name: "kind/bug" }] }),
    issue(8, "2026-11-20T07:00:00Z", { closed_at: "2026-11-24T07:30:00Z", labels: [{ name: "bug" }] }),
    issue(9, "2026-11-23T07:00:00Z"),
  ];

  test("a week set against the week before: every series a number a day, oldest first", async () => {
    const q = queueDb();
    q.task(1, "feature slice");
    q.event(1, "completed", "2026-11-24T10:00:00Z");
    q.task(2, "small fix");
    q.event(2, "completed", "2026-11-23T10:00:00Z");
    q.task(3, "feature slice");
    q.event(3, "completed", "2026-11-12T10:00:00Z");
    const w = world(items(), q.db);
    // The history is read to its end first, as the background does.
    await w.trends.advance(20);
    const doc = await w.trends.read("7");
    expect(doc.window).toBe(7);
    expect(doc.today).toBe("2026-11-24");
    expect(doc.days).toEqual(daysBack("2026-11-24", 14));
    expect(doc.days.length).toBe(14);
    const last = <T>(list: T[], n = 3) => list.slice(-n);
    // Merged: the 12th has one (index 1), the 23rd one, the 24th three.
    expect(doc.merged.all).toEqual([0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 3]);
    expect(last(doc.merged.kinds.feat)).toEqual([0, 1, 1]);
    expect(last(doc.merged.kinds.fix)).toEqual([0, 0, 1]);
    expect(last(doc.merged.kinds.other)).toEqual([0, 0, 1]);
    expect(last(doc.merged.projects.id)).toEqual([0, 1, 1]);
    expect(last(doc.merged.projects.fleet)).toEqual([0, 0, 1]);
    expect(last(doc.merged.projects.other)).toEqual([0, 0, 1]);
    expect(doc.merged.projects["pm-stack"][1]).toBe(1);
    // The middle time to merge on the 24th: 10, 30 and 60 minutes; the 23rd: 120; none on the 22nd.
    expect(last(doc.merged.minutes)).toEqual([null, 120, 30]);
    // Features by project, and each project's latest within the window, newest first.
    expect(last(doc.features.projects.id)).toEqual([0, 1, 1]);
    expect(doc.features.projects.fleet.every((n) => n === 0)).toBe(true);
    expect(doc.features.latest.id).toEqual([
      { number: 2, title: "beta", at: "2026-11-24T08:30:00Z", url: `https://github.com/${CFG.repo}/pull/2` },
      { number: 1, title: "alpha", at: "2026-11-23T10:00:00Z", url: `https://github.com/${CFG.repo}/pull/1` },
    ]);
    // The pm-stack feature merged before the window: counted in its day, not listed.
    expect(doc.features.latest["pm-stack"]).toEqual([]);
    expect(doc.features.projects["pm-stack"][1]).toBe(1);
    expect(last(doc.issues.opened, 5)).toEqual([1, 0, 0, 1, 1]);
    expect(last(doc.issues.closed)).toEqual([0, 0, 1]);
    expect(last(doc.bugs.opened, 5)).toEqual([1, 0, 0, 0, 1]);
    expect(last(doc.bugs.closed)).toEqual([0, 0, 1]);
    expect(last(doc.prs.opened)).toEqual([0, 1, 4]);
    expect(last(doc.prs.closed)).toEqual([0, 0, 1]);
    expect(doc.projects.map((p) => p.id)).toEqual(["id", "fleet", "atlas", "pm-stack", "mcpx", "other"]);
    // Read to its end in one go (there is little of it): every day is whole, back to the start.
    expect(doc.history).toEqual({
      from: "2026-08-26",
      whole: true,
      covered: "2026-08-26T00:00:00Z",
      readTo: "2026-11-24T10:00:00Z",
      readAt: NOW.toISOString(),
    });
    // Tasks: the queue was asked from the first day shown.
    expect(doc.tasks).toEqual({
      all: [0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1],
      kinds: [
        { kind: "feature slice", days: [0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1] },
        { kind: "small fix", days: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0] },
      ],
      since: "2026-11-12",
    });
    expect(doc.partial).toBeUndefined();
  });

  test("the windows on offer, and anything else is thirty days", async () => {
    const w = world(items(), queueDb().db);
    await w.trends.advance(20);
    expect((await w.trends.read("90")).days.length).toBe(90);
    expect((await w.trends.read("30")).days.length).toBe(60);
    for (const odd of [null, "", "0", "8", "abc", "-7", "1e2"])
      expect([(await w.trends.read(odd)).window, odd]).toEqual([30, odd]);
    // Ninety days back is all the history there is: no day before it is shown.
    expect((await w.trends.read("90")).days[0]).toBe("2026-08-27");
  });

  test("a project lists its latest twelve features, and no more", async () => {
    const lots = Array.from({ length: 15 }, (_, i) => {
      const at = `2026-11-24T${String(i).padStart(2, "0")}:00:00Z`;
      return pull(100 + i, `feat(mcpx): number ${i}`, at, at);
    });
    const w = world(lots, queueDb().db);
    await w.trends.advance(20);
    const doc = await w.trends.read("7");
    expect(doc.features.latest.mcpx.length).toBe(12);
    expect(doc.features.latest.mcpx[0].title).toBe("number 14");
    expect(doc.features.latest.mcpx[11].title).toBe("number 3");
    expect(doc.features.projects.mcpx[doc.days.length - 1]).toBe(15);
  });

  test("opened before the history is whole, the screen reads three pages on each time and says how far back it is whole", async () => {
    const w = world(items(), queueDb().db);
    const first = await w.trends.read("7");
    // The last day (to its end), and two older stretches.
    expect(w.asked.length).toBe(3);
    expect([first.history.whole, first.history.covered]).toEqual([false, "2026-11-17T12:00:00Z"]);
    // What changed in the last week is all there already: the three merged on the 24th.
    expect(first.merged.all.slice(-1)).toEqual([3]);
    // Not yet the one from the 12th, which is older than a week.
    expect(first.merged.all[1]).toBe(0);
    const second = await w.trends.read("7");
    expect(w.asked.length).toBe(6);
    expect([second.history.whole, second.history.covered]).toEqual([false, "2026-10-25T12:00:00Z"]);
    expect(second.merged.all[1]).toBe(1);
    const third = await w.trends.read("7");
    expect([third.history.whole, third.history.covered, w.asked.length]).toEqual([true, "2026-08-26T00:00:00Z", 9]);
  });

  test("read again within five minutes GitHub is not asked; pressed to refresh, it is", async () => {
    const w = world(items(), queueDb().db);
    await w.trends.advance(20);
    await w.trends.read("7");
    const asked = w.asked.length;
    await w.trends.read("7");
    await w.trends.read("30");
    expect(w.asked.length).toBe(asked);
    await w.trends.read("7", true);
    expect(w.asked.length).toBe(asked + 1);
  });

  test("with GitHub out of reach it shows what was kept and says so; with the queue out of reach, the rest", async () => {
    const w = world(items(), queueDb().db);
    await w.trends.advance(20);
    await w.trends.read("7");
    w.state.fail = () => "GitHub could not be reached: timed out";
    const stale = await w.trends.read("7", true);
    expect(stale.partial).toBe(true);
    expect(stale.why).toBe(
      "GitHub's history could not be read on: fleet gh failed (exit 1): GitHub could not be reached: timed out",
    );
    expect(stale.merged.all.slice(-1)).toEqual([3]);
    w.state.fail = null;
    w.state.queueDown = true;
    const half = await w.trends.read("7", true);
    expect(half.partial).toBe(true);
    expect(half.why).toContain("The queue's finished tasks could not be read");
    expect("error" in half.tasks).toBe(true);
    expect(half.merged.all.slice(-1)).toEqual([3]);
    // Never read at all, and GitHub out of reach: days of nothing, said to be partial, history not whole.
    const cold = world(items(), queueDb().db);
    cold.state.fail = () => "GitHub could not be reached: timed out";
    const none = await cold.trends.read("7");
    expect([none.partial, none.history.whole, none.merged.all.every((n) => n === 0)]).toEqual([true, false, true]);
  });
});
