// The primary screens: what is in progress, what is next, what needs the
// owner, and what he has asked for.
//
// Oracles: the queue store, the dashboard and the fleet's commands are
// recorders, so every read and write is compared with what it should be
// as an exact statement, request or argument vector. The questions store
// is a file written by the test, so "open" and "recommended" are defined
// here and not by the code under test.
import { Database } from "bun:sqlite";
import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { gunzipSync } from "node:zlib";
import { actions, perform } from "../src/actions.ts";
import { readConfig } from "../src/config.ts";
import { Docs, Stamped } from "../src/docs.ts";
import { Named } from "../src/named.ts";
import { type Fetch, OpError, Ops, type Run } from "../src/ops.ts";
import { type Deps, handle, loaders } from "../src/server.ts";
import { memoryStore } from "../src/store.ts";
import { BLOCKED_KINDS_SQL, DONE_COUNT_SQL, DONE_TASKS_SQL, promptPacket, Work } from "../src/work.ts";
import { asThePlatformFetch } from "./support.ts";

const CFG = readConfig({ HOME: "/h" });
const REPO = "developing-today/code";

type Sql = { sql: string; args: (string | number)[] };
/** A queue store and a dashboard in one fetch, both recording. */
function world(opts: {
  rows?: (st: Sql) => Record<string, unknown>[] | { affected: number };
  dash?: (path: string, body: unknown) => unknown | { status: number; error: string };
  commands?: (argv: string[], stdin?: string) => string | { code: number; stderr: string };
}) {
  const sqls: Sql[][] = [];
  const dashed: { path: string; body: unknown }[] = [];
  const calls: { argv: string[]; stdin?: string; packet?: unknown }[] = [];
  const fetchImpl: Fetch = async (url, init) => {
    if (url.startsWith(CFG.queueDbUrl)) {
      const requests = (
        JSON.parse(init.body ?? "{}").requests as {
          type: string;
          stmt?: { sql: string; args: { value: string; type: string }[] };
        }[]
      ).filter((r) => r.type === "execute");
      const batch = requests.map((r) => ({
        sql: r.stmt?.sql ?? "",
        args: (r.stmt?.args ?? []).map((a) => (a.type === "integer" ? Number(a.value) : a.value)),
      }));
      sqls.push(batch);
      return {
        ok: true,
        status: 200,
        json: async () => ({
          results: batch.map((st) => {
            const out = opts.rows?.(st) ?? [];
            if (!Array.isArray(out))
              return { type: "ok", response: { result: { cols: [], rows: [], affected_row_count: out.affected } } };
            const cols = Object.keys(out[0] ?? {}).map((name) => ({ name }));
            return {
              type: "ok",
              response: { result: { cols, rows: out.map((r) => cols.map((c) => ({ value: r[c.name] }))) } },
            };
          }),
        }),
      };
    }
    const path = url.replace(CFG.dashboardUrl ?? "", "");
    const body = init.body ? JSON.parse(init.body) : undefined;
    dashed.push({ path, body });
    const out = opts.dash?.(path, body) ?? {};
    const failed = out !== null && typeof out === "object" && "status" in out && "error" in out;
    return { ok: !failed, status: failed ? (out as { status: number }).status : 200, json: async () => out };
  };
  const run: Run = async (argv, o) => {
    const entry: { argv: string[]; stdin?: string; packet?: unknown } = { argv, stdin: o?.stdin };
    if (argv[1] === "queue" && argv[2] === "enqueue") entry.packet = JSON.parse(readFileSync(argv[3], "utf8"));
    calls.push(entry);
    if (argv[1] === "secrets") return { code: 0, stdout: "token\n", stderr: "" };
    const a = opts.commands?.(argv, o?.stdin) ?? "{}";
    return typeof a === "string" ? { code: 0, stdout: a, stderr: "" } : { code: a.code, stdout: "", stderr: a.stderr };
  };
  // As the Worker runtime's fetch is (these classes run there too): not to be called as a method.
  const ops = new Ops(CFG, run, asThePlatformFetch(fetchImpl) as Fetch);
  return { ops, sqls, dashed, calls, gh: () => calls.filter((c) => c.argv[1] === "gh").map((c) => c.argv.slice(3)) };
}

const TOKEN = "set-token-never-leaves-the-host";
function store(extra: Record<string, unknown> = {}) {
  const q = (qid: string, more: Record<string, unknown> = {}) => ({
    qid,
    question: `Question ${qid}?`,
    context: "c".repeat(900),
    description: "d",
    options: [
      { id: "opt-1", label: "First", tags: ["recommended"] },
      { id: "opt-2", label: "Second" },
    ],
    recommendedOptionIds: ["opt-1"],
    ...more,
  });
  return JSON.stringify({
    version: 1,
    sets: [
      {
        setId: "old-set",
        title: "Older",
        token: `${TOKEN}-old`,
        createdAt: "2026-11-10T00:00:00Z",
        items: [{ ledgerId: "L0", title: "Old item", group: "g", links: [], questions: [q("a")] }],
      },
      {
        setId: "new-set",
        title: "Newer",
        token: TOKEN,
        createdAt: "2026-11-17T00:00:00Z",
        items: [
          {
            ledgerId: "L1",
            title: "Item one",
            group: "g",
            links: ["https://github.com/developing-today/code/issues/10036"],
            questions: [
              q("b", { order: 2 }),
              q("c", { order: 1, mode: "multi", recommendedOptionIds: ["opt-1", "opt-2"] }),
              q("answered", { resolution: { kind: "answered", at: "x", via: "y" } }),
              q("norec", { order: 3, recommendedOptionIds: [] }),
            ],
          },
        ],
      },
    ],
    ...extra,
  });
}

// What was finished lately. The oracle for the queue's half is SQLite
// itself, with the queue's own tables and indexes (fleet-queue's
// SCHEMA): the statements are run on rows written the way the fleet
// writes them. A task is finished when an event says so; it is shown
// once, at its last finishing, and only while it still stands as done.
// The oracle for GitHub's half is the shape GitHub documents for
// "list repository issues": a pull request there carries
// `pull_request.merged_at`, an issue `state_reason`.
describe("what was finished lately", () => {
  const NOW = new Date("2026-11-24T12:00:00Z");
  const ago = (hours: number) => new Date(NOW.getTime() - hours * 3_600_000).toISOString().replace(/\.\d+Z$/, "Z");
  const db = new Database(":memory:");
  db.run(
    "CREATE TABLE tasks (id INTEGER PRIMARY KEY, queue TEXT NOT NULL, title TEXT, payload TEXT NOT NULL DEFAULT '{}', priority INTEGER, status TEXT NOT NULL DEFAULT 'queued', lane_id TEXT, claim_model TEXT, result TEXT NOT NULL DEFAULT '{}', created_at TEXT)",
  );
  db.run("CREATE INDEX tasks_claim_scan ON tasks(queue, status, priority, created_at)");
  db.run(
    "CREATE TABLE events (id INTEGER PRIMARY KEY AUTOINCREMENT, worker_id TEXT, task_id INTEGER, kind TEXT NOT NULL, detail TEXT NOT NULL DEFAULT '{}', created_at TEXT NOT NULL)",
  );
  db.run("CREATE INDEX events_task ON events(task_id, id)");
  db.run("CREATE INDEX events_kind_ts ON events(kind, created_at)");
  const task = (
    id: number,
    row: {
      queue?: string;
      status?: string;
      title?: string;
      payload?: string;
      result?: string;
      lane?: string;
      model?: string;
    },
  ) =>
    db.run(
      "INSERT INTO tasks (id, queue, title, payload, status, lane_id, claim_model, result) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
      [
        id,
        row.queue ?? CFG.queue,
        row.title ?? `Task ${id}`,
        row.payload ?? "{}",
        row.status ?? "done",
        row.lane ?? null,
        row.model ?? null,
        row.result ?? "{}",
      ],
    );
  const event = (taskId: number, kind: string, hoursAgo: number) =>
    db.run("INSERT INTO events (task_id, kind, created_at) VALUES (?, ?, ?)", [taskId, kind, ago(hoursAgo)]);
  // Finished an hour ago by a merged pull request, as the completion record says.
  task(1, {
    title: "Fix the claim scan",
    lane: "code-a",
    model: "m-1",
    payload: JSON.stringify({ kind: "small_fix", issue: { number: 5 } }),
    result: JSON.stringify({ disposition: { verdict: "merged_verified", pulls: [{ number: 77 }, { number: 78 }] } }),
  });
  event(1, "claimed", 2);
  event(1, "completed", 1);
  // Found already done by the queue's own sweep; its pull request is written as an address.
  task(2, { result: JSON.stringify({ pr: "https://github.com/o/r/pull/88" }) });
  event(2, "staleness_autocomplete", 2);
  // Finished, taken up again, finished again: once, at the later time.
  task(3, { payload: JSON.stringify({ kind: "triage", pr: { number: 9 } }) });
  event(3, "completed", 30);
  event(3, "revived", 29);
  event(3, "completed", 3);
  // Finished, then taken up again and still going: not done.
  task(4, { status: "queued" });
  event(4, "completed", 5);
  // Another queue's.
  task(5, { queue: "other" });
  event(5, "completed", 1);
  // Finished more than a week ago.
  task(6, {});
  event(6, "completed", 8 * 24);
  // Written by something older: neither its packet nor its result is JSON.
  task(7, { payload: "not json", result: "done by hand" });
  event(7, "completed", 4);
  // Marked done with no event that says so in the week: not found this way.
  task(8, {});
  event(8, "claimed", 1);
  // Finished twice within the day, the second time in the same second
  // as task 7: once in the list, once in the count, and before task 7.
  task(10, {});
  event(10, "completed", 6);
  event(10, "revived", 5);
  event(10, "completed", 4);
  // Finished yesterday, more than a day ago.
  task(9, { result: JSON.stringify({ disposition: { verdict: "closed_verified", pulls: [] }, pr: 12 }) });
  event(9, "completed", 26);

  test("each finished task is found once, newest first, by the events that say so", () => {
    expect(db.query(DONE_TASKS_SQL).all(ago(7 * 24), CFG.queue) as Record<string, unknown>[]).toEqual([
      {
        id: 1,
        title: "Fix the claim scan",
        at: ago(1).slice(0, 19),
        lane: "code-a",
        model: "m-1",
        kind: "small_fix",
        issue: 5,
        pr: 77,
        verdict: "merged_verified",
      },
      {
        id: 2,
        title: "Task 2",
        at: ago(2).slice(0, 19),
        lane: null,
        model: null,
        kind: null,
        issue: null,
        pr: "https://github.com/o/r/pull/88",
        verdict: null,
      },
      {
        id: 3,
        title: "Task 3",
        at: ago(3).slice(0, 19),
        lane: null,
        model: null,
        kind: "triage",
        issue: null,
        pr: 9,
        verdict: null,
      },
      {
        id: 10,
        title: "Task 10",
        at: ago(4).slice(0, 19),
        lane: null,
        model: null,
        kind: null,
        issue: null,
        pr: null,
        verdict: null,
      },
      {
        id: 7,
        title: "Task 7",
        at: ago(4).slice(0, 19),
        lane: null,
        model: null,
        kind: null,
        issue: null,
        pr: null,
        verdict: null,
      },
      {
        id: 9,
        title: "Task 9",
        at: ago(26).slice(0, 19),
        lane: null,
        model: null,
        kind: null,
        issue: null,
        pr: 12,
        verdict: "closed_verified",
      },
    ]);
    // In the last day: the same tasks less yesterday's, each once.
    expect(db.query(DONE_COUNT_SQL).get(ago(24), CFG.queue)).toEqual({ n: 5 });
    expect(db.query(DONE_COUNT_SQL).get(ago(24), "nobody")).toEqual({ n: 0 });
  });

  test("it is read from the events of the time asked for, not by going through every task", () => {
    for (const sql of [DONE_TASKS_SQL, DONE_COUNT_SQL]) {
      const plan = (db.query(`EXPLAIN QUERY PLAN ${sql}`).all(ago(24), CFG.queue) as { detail: string }[]).map(
        (r) => r.detail,
      );
      // The events of those kinds since that time, by their index; each task by its number.
      expect(plan.filter((line) => /events_kind_ts \(kind=\? AND created_at>\?\)/.test(line))).toHaveLength(1);
      expect(plan.filter((line) => /SEARCH t USING INTEGER PRIMARY KEY/.test(line))).toHaveLength(1);
      expect(plan.filter((line) => /SCAN/.test(line))).toEqual([]);
    }
  });

  const CLOSED = [
    // A pull request merged two hours ago.
    {
      number: 101,
      title: "feat: live events",
      html_url: "https://github.com/o/r/pull/101",
      closed_at: ago(2),
      updated_at: ago(2),
      user: { login: "lane-a" },
      pull_request: { merged_at: ago(2) },
    },
    // Another merged in the same second: the later-numbered is shown first.
    {
      number: 108,
      title: "fix: same second",
      html_url: "https://github.com/o/r/pull/108",
      closed_at: ago(2),
      updated_at: ago(2),
      user: { login: "lane-c" },
      pull_request: { merged_at: ago(2) },
    },
    // One closed without merging: not something done.
    {
      number: 102,
      title: "wip: abandoned",
      html_url: "https://github.com/o/r/pull/102",
      closed_at: ago(1),
      updated_at: ago(1),
      user: { login: "lane-b" },
      pull_request: { merged_at: null },
    },
    // An issue closed as done, and one closed as not planned.
    {
      number: 103,
      title: "Board shows 394",
      html_url: "https://github.com/o/r/issues/103",
      closed_at: ago(3),
      updated_at: ago(3),
      state_reason: "completed",
      user: { login: "owner" },
    },
    {
      number: 104,
      title: "Duplicate of 103",
      html_url: "https://github.com/o/r/issues/104",
      closed_at: ago(4),
      updated_at: ago(4),
      state_reason: "not_planned",
      user: { login: "owner" },
    },
    // Merged yesterday, more than a day ago; and one long ago that was only commented on lately.
    {
      number: 105,
      title: "fix: yesterday",
      html_url: "https://github.com/o/r/pull/105",
      closed_at: ago(30),
      updated_at: ago(30),
      user: { login: "lane-a" },
      pull_request: { merged_at: ago(30) },
    },
    {
      number: 106,
      title: "fix: last month",
      html_url: "https://github.com/o/r/pull/106",
      closed_at: ago(30 * 24),
      updated_at: ago(1),
      user: { login: "lane-a" },
      pull_request: { merged_at: ago(30 * 24) },
    },
    // Things that are not what GitHub sends are left out, not shown as nonsense.
    null,
    { number: "x", title: 7 },
    { number: 107, title: "no time on it", html_url: "https://github.com/o/r/issues/107", closed_at: null, user: null },
  ];
  const rows = (st: Sql) =>
    st.sql === DONE_TASKS_SQL
      ? (db.query(DONE_TASKS_SQL).all(...(st.args as [string, string])) as Record<string, unknown>[])
      : st.sql === DONE_COUNT_SQL
        ? [db.query(DONE_COUNT_SQL).get(...(st.args as [string, string])) as Record<string, unknown>]
        : [];

  test("the tasks finished, the pull requests merged and the issues closed are read together, newest first", async () => {
    const w = world({ rows, commands: (argv) => (argv[1] === "gh" ? JSON.stringify(CLOSED) : "{}") });
    const done = await new Work(w.ops, undefined, () => NOW).done();
    // One round trip to the queue store, one question of GitHub, at an
    // address that does not change from one asking to the next.
    expect(w.sqls).toEqual([
      [
        { sql: DONE_TASKS_SQL, args: [ago(7 * 24), CFG.queue] },
        { sql: DONE_COUNT_SQL, args: [ago(24), CFG.queue] },
      ],
    ]);
    expect(w.gh()).toEqual([[`repos/${REPO}/issues?state=closed&sort=updated&direction=desc&per_page=100`]]);
    expect(done).toEqual({
      tasks: {
        day: 5,
        latest: [
          {
            id: 1,
            title: "Fix the claim scan",
            at: ago(1).slice(0, 19),
            lane: "code-a",
            model: "m-1",
            kind: "small_fix",
            issue: 5,
            pr: 77,
            verdict: "merged_verified",

            url: "https://github.com/developing-today/code/issues/5",

            prUrl: "https://github.com/developing-today/code/pull/77",
          },
          {
            id: 2,
            title: "Task 2",
            at: ago(2).slice(0, 19),
            lane: null,
            model: null,
            kind: null,
            issue: null,
            pr: 88,
            verdict: null,

            url: null,

            prUrl: "https://github.com/developing-today/code/pull/88",
          },
          {
            id: 3,
            title: "Task 3",
            at: ago(3).slice(0, 19),
            lane: null,
            model: null,
            kind: "triage",
            issue: null,
            pr: 9,
            verdict: null,

            url: null,

            prUrl: "https://github.com/developing-today/code/pull/9",
          },
          {
            id: 10,
            title: "Task 10",
            at: ago(4).slice(0, 19),
            lane: null,
            model: null,
            kind: null,
            issue: null,
            pr: null,
            verdict: null,

            url: null,

            prUrl: null,
          },
          {
            id: 7,
            title: "Task 7",
            at: ago(4).slice(0, 19),
            lane: null,
            model: null,
            kind: null,
            issue: null,
            pr: null,
            verdict: null,

            url: null,

            prUrl: null,
          },
          {
            id: 9,
            title: "Task 9",
            at: ago(26).slice(0, 19),
            lane: null,
            model: null,
            kind: null,
            issue: null,
            pr: 12,
            verdict: "closed_verified",

            url: null,

            prUrl: "https://github.com/developing-today/code/pull/12",
          },
        ],
      },
      merged: {
        day: 2,
        more: false,
        latest: [
          {
            number: 108,
            title: "fix: same second",
            url: "https://github.com/o/r/pull/108",
            at: ago(2).slice(0, 19),
            by: "lane-c",
          },
          {
            number: 101,
            title: "feat: live events",
            url: "https://github.com/o/r/pull/101",
            at: ago(2).slice(0, 19),
            by: "lane-a",
          },
          {
            number: 105,
            title: "fix: yesterday",
            url: "https://github.com/o/r/pull/105",
            at: ago(30).slice(0, 19),
            by: "lane-a",
          },
        ],
      },
      closed: {
        day: 2,
        more: false,
        latest: [
          {
            number: 103,
            title: "Board shows 394",
            url: "https://github.com/o/r/issues/103",
            at: ago(3).slice(0, 19),
            reason: "completed",
          },
          {
            number: 104,
            title: "Duplicate of 103",
            url: "https://github.com/o/r/issues/104",
            at: ago(4).slice(0, 19),
            reason: "not_planned",
          },
        ],
      },
    });
  });

  test("a full page of things closed within the day is said to be at least that many", async () => {
    const page = Array.from({ length: 100 }, (_, i) => ({
      number: 1000 + i,
      title: `fix: ${i}`,
      html_url: `https://github.com/o/r/pull/${1000 + i}`,
      closed_at: ago(i / 10),
      updated_at: ago(i / 10),
      user: { login: "lane" },
      pull_request: { merged_at: ago(i / 10) },
    }));
    let sent: unknown[] = page;
    const w = world({ rows: () => [], commands: (argv) => (argv[1] === "gh" ? JSON.stringify(sent) : "{}") });
    const done = await new Work(w.ops, undefined, () => NOW).done();
    // A hundred came and the oldest of them is within the day: there may be more.
    expect([done.merged.day, done.merged.more, done.merged.latest.length]).toEqual([100, true, 40]);
    expect([done.closed.day, done.closed.more]).toEqual([0, true]);
    expect(done.merged.latest[0].number).toBe(1000);
    // No count of tasks came back: none, not nonsense.
    expect(done.tasks).toEqual({ day: 0, latest: [] });
    // The same page with its oldest from before the day: that is all of them.
    page[99].closed_at = ago(25);
    page[99].updated_at = ago(25);
    page[99].pull_request.merged_at = ago(25);
    const again = await new Work(w.ops, undefined, () => NOW).done();
    expect([again.merged.day, again.merged.more]).toEqual([99, false]);
    // The list is in the order things were last touched, not closed: a
    // full page whose last was closed long ago but touched within the
    // day may still have things closed today after it.
    page[99].updated_at = ago(23);
    const touched = await new Work(w.ops, undefined, () => NOW).done();
    expect([touched.merged.day, touched.merged.more]).toEqual([99, true]);
    // One short of a full page, every one touched within the day: that is all there is.
    sent = page.slice(0, 99);
    const short = await new Work(w.ops, undefined, () => NOW).done();
    expect([short.merged.day, short.merged.more, short.closed.more]).toEqual([99, false, false]);
  });
});

// Why tasks are blocked, by the kind of reason. The oracle is SQLite
// itself (the queue's store speaks its SQL): the query is run on rows
// like the ones seen in the live queue on 2026-11-24, where almost
// every blocked task carried a reason of its own, a few waited for a
// time long past, and a few named other tasks.
describe("why tasks are blocked", () => {
  const db = new Database(":memory:");
  db.run(
    "CREATE TABLE tasks (id INTEGER PRIMARY KEY, queue TEXT, status TEXT, progress TEXT, ready_condition TEXT, blocked_deps TEXT)",
  );
  const add = (row: {
    queue?: string;
    status?: string;
    progress?: string | null;
    ready?: string | null;
    deps?: string | null;
  }) =>
    db.run("INSERT INTO tasks (queue, status, progress, ready_condition, blocked_deps) VALUES (?, ?, ?, ?, ?)", [
      row.queue ?? "q",
      row.status ?? "blocked",
      row.progress ?? null,
      row.ready ?? null,
      row.deps ?? null,
    ]);
  const kinds = () =>
    Object.fromEntries(
      (db.query(BLOCKED_KINDS_SQL).all("q") as { kind: string; n: number }[]).map((r) => [r.kind, r.n]),
    );

  test("each blocked task is counted once, under the kind of its reason", () => {
    const past = "2020-01-01T00:00:00Z";
    const ahead = "2999-01-01T00:00:00.171848+00:00";
    // A reason or a blocker written on the task, each its own text.
    add({ progress: JSON.stringify({ reason: "Real-client fixture required" }) });
    add({ progress: JSON.stringify({ blocker: "Merged contract lacks claim selection" }) });
    add({ progress: JSON.stringify({ reason: "another" }), ready: JSON.stringify({ kind: "time", until: past }) });
    // Waiting for a time: passed, and still to come, in both forms the queue writes.
    add({ ready: JSON.stringify({ kind: "time", until: past }) });
    add({ ready: `{"kind": "time", "until": "${past}"}` });
    add({ ready: JSON.stringify({ kind: "time", until: ahead }) });
    // A condition that is not a time is a reason of its own.
    add({ ready: JSON.stringify({ kind: "pr-merged", pr: 7 }) });
    add({ ready: "when the cache is warm" });
    // Waiting on other tasks.
    add({ deps: "1080" });
    add({ deps: "[1080, 1081]" });
    // Nothing recorded, in each of the ways nothing is written.
    add({});
    add({ progress: "not json", ready: "", deps: "[]" });
    add({ progress: "{}", deps: "" });
    // Not blocked, and another queue's: not counted.
    add({ status: "queued", progress: JSON.stringify({ reason: "x" }) });
    add({ queue: "other", deps: "5" });
    expect(kinds()).toEqual({ reason: 5, "time-past": 2, time: 1, tasks: 2, none: 3 });
    const total = (
      db.query("SELECT count(*) AS n FROM tasks WHERE queue = 'q' AND status = 'blocked'").get() as { n: number }
    ).n;
    expect(Object.values(kinds()).reduce((a, b) => a + b, 0)).toBe(total);
  });

  test("the board carries them, read in the same round trip as the rest", async () => {
    const w = world({
      rows: (st) => (st.sql === BLOCKED_KINDS_SQL ? [{ kind: "reason", n: 4 }] : []),
    });
    const board = await new Work(w.ops).board();
    expect(w.sqls.length).toBe(1);
    expect(w.sqls[0].map((st) => st.sql)).toContain(BLOCKED_KINDS_SQL);
    expect(board.blockedKinds).toEqual([{ kind: "reason", n: 4 }]);
  });
});

describe("what is in progress and what is next", () => {
  test("one round trip, reads only, in the queue's own order", async () => {
    const w = world({
      rows: (st) =>
        st.sql.includes("GROUP BY status")
          ? [
              { status: "claimed", n: 3 },
              { status: "blocked", n: 474 },
            ]
          : st.sql.includes("'claimed'")
            ? [{ id: 7, title: "t", issue: 9075, kind: "small_fix", step: "next thing" }]
            : [],
    });
    const board = await new Work(w.ops).board();
    expect(w.sqls.length).toBe(1);
    expect(w.sqls[0].length).toBe(4);
    for (const st of w.sqls[0]) {
      expect(st.sql.trimStart().startsWith("SELECT")).toBe(true);
      expect(st.args).toEqual(["repo-health-code-pilot"]);
    }
    // In progress: newest claim first. Next: the claim order the queue itself uses.
    expect(w.sqls[0][1].sql).toContain("status = 'claimed' ORDER BY claimed_at DESC");
    expect(w.sqls[0][2].sql).toContain(
      "status = 'queued' ORDER BY (priority IS NULL), priority ASC, created_at ASC, id ASC",
    );
    // A row whose progress or payload is not JSON must not fail the whole read.
    expect(w.sqls[0][1].sql).toContain("json_valid(progress)");
    expect(w.sqls[0][1].sql).toContain("json_valid(payload)");
    expect(board.counts).toEqual({ claimed: 3, blocked: 474 });
    expect(board.now).toEqual([
      {
        id: 7,
        title: "t",
        issue: 9075,
        kind: "small_fix",
        step: "next thing",
        url: `https://github.com/${REPO}/issues/9075`,
      },
    ]);
  });

  test("moving a queued task changes one column on one queued row, along the scale", async () => {
    const run = async (to: string, current: number | null, affected = 1) => {
      const w = world({
        rows: (st) => (st.sql.startsWith("UPDATE") ? { affected } : current === null ? [] : [{ priority: current }]),
      });
      const out = await new Work(w.ops).moveTask({ task: 21016, to }).catch((e: OpError) => e.status);
      return { out, writes: w.sqls.flat().filter((s) => !s.sql.startsWith("SELECT")) };
    };
    const update = (priority: number) => [
      {
        sql: "UPDATE tasks SET priority = ? WHERE id = ? AND queue = ? AND status = 'queued'",
        args: [priority, 21016, "repo-health-code-pilot"],
      },
    ];
    expect(await run("first", 30)).toEqual({ out: { task: 21016, priority: 10 }, writes: update(10) });
    expect(await run("sooner", 30)).toEqual({ out: { task: 21016, priority: 20 }, writes: update(20) });
    expect(await run("later", 30)).toEqual({ out: { task: 21016, priority: 40 }, writes: update(40) });
    expect(await run("later", 50)).toEqual({ out: { task: 21016, priority: 70 }, writes: update(70) });
    // Off-scale values in the store still move to the nearest step in the direction asked.
    expect(await run("sooner", 25)).toEqual({ out: { task: 21016, priority: 20 }, writes: update(20) });
    // Nothing further to move to, a task that is no longer queued, and a bad request: no write.
    expect(await run("sooner", 10)).toEqual({ out: 409, writes: [] });
    expect(await run("later", 90)).toEqual({ out: 409, writes: [] });
    expect(await run("sooner", null)).toEqual({ out: 409, writes: [] });
    expect((await run("first", 30, 0)).out).toBe(409);
    expect(await run("sideways", 30)).toEqual({ out: 400, writes: [] });
    const w = world({});
    for (const [task, priority] of [
      [1, 25],
      [1, 0],
      ["1; DROP", 10],
      [-1, 10],
    ] as const) {
      await expect(w.ops.setTaskPriority(task, priority)).rejects.toThrow(OpError);
    }
    expect(w.sqls).toEqual([]);
    await expect(w.ops.reads([{ sql: "UPDATE tasks SET status = 'done'", args: [] }])).rejects.toThrow(/only reads/);
  });
});

describe("questions waiting on the owner", () => {
  test("only open questions, newest set first, and the answer tokens stay on the host", async () => {
    const w = world({});
    const q = await new Work(w.ops, async () => store()).questions();
    expect(q.total).toBe(4);
    expect(q.sets).toBe(2);
    expect(q.open.map((x) => [x.set, x.qid])).toEqual([
      ["new-set", "c"],
      ["new-set", "b"],
      ["new-set", "norec"],
      ["old-set", "a"],
    ]);
    expect(q.open[0]).toMatchObject({ multi: true, recommended: ["opt-1", "opt-2"], ledger: "L1", setTitle: "Newer" });
    expect(q.open[0].context.length).toBe(280);
    expect(q.open[0].more).toBe(true);
    expect(JSON.stringify(q)).not.toContain(TOKEN);
    // The rest of the background is read when the question is opened.
    const work = new Work(w.ops, async () => store());
    expect(await work.question("new-set", "L1", "b")).toEqual({ context: "c".repeat(900), detail: "d" });
    await expect(work.question("new-set", "L1", "nope")).rejects.toThrow(/not in the store/);
    await expect(work.question("__proto__", "L1", "b")).rejects.toThrow(/not in the store/);
    await expect(new Work(w.ops, async () => "not json").questions()).rejects.toThrow(/questions store/);
    expect((await new Work(w.ops, async () => "{}").questions()).total).toBe(0);
  });

  test("an answer goes through the dashboard's own endpoint with the set's token", async () => {
    const w = world({ dash: () => ({ outcomes: [{ qid: "b", applied: true }] }) });
    const work = new Work(w.ops, async () => store());
    expect(
      await work.answer({ set: "new-set", ledger: "L1", qid: "b", option: "opt-2", comment: " because " }),
    ).toEqual({ set: "new-set", applied: 1, refused: [] });
    expect(w.dashed).toEqual([
      {
        path: "/api/decisions/new-set/submit",
        body: {
          token: TOKEN,
          submissions: [{ ledgerId: "L1", qid: "b", kind: "answered", optionId: "opt-2", comment: "because" }],
        },
      },
    ]);
    await work.answer({ set: "new-set", ledger: "L1", qid: "c", options: ["opt-1", "opt-2"] });
    await work.answer({ set: "new-set", ledger: "L1", qid: "b", text: "my own answer" });
    await work.defer({ set: "new-set", ledger: "L1", qid: "b" });
    expect(w.dashed.slice(1).map((d) => (d.body as { submissions: unknown[] }).submissions[0])).toEqual([
      { ledgerId: "L1", qid: "c", kind: "answered", optionIds: ["opt-1", "opt-2"] },
      { ledgerId: "L1", qid: "b", kind: "answered", freeText: "my own answer" },
      { ledgerId: "L1", qid: "b", kind: "deferred", lean: "approve" },
    ]);
    const before = w.dashed.length;
    for (const args of [
      { set: "new-set", ledger: "L1", qid: "b" }, // nothing chosen
      { set: "../etc", ledger: "L1", qid: "b", option: "opt-1" },
      { set: "missing-set", ledger: "L1", qid: "b", option: "opt-1" },
      { set: "new-set", ledger: "", qid: "b", option: "opt-1" },
      { set: "new-set", ledger: "L1", qid: "b", text: "x".repeat(4001) },
    ]) {
      await expect(work.answer(args)).rejects.toThrow(OpError);
    }
    await expect(work.defer({ set: "new-set", ledger: "L1", qid: "b", lean: "whatever" })).rejects.toThrow(/lean/);
    expect(w.dashed.length).toBe(before);
    // What the dashboard refuses is reported, not hidden.
    const refusing = world({ dash: () => ({ outcomes: [{ qid: "b", applied: false, reason: "already answered" }] }) });
    expect(
      await new Work(refusing.ops, async () => store()).answer({
        set: "new-set",
        ledger: "L1",
        qid: "b",
        option: "opt-1",
      }),
    ).toEqual({ set: "new-set", applied: 0, refused: [{ qid: "b", why: "already answered" }] });
    const down = world({ dash: () => ({ status: 403, error: "unknown set or bad token" }) });
    await expect(
      new Work(down.ops, async () => store()).answer({ set: "new-set", ledger: "L1", qid: "b", option: "opt-1" }),
    ).rejects.toThrow(/bad token/);
  });

  test("accepting recommendations answers exactly the open questions that have one", async () => {
    const w = world({
      dash: (_p, body) => ({
        outcomes: (body as { submissions: { qid: string }[] }).submissions.map((s) => ({ qid: s.qid, applied: true })),
      }),
    });
    const work = new Work(w.ops, async () => store());
    expect((await work.acceptRecommended("new-set")).applied).toBe(2);
    expect((w.dashed[0].body as { submissions: unknown[] }).submissions).toEqual([
      { ledgerId: "L1", qid: "b", kind: "answered", optionId: "opt-1" },
      { ledgerId: "L1", qid: "c", kind: "answered", optionIds: ["opt-1", "opt-2"] },
    ]);
    await expect(work.acceptRecommended("nope")).rejects.toThrow(/not in the store/);
    // The dashboard takes at most 256 answers a request: a large set goes in parts, and all of it goes.
    const large = JSON.parse(store());
    const template = large.sets[1].items[0].questions[0];
    large.sets[1].items[0].questions = Array.from({ length: 450 }, (_, n) => ({ ...template, qid: `q${n}`, order: n }));
    const big = world({
      dash: (_p, body) => ({
        outcomes: (body as { submissions: { qid: string }[] }).submissions.map((x) => ({ qid: x.qid, applied: true })),
      }),
    });
    expect((await new Work(big.ops, async () => JSON.stringify(large)).acceptRecommended("new-set")).applied).toBe(450);
    const sizes = big.dashed.map((d) => (d.body as { submissions: unknown[] }).submissions.length);
    expect(sizes).toEqual([200, 200, 50]);
    expect(Math.max(...sizes)).toBeLessThanOrEqual(256);
    const none = JSON.parse(store());
    none.sets[1].items[0].questions = none.sets[1].items[0].questions.filter((q: { qid: string }) => q.qid === "norec");
    await expect(new Work(w.ops, async () => JSON.stringify(none)).acceptRecommended("new-set")).rejects.toThrow(
      /no open question/,
    );
  });
});

describe("what needs the owner, read with no dashboard", () => {
  const listed = (argv: string[]) => /^repos\/([^/]+\/[^/]+)\/issues\?state=open&labels=([^&]+)&/.exec(argv[3] ?? "");
  const raw = (n: number, labels: string[], body: string | null = null) => ({
    number: n,
    title: `t${n}`,
    body,
    state: "open",
    html_url: `https://github.com/x/y/issues/${n}`,
    updated_at: "2026-11-20T01:02:03Z",
    labels: labels.map((name) => ({ name })),
  });

  test("each repository's own list of each label is asked for through the fleet's gh, and makes the same three lists", async () => {
    const w = world({
      commands: (argv) => {
        const m = listed(argv);
        if (!m) return "[]";
        // The first repository has something of each kind; the others have nothing.
        if (m[1] !== CFG.needsRepos[0]) return "[]";
        const label = decodeURIComponent(m[2]);
        return JSON.stringify(
          [
            raw(1, ["user/escalated"], '<!-- hands:v1 kind=physical action="Power on the host." -->'),
            raw(2, ["user/review"]),
            raw(3, ["user/escalated"], "Reason: an agent could not choose"),
          ].filter((i) => i.labels.some((l) => l.name === label)),
        );
      },
    });
    const c = await new Work(w.ops).censusDirect();
    expect(c.hands.map((i) => [i.ref, i.kind, i.action])).toEqual([
      [`${CFG.needsRepos[0]}#1`, "physical", "Power on the host."],
    ]);
    expect(c.reviews.map((i) => [i.ref, i.updated])).toEqual([[`${CFG.needsRepos[0]}#2`, "2026-11-20T01:02:03"]]);
    expect(c.other.map((i) => [i.ref, i.why])).toEqual([[`${CFG.needsRepos[0]}#3`, "an agent could not choose"]]);
    expect([c.at, c.complete, c.unread]).toEqual([null, true, []]);
    // Asked of GitHub's REST lists, one page of each label for each repository, and never of the search.
    const asked = w.calls.map((call) => call.argv.join(" "));
    expect(asked).toHaveLength(CFG.needsRepos.length * 2);
    expect(
      asked.every((line) =>
        /^fleet gh api repos\/\S+\/issues\?state=open&labels=user%2F(review|escalated)&per_page=100&page=1$/.test(line),
      ),
    ).toBe(true);
    expect(asked.some((line) => line.includes("search/"))).toBe(false);
  });

  test("a repository the token cannot see is named, in the binary's own words for it; a failure that is not one repository's ends the read", async () => {
    const unseen = CFG.needsRepos[1];
    const w = world({
      commands: (argv) => (listed(argv)?.[1] === unseen ? { code: 1, stderr: "gh: Not Found (HTTP 404)" } : "[]"),
    });
    const c = await new Work(w.ops).censusDirect();
    expect([c.complete, c.unread]).toEqual([false, [unseen]]);
    const limited = world({
      commands: () => ({ code: 1, stderr: "gh: API rate limit exceeded for user ID 1. (HTTP 403)" }),
    });
    await expect(new Work(limited.ops).censusDirect()).rejects.toThrow(/rate limit exceeded/);
    expect(limited.calls).toHaveLength(1);
  });
});

describe("what needs the owner's hands", () => {
  test("the census is split into what only he can do, reviews, and the rest", async () => {
    const item = (n: number, labels: string[], hands?: unknown) => ({
      ref: `${REPO}#${n}`,
      kind: "issue",
      title: `t${n}`,
      url: `u${n}`,
      needsLabels: labels,
      reason: n === 3 ? "why" : null,
      updatedAt: "2026-11-20T01:02:03Z",
      hands,
    });
    const w = world({
      dash: (path) =>
        path === "/api/needs-drew.json"
          ? {
              fetchedAt: "x",
              complete: true,
              items: [
                item(1, ["user/escalated"], { kind: "physical", action: "Power on the host." }),
                item(2, ["user/review"]),
                item(3, ["user/escalated"]),
                item(4, ["user/escalated", "user/review"], { kind: "payment", action: null }),
              ],
            }
          : [
              {
                tier: "in-motion",
                sessionId: "s",
                title: "moving",
                agent: "build",
                model: "m",
                directory: "/home/user/git/code-j",
                lastActivityAt: "2026-11-22T00:00:00Z",
                activeToolCalls: 2,
                messageCount: 9,
              },
              { tier: "needs-drew", id: "p", title: "log in", ref: "gh:x/y#1", drewReason: null },
              { tier: "needs-orchestrator", title: "not his" },
            ],
    });
    const work = new Work(w.ops);
    const c = await work.census();
    expect(c.hands.map((i) => [i.ref, i.kind, i.action])).toEqual([
      [`${REPO}#1`, "physical", "Power on the host."],
      [`${REPO}#4`, "payment", ""],
    ]);
    expect(c.reviews.map((i) => i.ref)).toEqual([`${REPO}#2`]);
    expect(c.other.map((i) => [i.ref, i.why])).toEqual([[`${REPO}#3`, "why"]]);
    // When each was last touched is a time the app can read (it was once shortened like a
    // title, with a mark at the cut, and every card then said "updated" and nothing after).
    expect(c.reviews[0].updated).toBe("2026-11-20T01:02:03");
    expect(Date.parse(`${c.reviews[0].updated}Z`)).toBe(Date.parse("2026-11-20T01:02:03Z"));
    expect([c.at, c.complete, "unread" in c]).toEqual(["x", true, false]);
    // A census the dashboard could not finish is not passed on as whole; one that does not say is.
    const partly = (complete?: boolean) =>
      new Work(
        world({ dash: () => ({ fetchedAt: "x", items: [], ...(complete === undefined ? {} : { complete }) }) }).ops,
      )
        .census()
        .then((census) => census.complete);
    expect([await partly(false), await partly(true), await partly()]).toEqual([false, true, true]);
    const s = await work.sessions();
    expect(s.moving).toEqual([
      {
        id: "s",
        title: "moving",
        agent: "build",
        model: "m",
        where: "git/code-j",
        last: "2026-11-22T00:00:00Z",
        tools: 2,
        messages: 9,
      },
    ]);
    expect(s.waiting).toEqual([{ id: "p", title: "log in", ref: "gh:x/y#1", why: "" }]);
  });

  test("done means the label gone and a comment, only for a repository on the list", async () => {
    const w = world({});
    const work = new Work(w.ops);
    await work.handsDone({ ref: "dezren39/darwin-nix#12", note: " the BMC is on " });
    // The label first: see the next test for why.
    expect(w.gh()).toEqual([
      ["-X", "DELETE", "repos/dezren39/darwin-nix/issues/12/labels/user%2Fescalated"],
      ["-X", "POST", "repos/dezren39/darwin-nix/issues/12/comments", "--input", "-"],
    ]);
    expect(JSON.parse(w.calls[1].stdin ?? "").body).toContain("the BMC is on");
    await work.needComment({ ref: `${REPO}#5`, text: "go on" });
    expect(w.gh()[2]).toEqual(["-X", "POST", `repos/${REPO}/issues/5/comments`, "--input", "-"]);
    const before = w.calls.length;
    for (const ref of ["someone/else#1", `${REPO}#0`, `${REPO}#1/labels`, `${REPO}`, "", 7, `../${REPO}#1`]) {
      await expect(work.handsDone({ ref })).rejects.toThrow(OpError);
    }
    await expect(work.needComment({ ref: `${REPO}#5`, text: " " })).rejects.toThrow(OpError);
    expect(w.calls.length).toBe(before);
  });

  // "Done" is two things asked of GitHub, and either may fail. Taking a
  // label off twice is the same as once: GitHub says it is not there,
  // which is what was wanted. Saying "done" twice is two comments. So
  // the label goes first, and a failure part-way is tried again from
  // the start without anything being said twice. The oracle for "not
  // there" is GitHub's own answer to removing a label an issue does not
  // carry: 404, "Label does not exist".
  test("done: the label is taken off first, so that trying again after a failure says it once", async () => {
    const comments: string[] = [];
    let labelled = true;
    let commentsFail = 1;
    const w = world({
      commands: (argv, stdin) => {
        if (argv.includes("DELETE")) {
          if (!labelled) return { code: 1, stderr: "gh: Label does not exist (HTTP 404)" };
          labelled = false;
          return "[]";
        }
        if (argv.includes("POST")) {
          if (commentsFail-- > 0) return { code: 1, stderr: "gh: Server Error (HTTP 500)" };
          comments.push(JSON.parse(stdin ?? "{}").body);
          return '{"html_url":"https://github.com/x"}';
        }
        return "{}";
      },
    });
    const work = new Work(w.ops);
    // The comment fails: the label is off, and nothing was said.
    await expect(work.handsDone({ ref: `${REPO}#7`, note: "plugged in" })).rejects.toThrow(/Server Error/);
    expect([labelled, comments]).toEqual([false, []]);
    // Tried again: the label is not there to take off, which is fine, and it is said once.
    expect(await work.handsDone({ ref: `${REPO}#7`, note: "plugged in" })).toEqual({ ref: `${REPO}#7`, done: true });
    expect(comments).toHaveLength(1);
    expect(comments[0]).toContain("plugged in");
  });

  test("done: a label an agent had already taken off does not stop the fleet being told", async () => {
    const said: string[] = [];
    const w = world({
      commands: (argv, stdin) => {
        if (argv.includes("DELETE")) return { code: 1, stderr: '{"message":"Label does not exist","status":"404"}' };
        if (argv.includes("POST")) said.push(JSON.parse(stdin ?? "{}").body);
        return "{}";
      },
    });
    expect(await new Work(w.ops).handsDone({ ref: `${REPO}#7` })).toEqual({ ref: `${REPO}#7`, done: true });
    expect(said).toHaveLength(1);
  });

  test("done: any other failure to take the label off stops it there, with nothing said", async () => {
    for (const stderr of [
      "gh: Not Found (HTTP 404)",
      "gh: Resource not accessible by personal access token (HTTP 403)",
      "gh: API rate limit exceeded (HTTP 403)",
      "gh: Server Error (HTTP 500)",
    ]) {
      const said: string[] = [];
      const w = world({
        commands: (argv, stdin) => {
          if (argv.includes("DELETE")) return { code: 1, stderr };
          if (argv.includes("POST")) said.push(JSON.parse(stdin ?? "{}").body);
          return "{}";
        },
      });
      await expect(new Work(w.ops).handsDone({ ref: `${REPO}#7` })).rejects.toThrow(OpError);
      expect([stderr, said]).toEqual([stderr, []]);
    }
  });
});

describe("what the owner asks for in his own words", () => {
  test("a prompt becomes a labelled issue and a top-band task in one pass", async () => {
    const w = world({
      commands: (argv) =>
        argv.includes(`repos/${REPO}/issues`) ? JSON.stringify({ number: 12501, html_url: "https://x/12501" }) : "{}",
    });
    const work = new Work(w.ops, undefined, () => new Date("2026-11-22T06:00:00Z"));
    expect(await work.ask({ text: "Make the join slice the top priority\nand tell me what is in its way." })).toEqual({
      issue: 12501,
      url: "https://x/12501",
      queued: true,
    });
    expect(w.gh()).toEqual([
      ["-X", "POST", `repos/${REPO}/labels`, "--input", "-"],
      ["-X", "POST", `repos/${REPO}/issues`, "--input", "-"],
    ]);
    const issue = JSON.parse(w.calls[1].stdin ?? "");
    expect(issue.title).toBe("Owner: Make the join slice the top priority");
    expect(issue.labels).toEqual(["owner/prompt", "expedite"]);
    expect(issue.body.startsWith("Make the join slice the top priority\nand tell me")).toBe(true);
    expect(issue.body).toContain("<!-- owner-prompt:v1 -->");
    const enqueue = w.calls[2];
    expect(enqueue.argv.slice(0, 3)).toEqual(["fleet", "queue", "enqueue"]);
    expect(enqueue.argv.slice(4)).toEqual(["--queue", "repo-health-code-pilot"]);
    expect(enqueue.packet).toEqual(
      promptPacket(
        REPO,
        "repo-health-code-pilot",
        12501,
        "Make the join slice the top priority\nand tell me what is in its way.",
        "2026-11-22T06:00:00.000Z",
      ),
    );
    expect(enqueue.packet).toMatchObject({
      priority: 10,
      dedupe_key: `repo-health:v1:triage:${REPO}:issue:12501`,
      payload: { schema: "repo-task/v1", kind: "triage", issue: { number: 12501 } },
    });
    // The label is prepared once per process, and "already there" is success.
    await work.ask({ text: "second" });
    expect(w.gh().filter((a) => a[2] === `repos/${REPO}/labels`).length).toBe(1);
    const exists = world({
      commands: (argv) =>
        argv.includes(`repos/${REPO}/labels`)
          ? { code: 1, stderr: 'gh: Validation Failed (HTTP 422) {"code":"already_exists"}' }
          : JSON.stringify({ number: 1, html_url: "u" }),
    });
    expect((await new Work(exists.ops).ask({ text: "x" })).issue).toBe(1);
    const denied = world({
      commands: (argv) =>
        argv.includes(`repos/${REPO}/labels`) ? { code: 1, stderr: "gh: Forbidden (HTTP 403)" } : "{}",
    });
    await expect(new Work(denied.ops).ask({ text: "x" })).rejects.toThrow(/label/);
    expect(denied.gh().length).toBe(1); // no issue is filed if the label cannot be prepared
    // The issue exists either way: a failed enqueue is said, not thrown.
    const stuck = world({
      commands: (argv) =>
        argv[1] === "queue"
          ? { code: 1, stderr: "repo-queue: broker unreachable" }
          : JSON.stringify({ number: 2, html_url: "u" }),
    });
    expect(await new Work(stuck.ops).ask({ text: "x" })).toEqual({
      issue: 2,
      url: "u",
      queued: false,
      note: "filed, not queued: repo-queue: broker unreachable",
    });
    for (const text of ["", "  ", 5, "x".repeat(8001)]) await expect(work.ask({ text })).rejects.toThrow(OpError);
  });

  test("a reply wakes the fleet again, and only on one of his prompts", async () => {
    const issue = (state: string, labels: string[]) =>
      JSON.stringify({ state, labels: labels.map((name) => ({ name })) });
    const w = world({
      commands: (argv) =>
        argv.includes("--input") && argv[5].endsWith("/comments")
          ? JSON.stringify({ id: 777 })
          : argv[3] === `repos/${REPO}/issues/12501`
            ? issue("closed", ["owner/prompt"])
            : "{}",
    });
    const work = new Work(w.ops, undefined, () => new Date("2026-11-22T06:00:00Z"));
    expect(await work.askReply({ issue: 12501, text: "also do the docs" })).toEqual({ issue: 12501, queued: true });
    expect(w.gh()).toEqual([
      [`repos/${REPO}/issues/12501`],
      ["-X", "POST", `repos/${REPO}/issues/12501/comments`, "--input", "-"],
      ["-X", "PATCH", `repos/${REPO}/issues/12501`, "--input", "-"], // it was closed: reopened
    ]);
    expect(JSON.parse(w.calls[1].stdin ?? "").body).toBe("also do the docs\n\n<!-- owner-reply:v1 -->");
    expect(w.calls[3].packet).toMatchObject({
      priority: 10,
      dedupe_key: `repo-health:v1:triage:${REPO}:issue:12501`,
    });
    const other = world({ commands: () => issue("open", ["bug"]) });
    await expect(new Work(other.ops).askReply({ issue: 5, text: "x" })).rejects.toThrow(
      /not one of the owner's prompts/,
    );
    await expect(new Work(other.ops).askClose({ issue: 5 })).rejects.toThrow(/not one of the owner's prompts/);
    expect(other.calls.length).toBe(2);
    await expect(work.askReply({ issue: "12501/comments", text: "x" })).rejects.toThrow(OpError);
  });

  test("a custom enqueue hook takes the queue task instead of the laptop's command", async () => {
    const w = world({
      commands: (argv) =>
        argv.includes(`repos/${REPO}/issues`) ? JSON.stringify({ number: 12501, html_url: "https://x/12501" }) : "{}",
    });
    const work = new Work(w.ops, undefined, () => new Date("2026-11-22T06:00:00Z"));
    const held: unknown[] = [];
    const hook = async (packet: unknown) => {
      held.push(packet);
      return { code: 0, stdout: "", stderr: "" };
    };
    // A new prompt: the issue is filed at once, the task goes to the hook.
    expect(await work.ask({ text: "Do the docs" }, hook)).toEqual({
      issue: 12501,
      url: "https://x/12501",
      queued: true,
    });
    expect(held).toHaveLength(1);
    expect(held[0]).toEqual(
      promptPacket(REPO, "repo-health-code-pilot", 12501, "Do the docs", "2026-11-22T06:00:00.000Z"),
    );
    expect(w.calls.some((c) => c.argv[1] === "queue")).toBe(false);
    // A reply: the comment is posted at once, the task goes to the hook.
    const issue = (state: string, labels: string[]) =>
      JSON.stringify({ state, labels: labels.map((name) => ({ name })) });
    const r = world({
      commands: (argv) =>
        argv[3] === `repos/${REPO}/issues/12501` ? issue("open", ["owner/prompt"]) : JSON.stringify({ id: 777 }),
    });
    const replyWork = new Work(r.ops, undefined, () => new Date("2026-11-22T06:00:00Z"));
    expect(await replyWork.askReply({ issue: 12501, text: "also do the docs" }, hook)).toEqual({
      issue: 12501,
      queued: true,
    });
    expect(held).toHaveLength(2);
    expect(held[1]).toMatchObject({
      priority: 10,
      dedupe_key: `repo-health:v1:triage:${REPO}:issue:12501`,
    });
    expect(r.calls.some((c) => c.argv[1] === "queue")).toBe(false);
  });

  test("each prompt shows where it stands", async () => {
    const found = (n: number, state: string, comments: number) => ({
      number: n,
      title: `Owner: t${n}`,
      body: `words ${n}\n\n---\nnote`,
      html_url: `u${n}`,
      state,
      created_at: "2026-11-22T01:00:00Z",
      updated_at: `2026-11-22T0${n}:00:00Z`,
      comments,
    });
    let commentReads = 0;
    const w = world({
      commands: (argv) => {
        // The issue list (not the search index, which lags), newest change first.
        if (argv[3] === `repos/${REPO}/issues?labels=owner%2Fprompt&state=all&sort=updated&direction=desc&per_page=20`)
          return JSON.stringify([
            found(1, "closed", 1),
            found(2, "open", 1),
            found(3, "open", 0),
            { ...found(99, "open", 0), pull_request: {} }, // a labelled pull request is not a prompt
            found(4, "open", 2),
            found(5, "open", 0),
          ]);
        commentReads++;
        const n = Number(/issues\/(\d+)\/comments/.exec(argv[3])?.[1]);
        return JSON.stringify(
          n === 4
            ? [
                { body: "fleet says", created_at: "2026-11-22T02:00:00Z" },
                { body: "and I say\n\n<!-- owner-reply:v1 -->", created_at: "2026-11-22T03:00:00Z" },
              ]
            : [{ body: "fleet says", created_at: "2026-11-22T02:00:00Z" }],
        );
      },
      rows: () => [
        { dedupe_key: `repo-health:v1:triage:${REPO}:issue:3`, status: "claimed", worker: "w1" },
        { dedupe_key: `repo-health:v1:triage:${REPO}:issue:4`, status: "queued", worker: null },
        { dedupe_key: `repo-health:v1:triage:${REPO}:issue:4`, status: "done", worker: null },
      ],
    });
    const work = new Work(w.ops);
    const asks = await work.asks();
    expect(asks.map((a) => [a.issue, a.status, a.worker, a.text, a.replies.map((r) => r.you)])).toEqual([
      [1, "done", null, "words 1", [false]],
      [2, "answered", null, "words 2", [false]],
      [3, "working", "w1", "words 3", []],
      [4, "queued", null, "words 4", [false, true]], // the last word is his: back in the queue
      [5, "waiting", null, "words 5", []],
    ]);
    expect(asks[3].replies[1].text).toBe("and I say");
    // A long thread is read from its last page backwards until six replies
    // are in hand, and a bot that only links the issue somewhere else is
    // not the fleet answering.
    const real = (n: number) => ({ body: `reply ${n}`, created_at: "2026-11-22T02:00:00Z", user: { type: "User" } });
    const bot = {
      body: "[CODE-1](https://kaneo.example/task/1)",
      created_at: "2026-11-22T02:01:00Z",
      user: { type: "Bot" },
    };
    const pages: Record<string, unknown[]> = {
      // 250 comments: the last page holds one answer and a bot's link, the page before it the rest.
      "7:3": [real(201), bot],
      "7:2": [real(101), real(102), real(103), real(104), real(105), real(106), real(107)],
      "7:1": [real(1)],
      // 101 comments: the answer is the last comment of page 1, and page 2 is only the bot.
      "8:2": [bot],
      "8:1": [real(99), real(100)],
    };
    const long = world({
      commands: (argv) => {
        if (argv[3].includes("/issues?labels=")) return JSON.stringify([found(7, "open", 250), found(8, "open", 101)]);
        const m = /issues\/(\d+)\/comments\?per_page=100&page=(\d+)$/.exec(argv[3]);
        return JSON.stringify(m ? (pages[`${m[1]}:${m[2]}`] ?? []) : []);
      },
    });
    const [deep, boundary] = await new Work(long.ops).asks();
    expect(long.gh().slice(1)).toEqual([
      [`repos/${REPO}/issues/7/comments?per_page=100&page=3`],
      [`repos/${REPO}/issues/7/comments?per_page=100&page=2`],
      [`repos/${REPO}/issues/8/comments?per_page=100&page=2`],
      [`repos/${REPO}/issues/8/comments?per_page=100&page=1`],
    ]);
    // Six replies, oldest first, across the page boundary; the bot's link is not one of them.
    expect(deep.replies.map((r) => r.text)).toEqual([
      "reply 103",
      "reply 104",
      "reply 105",
      "reply 106",
      "reply 107",
      "reply 201",
    ]);
    expect(boundary.replies.map((r) => r.text)).toEqual(["reply 99", "reply 100"]);
    expect([deep.status, boundary.status]).toEqual(["answered", "answered"]);
    // Threads are read again only when the issue has changed.
    await work.asks();
    expect(commentReads).toBe(3);
    // A holder that was restarted and kept its last list (the edge)
    // recalls it, and reads no thread that has not changed since.
    const restarted = new Work(w.ops);
    restarted.recall(JSON.parse(JSON.stringify(asks)));
    expect((await restarted.asks()).map((a) => a.replies)).toEqual(asks.map((a) => a.replies));
    expect(commentReads).toBe(3);
    // One whose issue changed since the list was kept is read again, alone.
    const older = JSON.parse(JSON.stringify(asks)) as { issue: number; updated: string }[];
    (older.find((a) => a.issue === 4) as { updated: string }).updated = "2026-11-22T03:59:59";
    const behind = new Work(w.ops);
    behind.recall(older);
    await behind.asks();
    expect(commentReads).toBe(4);
    // What was read since is not unlearned by recalling an older list.
    const knows = new Work(w.ops);
    await knows.asks();
    const read = commentReads;
    knows.recall(older);
    await knows.asks();
    expect(commentReads).toBe(read);
    // What is not such a list teaches nothing: every thread is read, as from new.
    for (const junk of [null, "asks", { issue: 1 }, [null, 5, { issue: "1", updated: "x", replies: [] }]]) {
      const fresh = new Work(w.ops);
      fresh.recall(junk);
      const before = commentReads;
      await fresh.asks();
      expect(commentReads - before).toBe(3);
    }
    // Nor does an entry whose replies are not replies: its thread is read.
    const bent = JSON.parse(JSON.stringify(asks)) as { issue: number; replies: unknown[] }[];
    (bent.find((a) => a.issue === 2) as { replies: unknown[] }).replies = [{ you: "no", at: 1, text: null }];
    const wary = new Work(w.ops);
    wary.recall(bent);
    const before = commentReads;
    await wary.asks();
    expect(commentReads - before).toBe(1);
  });
});

describe("the state kept ready", () => {
  const source = () => {
    let n = 0;
    let fail = false;
    let gate: Promise<void> | null = null;
    return {
      load: async () => {
        n++;
        await gate;
        if (fail) throw new Error("source is down");
        return { n };
      },
      reads: () => n,
      break: (b: boolean) => {
        fail = b;
      },
      hold: () => {
        let open = () => {};
        gate = new Promise<void>((r) => {
          open = r;
        });
        return () => {
          gate = null;
          open();
        };
      },
    };
  };
  const all = (board: { ttlMs: number; load: () => Promise<unknown> }) => {
    const still = { ttlMs: 1e12, load: async () => "x" };
    return {
      board,
      sessions: still,
      census: still,
      asks: still,
      questionCount: still,
      questions: still,
      prs: still,
      priorities: still,
      done: still,
      agents: still,
      services: still,
      more: still,
    };
  };

  test("a page never waits on a source that has been read before", async () => {
    let now = 1_000_000;
    const s = source();
    const docs = new Docs(all({ ttlMs: 10_000, load: s.load }), () => now);
    const first = JSON.parse((await docs.doc("core")).body);
    expect(first.board).toEqual({ n: 1 }); // the first read is waited for
    expect(first.health.board).toEqual({ ok: true, error: null });
    now += 5_000;
    await docs.doc("core");
    expect(s.reads()).toBe(1); // fresh: not read again
    now += 6_000;
    const release = s.hold();
    const stale = JSON.parse((await docs.doc("core")).body); // answers at once with what it has
    expect(stale.board).toEqual({ n: 1 });
    expect(s.reads()).toBe(2); // and a read is under way
    await docs.doc("core");
    expect(s.reads()).toBe(2); // one read at a time
    release();
    await docs.refresh("board");
    expect(JSON.parse((await docs.doc("core")).body).board).toEqual({ n: 2 });
  });

  test("a failed read keeps the last good copy and says so", async () => {
    let now = 1_000_000;
    const s = source();
    const docs = new Docs(all({ ttlMs: 10_000, load: s.load }), () => now);
    const good = await docs.doc("core");
    s.break(true);
    now += 11_000;
    await docs.refresh("board");
    const after = await docs.doc("core");
    const body = JSON.parse(after.body);
    expect(body.board).toEqual({ n: 1 });
    expect(body.health.board).toEqual({ ok: false, error: "source is down" });
    expect(after.etag).not.toBe(good.etag);
    expect(after.asOf).toBe(now - 1_000); // the board is a second past its interval
    // Two times travel with it: when the board was last tried, which the failed try moved
    // on, and when what is shown of it was read, which it did not.
    const wasRead = new Date(now - 11_000).toISOString();
    expect([body.read.board, body.good.board]).toEqual([new Date(now).toISOString(), wasRead]);
    expect(JSON.parse(good.body).good.board).toBe(wasRead);
    // Tried again and failing again an hour on, the copy is as old as it is, not as the try.
    now += 3_600_000;
    await docs.refresh("board");
    const later = JSON.parse((await docs.doc("core")).body);
    expect([later.read.board, later.good.board]).toEqual([new Date(now).toISOString(), wasRead]);
    // Read again, both are now.
    s.break(false);
    now += 11_000;
    await docs.refresh("board");
    const mended = JSON.parse((await docs.doc("core")).body);
    expect([mended.read.board, mended.good.board]).toEqual([new Date(now).toISOString(), new Date(now).toISOString()]);
    // A section never read has neither.
    const never = new Docs(all({ ttlMs: 10_000, load: async () => new Promise(() => {}) }), () => now);
    expect(never.goodAt("board")).toBe(0);
  });

  test("a copy kept from before is shown at once and read again behind the same answer", async () => {
    let now = 1_000_000;
    let reads = 0;
    let fail = false;
    // A loader that can read its source, and that starts from a copy it
    // kept five minutes ago (the edge, after a restart).
    const load = async () => {
      reads++;
      if (reads === 1) return new Stamped({ kept: true }, now - 300_000, null, true);
      if (fail) throw new Error("source is down");
      return { fresh: reads };
    };
    const docs = new Docs(all({ ttlMs: 10_000, load }), () => now);
    expect(docs.tried("board")).toBe(false);
    const first = await docs.doc("core");
    // The kept copy, said to be as old as it is ...
    expect(JSON.parse(first.body).board).toEqual({ kept: true });
    expect(first.asOf).toBe(now - 300_000 + 10_000);
    expect(JSON.parse(first.body).read.board).toBe(new Date(now - 300_000).toISOString());
    // ... and already being read again, with nobody having asked twice.
    expect(reads).toBe(2);
    await docs.settled(["board"]);
    const second = await docs.doc("core");
    expect(JSON.parse(second.body).board).toEqual({ fresh: 2 });
    expect(second.asOf).toBe(now);
    expect(reads).toBe(2);
    expect(docs.tried("board")).toBe(true);
    // settled() with nothing under way answers at once, and for every section when none is named.
    await docs.settled();

    // A copy that somewhere else read (no way to read it here) is shown
    // the same way, and is not asked for again until its interval is up.
    let mirrored = 0;
    const far = new Docs(
      all({ ttlMs: 10_000, load: async () => (mirrored++, new Stamped({ theirs: true }, now - 300_000)) }),
      () => now,
    );
    await far.doc("core");
    await far.settled();
    expect(mirrored).toBe(1);
    now += 11_000;
    await far.doc("core");
    await far.settled();
    expect(mirrored).toBe(2);

    // A kept copy whose first read behind it fails stays, with the reason.
    reads = 0;
    fail = true;
    const down = new Docs(all({ ttlMs: 10_000, load }), () => now);
    await down.doc("core");
    await down.settled(["board"]);
    const body = JSON.parse((await down.doc("core")).body);
    expect(body.board).toEqual({ kept: true });
    expect(body.health.board).toEqual({ ok: false, error: "source is down" });
    // A source never read with success has been tried all the same.
    const never = new Docs(
      all({
        ttlMs: 10_000,
        load: async () => {
          throw new Error("nothing yet");
        },
      }),
      () => now,
    );
    await never.refresh("board");
    expect([never.tried("board"), never.goodAt("board")]).toEqual([true, 0]);
  });

  test("after a change, a read that was already under way is not taken as showing it", async () => {
    let now = 1_000_000;
    let value = "first";
    let reads = 0;
    let gate: Promise<void> | null = null;
    let open = () => {};
    const hold = () => {
      gate = new Promise<void>((r) => {
        open = r;
      });
    };
    const release = () => {
      gate = null;
      open();
    };
    // What the source held when the read began is what the read gives.
    const load = async () => {
      reads++;
      const seen = value;
      await gate;
      return { seen };
    };
    const docs = new Docs(all({ ttlMs: 10_000, load }), () => now);
    const board = async () => JSON.parse((await docs.doc("core")).body).board;
    await docs.doc("core");
    // Reading now shares a read under way, whatever it saw.
    now += 11_000;
    hold();
    await docs.doc("core"); // a read starts behind the answer, and has seen "first"
    expect(reads).toBe(2);
    value = "second";
    const shared = docs.touch(["board"]);
    release();
    await shared;
    expect(reads).toBe(2);
    expect(await board()).toEqual({ seen: "first" });
    // Reading after a change waits for the read under way, which may
    // have been taken before the change, and then reads once more.
    now += 11_000;
    hold();
    await docs.doc("core"); // this one has seen "second"
    expect(reads).toBe(3);
    value = "third"; // something is done
    const reread = docs.reread(["board"]);
    release();
    await reread;
    expect(reads).toBe(4);
    expect(await board()).toEqual({ seen: "third" });
    // With no read under way it is one read, and a section there is no
    // such thing as is passed over.
    await docs.reread(["board", "nonesuch"]);
    expect(reads).toBe(5);
  });

  test("what an action changed is on the next page, though a read was under way while it ran", async () => {
    let now = 1_000_000;
    let value = "before";
    let gate: Promise<void> | null = null;
    let open = () => {};
    const load = async () => {
      const seen = value;
      await gate;
      return { seen };
    };
    const docs = new Docs(all({ ttlMs: 10_000, load }), () => now);
    await docs.doc("core");
    now += 11_000;
    gate = new Promise<void>((r) => {
      open = r;
    });
    await docs.doc("core"); // the board is being read, and has seen "before"
    const table = {
      "thing.do": {
        run: async () => {
          value = "after";
          return { done: true };
        },
        touch: ["board"],
        audit: () => ({}),
      },
    };
    const doing = perform(table, docs, "thing.do", {});
    gate = null;
    open();
    expect(await doing).toEqual({ done: true });
    expect(JSON.parse((await docs.doc("core")).body).board).toEqual({ seen: "after" });
  });

  test("an unchanged document keeps its tag, and nothing is read when nobody is looking", async () => {
    let now = 1_000_000;
    let reads = 0;
    const docs = new Docs(all({ ttlMs: 10_000, load: async () => (reads++, { same: true }) }), () => now);
    const a = await docs.doc("core");
    now += 11_000;
    await docs.refresh("board");
    const b = await docs.doc("core");
    expect(b.etag).toBe(a.etag);
    expect(b.asOf).toBeGreaterThan(a.asOf);
    expect(reads).toBe(2);
    now += 11_000;
    docs.tick(); // someone looked 11 seconds ago: refresh
    await docs.refresh("board");
    expect(reads).toBe(3);
    now += 11 * 60_000;
    docs.tick(); // nobody has looked for eleven minutes: leave the sources alone
    await docs.refresh("more");
    expect(reads).toBe(3);
    expect(() => new Docs({ board: { ttlMs: 1, load: async () => 1 } })).toThrow(/no loader/);
  });
});

describe("the interface's endpoints", () => {
  function deps(who: "owner" | 401 = "owner") {
    const w = world({
      rows: (st) => (st.sql.startsWith("UPDATE") ? { affected: 1 } : [{ priority: 30 }]),
      dash: () => [],
    });
    const work = new Work(w.ops, async () => store());
    const docs = new Docs(loaders(w.ops, work, [{ group: "Overview", pages: [] }]));
    const audited: Record<string, unknown>[] = [];
    const d: Deps = {
      cfg: CFG,
      ops: w.ops,
      docs,
      actions: actions(w.ops, work),
      identify: async () => {
        if (who === "owner") return { via: "cloudflare-access", email: "owner@example.com" };
        throw new (await import("../src/auth.ts")).AuthError(401, "sign-in required");
      },
      audit: async (e) => {
        audited.push(e);
      },
    };
    return { d, w, audited, docs };
  }
  const get = (path: string, headers: Record<string, string> = {}) =>
    new Request(`https://pm.security.cab${path}`, { headers: { host: "pm.security.cab", ...headers } });
  const post = (path: string, body: unknown) =>
    new Request(`https://pm.security.cab${path}`, {
      method: "POST",
      body: JSON.stringify(body),
      headers: {
        host: "pm.security.cab",
        "content-type": "application/json",
        "x-pm-console": "1",
        origin: "https://pm.security.cab",
      },
    });

  test("a document comes with a tag, is not sent twice, and is compressed", async () => {
    const { d } = deps();
    const res = await handle(get("/console/api/doc/questions"), d);
    expect(res.status).toBe(200);
    const body = (await res.json()) as { questions: { total: number } };
    expect(body.questions.total).toBe(4);
    expect(JSON.stringify(body)).not.toContain(TOKEN);
    const etag = res.headers.get("etag") ?? "";
    expect(etag.length).toBeGreaterThan(10);
    expect(Number(res.headers.get("x-as-of"))).toBeGreaterThan(0);
    const again = await handle(get("/console/api/doc/questions", { "if-none-match": etag }), d);
    expect([again.status, await again.text()]).toEqual([304, ""]);
    expect(Number(again.headers.get("x-as-of"))).toBeGreaterThan(0);
    const packed = await handle(get("/console/api/doc/questions", { "accept-encoding": "gzip, br" }), d);
    expect(packed.headers.get("content-encoding")).toBe("gzip");
    const raw = new Uint8Array(await packed.arrayBuffer());
    expect(JSON.parse(gunzipSync(raw).toString()).questions.total).toBe(4);
    expect(raw.byteLength).toBeLessThan(JSON.stringify(body).length / 2);
    expect((await handle(get("/console/api/doc/secrets"), d)).status).toBe(404);
    expect((await handle(get("/console/api/doc/core"), deps(401).d)).status).toBe(401);
  });

  test("a machine serves what was finished as a document of its own, read from the queue store and GitHub", async () => {
    const { d, w } = deps();
    const res = await handle(get("/console/api/doc/done"), d);
    expect(res.status).toBe(200);
    const body = (await res.json()) as {
      done: { tasks: unknown; merged: unknown; closed: unknown };
      health: Record<string, { ok: boolean; error: string | null }>;
    };
    expect(body.health).toEqual({ done: { ok: true, error: null } });
    // GitHub's stand-in lists nothing closed.
    expect([body.done.merged, body.done.closed]).toEqual([
      { day: 0, more: false, latest: [] },
      { day: 0, more: false, latest: [] },
    ]);
    expect(w.sqls.flat().map((st) => st.sql)).toEqual([DONE_TASKS_SQL, DONE_COUNT_SQL]);
    expect(w.gh()).toEqual([[`repos/${REPO}/issues?state=closed&sort=updated&direction=desc&per_page=100`]]);
  });

  test("with no edge in front of it, a host does an action once under the name a device gave it", async () => {
    const { d, w } = deps();
    d.named = new Named(memoryStore());
    const id = "0e000000-0000-4000-8000-000000000001";
    const moves = () => w.sqls.flat().filter((s) => s.sql.startsWith("UPDATE tasks SET priority")).length;
    const first = await handle(
      post("/console/api/act", { kind: "task.move", args: { task: 21016, to: "first" }, id }),
      d,
    );
    expect(await first.json()).toEqual({ ok: true, result: { task: 21016, priority: 10 } });
    const second = await handle(
      post("/console/api/act", { kind: "task.move", args: { task: 21016, to: "first" }, id }),
      d,
    );
    expect(await second.json()).toEqual({ ok: true, result: { task: 21016, priority: 10 }, again: true });
    expect(moves()).toBe(1);
    // Its record of what was done carries the name, once.
    const core = (await (await handle(get("/console/api/doc/core"), d)).json()) as {
      acts: { id: string; status: string }[];
    };
    expect(core.acts.map((a: { id: string; status: string }) => [a.id, a.status])).toEqual([[id, "done"]]);
    // With nowhere to write names down, an action is done as often as it is asked: as it always was.
    const bare = deps();
    await handle(post("/console/api/act", { kind: "task.move", args: { task: 21016, to: "first" }, id }), bare.d);
    await handle(post("/console/api/act", { kind: "task.move", args: { task: 21016, to: "first" }, id }), bare.d);
    expect(bare.w.sqls.flat().filter((s) => s.sql.startsWith("UPDATE tasks SET priority")).length).toBe(2);
  });

  test("two arrivals under one name at the same moment: one does it, the other is told it is being done", async () => {
    const { d, w, audited } = deps();
    d.named = new Named(memoryStore());
    const moves = () => w.sqls.flat().filter((s) => s.sql.startsWith("UPDATE tasks SET priority")).length;
    const body = (id: string) => ({ kind: "task.move", args: { task: 21016, to: "first" }, id });
    // The record of what is done is slow to write, which is where two
    // arrivals used to pass each other: both had looked, neither had begun.
    let open: () => void = () => {};
    const gate = new Promise<void>((r) => {
      open = r;
    });
    const audit = d.audit;
    d.audit = async (e) => {
      await gate;
      return audit(e);
    };
    const id = "0e000000-0000-4000-8000-000000000002";
    const a = handle(post("/console/api/act", body(id)), d);
    const b = handle(post("/console/api/act", body(id)), d);
    await new Promise((r) => setTimeout(r, 5));
    open();
    const answers = await Promise.all([a, b]);
    expect(answers.map((r) => r.status).sort()).toEqual([200, 409]);
    const told = await (answers.find((r) => r.status === 409) as Response).json();
    expect(told).toEqual({ error: "This is still being done.", running: true });
    expect(moves()).toBe(1);
    expect(audited.filter((e) => e.phase === "start")).toHaveLength(1);
    // Asked again once it is over, it is answered from the record.
    expect(await (await handle(post("/console/api/act", body(id)), d)).json()).toMatchObject({ ok: true, again: true });
    // An action that could not be recorded is not taken, and its name
    // is not left taken either: tried again, it is simply done.
    const other = "0e000000-0000-4000-8000-000000000003";
    d.audit = async () => {
      throw new Error("disk full");
    };
    const refused = await handle(post("/console/api/act", body(other)), d);
    expect([refused.status, await refused.json()]).toEqual([
      502,
      { error: "the action could not be recorded, so it was not taken" },
    ]);
    d.audit = audit;
    const retried = await handle(post("/console/api/act", body(other)), d);
    expect(await retried.json()).toEqual({ ok: true, result: { task: 21016, priority: 10 } });
    expect(moves()).toBe(2);
  });

  test("the record of names: what each state means, how many are kept, and a record that cannot be read", async () => {
    const store = memoryStore();
    const named = new Named(store);
    expect(await named.seen("a")).toBeNull();
    await named.begin("a");
    expect(await named.seen("a")).toEqual({ state: "running" });
    await named.end("a", { ok: true, result: { n: 1 } });
    expect(await named.seen("a")).toEqual({ state: "done", fate: { ok: true, result: { n: 1 } } });
    await named.begin("b");
    await named.end("b", { ok: false, error: "no", status: 404 });
    expect(await named.seen("b")).toEqual({ state: "done", fate: { ok: false, error: "no", status: 404 } });
    // Started, and the process stopped: another one, reading the same
    // record, knows it was started and not how it ended.
    await named.begin("c");
    const after = new Named(store);
    expect(await after.seen("c")).toEqual({ state: "lost" });
    expect(await after.seen("a")).toEqual({ state: "done", fate: { ok: true, result: { n: 1 } } });
    expect([named.isRunning("c"), after.isRunning("c")]).toEqual([true, false]);
    // An outcome that cannot be written still frees the name in this process.
    const failing = new Named({
      read: async () => null,
      write: async (text) => {
        if (text.includes('"ok"')) throw new Error("disk full");
      },
    });
    await failing.begin("d");
    await expect(failing.end("d", { ok: true })).rejects.toThrow("disk full");
    expect(failing.isRunning("d")).toBe(false);
    // A name is taken by one at a time, and given back.
    const one = new Named(memoryStore());
    expect([one.take("t"), one.take("t"), one.isRunning("t")]).toEqual([true, false, true]);
    expect(await one.seen("t")).toEqual({ state: "running" });
    one.release("t");
    expect([one.isRunning("t"), await one.seen("t"), one.take("t")]).toEqual([false, null, true]);
    // Three hundred names are kept: the oldest is forgotten first, and
    // one seen again counts as new.
    const many = new Named(memoryStore());
    for (let i = 0; i < 300; i++) await many.record(`n${i}`, { ok: true });
    await many.record("n0", { ok: true });
    await many.record("n300", { ok: true });
    expect([await many.get("n1"), await many.get("n0"), await many.get("n2")]).toEqual([
      undefined,
      { ok: true },
      { ok: true },
    ]);
    // A record that cannot be read is an empty one, not a crash.
    const broken = new Named({ read: async () => "not json", write: async () => {} });
    expect(await broken.seen("x")).toBeNull();
  });

  test("the record of names on a slow disk: read once however many ask together, and no write lands over a later one", async () => {
    let content: string | null = JSON.stringify({ old: { ok: true } });
    let reads = 0;
    // Each write takes as long as it is told to.
    const delays: number[] = [];
    const disk = {
      read: async () => {
        reads++;
        await new Promise((r) => setTimeout(r, 5));
        return content;
      },
      write: async (text: string) => {
        await new Promise((r) => setTimeout(r, delays.shift() ?? 1));
        content = text;
      },
    };
    const named = new Named(disk);
    // Three ask at the same moment, before the record has been read.
    await Promise.all([named.begin("a"), named.begin("b"), named.record("c", { ok: true })]);
    expect(reads).toBe(1);
    // All three are in the record, with what was there before.
    expect(Object.keys(JSON.parse(content as string))).toEqual(["old", "a", "b", "c"]);
    expect([named.isRunning("a"), named.isRunning("b")]).toEqual([true, true]);
    // One write is slow, and another name is written down while it is
    // out: the slow one does not land afterwards, over the newer.
    delays.push(12, 1);
    const slow = named.record("d", { ok: true });
    await new Promise((r) => setTimeout(r, 3));
    const quick = named.record("e", { ok: true });
    await Promise.all([slow, quick]);
    expect(Object.keys(JSON.parse(content as string))).toEqual(["old", "a", "b", "c", "d", "e"]);
    // A process started now reads all of them back.
    const after = new Named(disk);
    expect([await after.seen("a"), await after.seen("c"), await after.seen("e")]).toEqual([
      { state: "lost" },
      { state: "done", fate: { ok: true } },
      { state: "done", fate: { ok: true } },
    ]);
    // A write that fails is its own caller's to hear of; the next one is written all the same.
    const flaky = { fail: true };
    const kept: string[] = [];
    const sometimes = new Named({
      read: async () => null,
      write: async (text) => {
        if (flaky.fail) throw new Error("disk full");
        kept.push(text);
      },
    });
    await expect(sometimes.record("x", { ok: true })).rejects.toThrow("disk full");
    flaky.fail = false;
    await sometimes.record("y", { ok: true });
    expect(Object.keys(JSON.parse(kept[kept.length - 1]))).toEqual(["x", "y"]);
  });

  test("asked which machines there are, a machine serving the app by itself answers with itself: nothing is in front of it, and nothing waits", async () => {
    const { d } = deps();
    d.self = { name: "nixos", version: "0123abcd4567" };
    await handle(get("/console/api/doc/core"), d);
    const answer = (await (await handle(get("/console/api/machines"), d)).json()) as {
      served: string;
      platform: unknown;
      machines: { name: string; quiet: boolean; version: string | null; actions: string[]; sections: string[] }[];
      held: unknown[];
    };
    expect([answer.served, answer.platform, answer.held]).toEqual(["machine", null, []]);
    expect(answer.machines).toHaveLength(1);
    const [self] = answer.machines;
    expect([self.name, self.quiet, self.version]).toEqual(["nixos", false, "0123abcd4567"]);
    // What it can do is what it has: its actions by kind, and every section it reads.
    expect(self.actions).toEqual(Object.keys(d.actions ?? {}).sort());
    expect(self.actions).toContain("task.cancel");
    expect(self.sections).toContain("questions");
    expect(self.sections).toEqual([...self.sections].sort());
    // With no name given it, it is still a machine.
    d.self = undefined;
    const unnamed = (await (await handle(get("/console/api/machines"), d)).json()) as typeof answer;
    expect([unnamed.machines[0].name, unnamed.machines[0].version]).toEqual(["this machine", null]);
  });

  test("an action runs, is recorded without the owner's words, and shows in the state", async () => {
    const { d, w, audited } = deps();
    // What an action changes is read again before the answer goes back,
    // so the next document already shows it.
    const boardReads = () => w.sqls.filter((batch) => batch.length === 4).length;
    await handle(get("/console/api/doc/core"), d);
    const readsBefore = boardReads();
    const res = await handle(post("/console/api/act", { kind: "task.move", args: { task: 21016, to: "first" } }), d);
    expect(await res.json()).toEqual({ ok: true, result: { task: 21016, priority: 10 } });
    expect(boardReads()).toBe(readsBefore + 1);
    expect(w.sqls.flat().some((s) => s.sql.startsWith("UPDATE tasks SET priority"))).toBe(true);
    await handle(
      post("/console/api/act", { kind: "need.comment", args: { ref: `${REPO}#5`, text: "private words" } }),
      d,
    );
    expect(audited.map((e) => [e.action, e.phase, e.args])).toEqual([
      ["task.move", "start", { task: 21016, to: "first" }],
      ["task.move", "done", { task: 21016, to: "first" }],
      ["need.comment", "start", { ref: `${REPO}#5` }],
      ["need.comment", "done", { ref: `${REPO}#5` }],
    ]);
    expect(JSON.stringify(audited)).not.toContain("private words");
    const core = (await (await handle(get("/console/api/doc/core"), d)).json()) as {
      acts: { kind: string; status: string }[];
    };
    expect(core.acts.map((a: { kind: string; status: string }) => [a.kind, a.status])).toEqual([
      ["need.comment", "done"],
      ["task.move", "done"],
    ]);
    // Unknown actions, inherited names and bad arguments run nothing.
    const before = w.calls.length;
    for (const body of [
      { kind: "task.delete", args: {} },
      { kind: "constructor", args: {} },
      { kind: "toString", args: {} },
      { args: {} },
      { kind: "task.move", args: [] },
      { kind: "task.move", args: { task: 1, to: "up" } },
    ]) {
      expect((await handle(post("/console/api/act", body), d)).status).toBe(400);
    }
    expect(w.calls.length).toBe(before);
    const failed = (
      (await (await handle(get("/console/api/doc/core"), d)).json()) as { acts: { kind: string; status: string }[] }
    ).acts[0];
    expect([failed.kind, failed.status]).toEqual(["task.move", "failed"]);
    await expect(perform(actions(w.ops, new Work(w.ops)), deps().docs, "__proto__", {})).rejects.toThrow(
      /not an action/,
    );
  });

  test("the front door is the app, the dashboard's pages are compressed on the way", async () => {
    const { d } = deps();
    const big = "<p>row</p>".repeat(2000);
    const stream = "data: x\n\n".repeat(500);
    d.upstream = async (url) =>
      new Response(url.includes("/frag/") || url.endsWith("/freshness/stream") ? stream : big, {
        headers: {
          "content-type":
            url.includes("/frag/") || url.endsWith("/stream") ? "text/event-stream" : "text/html; charset=utf-8",
        },
      });
    const home = await handle(get("/"), d);
    expect([home.status, home.headers.get("location")]).toEqual([302, "/console/"]);
    expect((await handle(get("/"), deps(401).d)).status).toBe(401);
    // The dashboard's own home is still there, one link away.
    expect(await (await handle(get("/?dashboard"), d)).text()).toBe(big);
    const gz = { "accept-encoding": "gzip" };
    const page = await handle(get("/attention", gz), d);
    expect(page.headers.get("content-encoding")).toBe("gzip");
    expect(gunzipSync(new Uint8Array(await page.arrayBuffer())).toString()).toBe(big);
    const frag = await handle(get("/frag/attention/queues", gz), d);
    expect(frag.headers.get("content-encoding")).toBe("gzip");
    expect(frag.headers.get("content-type")).toBe("text/event-stream");
    expect(gunzipSync(new Uint8Array(await frag.arrayBuffer())).toString()).toBe(stream);
    // A stream that stays open is passed through as it comes.
    const live = await handle(get("/freshness/stream", gz), d);
    expect(live.headers.get("content-encoding")).toBeNull();
    expect(await live.text()).toBe(stream);
    // A client that did not ask for gzip gets the bytes as they are.
    expect(await (await handle(get("/attention"), d)).text()).toBe(big);
  });
});
