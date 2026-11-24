// Filing a task in the queue, as the fleet's broker files it.
//
// Oracles:
//   - the row a packet becomes: what the fleet binary itself made of the
//     same packets (`fleet queue enqueue --dry-run`, recorded in
//     test/fixtures/native-enqueue.json with the binary's version);
//   - the store: a real SQLite made from the broker's own CREATE TABLE
//     statements, read here from its source (turso_queue/core.rs) and
//     from the GitHub mirror's (pkgs/github-ci/worker/mirror.sql),
//     answering over the store's HTTP shape (integers as text);
//   - the columns written: the broker's INSERT and block UPDATE, read
//     from its source; and the broker's own recheck query, read from
//     its source, finds a task filed waiting and its release frees it;
//   - every expected value is written out by hand.
import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { readConfig } from "../src/config.ts";
import {
  BROKER_COLUMNS,
  blockerSaid,
  category,
  claimPriority,
  DEFAULT_OPTIONS,
  fileTask,
  filingOptions,
  NeedsBroker,
  optionArgs,
  ownerBlock,
  ownerLines,
  parseBlocker,
  queueTime,
  taskRow,
} from "../src/filing.ts";
import { type Fetch, Ops, type Run } from "../src/ops.ts";
import { triagePacket } from "../src/ops.ts";
import { promptPacket } from "../src/work.ts";
import { BROKER_SRC as BROKER, brokerDb, rustSql, storeAnswer } from "./broker-store.ts";
import { asThePlatformFetch } from "./support.ts";

const CFG = readConfig({ HOME: "/h" });
const REPO = CFG.repo;
const read = (path: string) => readFileSync(path, "utf8");
const NOW = new Date("2026-11-24T19:30:00.123Z");

const native = JSON.parse(read(join(import.meta.dir, "fixtures/native-enqueue.json"))) as {
  queue: string;
  cases: Record<string, { packet: Record<string, unknown>; native: Record<string, unknown> }>;
};

describe("the row a packet becomes is the fleet binary's", () => {
  // The packets, built now the way they were built when they were recorded.
  const at = "2026-11-24T19:30:00.000Z";
  const opts = (a: Record<string, unknown>) => filingOptions(a, REPO);
  const labels = (o: ReturnType<typeof opts>) => [
    CFG.promptLabel,
    `priority/${category(claimPriority(o.urgency, o.importance))}`,
  ];
  const d = opts({});
  const full = opts({
    urgency: "week",
    importance: "low",
    due: "2026-12-01",
    blockers: ["#14432", "PR #14422", "task 23198", "developing-today/opencode#12"],
    note: "Use the fleet's own words.",
    metadata: { area: "pm-stack", source: "phone" },
  });
  const today = opts({ urgency: "today", importance: "high" });
  const built: Record<string, unknown> = {
    "ask-default": promptPacket(
      REPO,
      CFG.queue,
      99001,
      "Make the queue say who filed each task\nwith more lines",
      at,
      "task",
      d,
      labels(d),
    ),
    "ask-question": promptPacket(REPO, CFG.queue, 99002, "Why are 13 pools walled?", at, "question", d, labels(d)),
    "ask-directive": promptPacket(
      REPO,
      CFG.queue,
      99003,
      "From now on, merge docs PRs without review",
      at,
      "directive",
      d,
      labels(d),
    ),
    "ask-today-high": promptPacket(
      REPO,
      CFG.queue,
      99004,
      "Look at the seating guard",
      at,
      "task",
      today,
      labels(today),
    ),
    "ask-everything": promptPacket(REPO, CFG.queue, 99005, "File it all", at, "task", full, labels(full)),
    reply: promptPacket(
      REPO,
      CFG.queue,
      99006,
      "And also the pools",
      at,
      "task",
      d,
      ["owner-prompt", "expedite"],
      true,
    ),
    "intake-triage": triagePacket(REPO, CFG.queue, 99007, "Bug: the chart is cut off", "now", at),
  };

  test("the packets are still the ones the binary was given (else record again)", () => {
    expect(native.queue).toBe(CFG.queue);
    expect(Object.keys(built).sort()).toEqual(Object.keys(native.cases).sort());
    for (const [name, packet] of Object.entries(built))
      expect([name, packet]).toEqual([name, native.cases[name].packet]);
  });

  test("each row is the binary's, field for field and in the payload's order", () => {
    for (const [name, c] of Object.entries(native.cases)) {
      const row = taskRow(c.packet, native.queue) as Record<string, unknown>;
      // The binary adds the platform after the row is made; this app files none.
      const { platform, ...theirs } = c.native;
      expect([name, platform]).toEqual([name, null]);
      expect([name, row]).toEqual([name, theirs]);
      expect([name, Object.keys(row.payload as object)]).toEqual([name, Object.keys(theirs.payload as object)]);
    }
    // What the binary chose, said plainly: the owner's choice where the labels say
    // nothing; the expedite band (15) where they do; the stated rank where none are given.
    const p = (name: string) => native.cases[name].native.priority;
    expect(
      [
        "ask-default",
        "ask-question",
        "ask-directive",
        "ask-today-high",
        "ask-everything",
        "reply",
        "intake-triage",
      ].map(p),
    ).toEqual([10, 10, 10, 30, 70, 15, 20]);
  });
});

describe("the owner's choices", () => {
  test("urgency and importance each move the task a step down the scale", () => {
    expect(claimPriority("now", "critical")).toBe(10);
    expect(claimPriority("now", "high")).toBe(20);
    expect(claimPriority("today", "critical")).toBe(20);
    expect(claimPriority("today", "high")).toBe(30);
    expect(claimPriority("week", "normal")).toBe(50);
    expect(claimPriority("later", "normal")).toBe(70);
    expect(claimPriority("later", "low")).toBe(90);
    expect([10, 15, 20, 25, 30, 40, 50, 70, 90].map(category)).toEqual([
      "critical",
      "high",
      "high",
      "high",
      "high",
      "normal",
      "normal",
      "low",
      "low",
    ]);
  });

  test("a blocker as the owner writes it", () => {
    expect(parseBlocker("#14432", REPO)).toEqual({ kind: "issue", repo: REPO, number: 14432 });
    expect(parseBlocker(" PR #14422 ", REPO)).toEqual({ kind: "pr", repo: REPO, number: 14422 });
    expect(parseBlocker("pull request #9", REPO)).toEqual({ kind: "pr", repo: REPO, number: 9 });
    expect(parseBlocker("task 23198", REPO)).toEqual({ kind: "task", task_id: 23198 });
    expect(parseBlocker("Task #5", REPO)).toEqual({ kind: "task", task_id: 5 });
    expect(parseBlocker("developing-today/opencode#12", REPO)).toEqual({
      kind: "issue",
      repo: "developing-today/opencode",
      number: 12,
    });
    expect(parseBlocker("https://github.com/developing-today/code/pull/14379", REPO)).toEqual({
      kind: "pr",
      repo: REPO,
      number: 14379,
    });
    expect(parseBlocker("https://github.com/dezren39/nix/issues/3#issuecomment-1", REPO)).toEqual({
      kind: "issue",
      repo: "dezren39/nix",
      number: 3,
    });
    for (const odd of [
      "",
      "14432",
      "#",
      "#abc",
      "task",
      "PR 12",
      "see #12",
      "https://example.com/a/b/issues/1",
      "#1234567890",
    ]) {
      expect([odd, parseBlocker(odd, REPO)]).toEqual([odd, null]);
    }
    expect(blockerSaid({ kind: "issue", repo: REPO, number: 3 }, REPO)).toBe("#3");
    expect(blockerSaid({ kind: "pr", repo: "a/b", number: 3 }, REPO)).toBe("pull request a/b#3");
    expect(blockerSaid({ kind: "task", task_id: 7 }, REPO)).toBe("task 7");
  });

  test("left out is now and critical and nothing else; anything given is checked and said back", () => {
    expect(filingOptions({}, REPO)).toEqual({ ...DEFAULT_OPTIONS, blockers: [], metadata: {} });
    const o = filingOptions(
      {
        urgency: "today",
        importance: "normal",
        due: "2026-12-01",
        blockers: ["#1", "#1", "task 2"],
        note: "  read the thread first  ",
        metadata: { area: "pm-stack", "x.y_z-1": "v" },
      },
      REPO,
    );
    // The same blocker twice is one; the note is trimmed.
    expect(o).toEqual({
      urgency: "today",
      importance: "normal",
      due: "2026-12-01",
      blockers: [
        { kind: "issue", repo: REPO, number: 1 },
        { kind: "task", task_id: 2 },
      ],
      note: "read the thread first",
      metadata: { area: "pm-stack", "x.y_z-1": "v" },
    });
    // Said back as arguments, then read again: the same.
    expect(optionArgs(o, REPO)).toEqual({
      urgency: "today",
      importance: "normal",
      due: "2026-12-01",
      blockers: ["#1", "task 2"],
      note: "read the thread first",
      metadata: { area: "pm-stack", "x.y_z-1": "v" },
    });
    expect(filingOptions(optionArgs(o, REPO), REPO)).toEqual(o);
    expect(optionArgs(filingOptions({}, REPO), REPO)).toEqual({});
    expect(optionArgs(filingOptions({ blockers: ["a/b#3", "PR #4"] }, REPO), REPO)).toEqual({
      blockers: ["a/b#3", "PR #4"],
    });
    // An empty due is no due.
    expect(filingOptions({ due: "" }, REPO).due).toBeNull();
    for (const [args, said] of [
      [{ urgency: "asap" }, "urgency: one of now, today, week, later"],
      [{ importance: "huge" }, "importance: one of critical, high, normal, low"],
      [{ due: "2026-02-30" }, "due: a day, as YYYY-MM-DD"],
      [{ due: "tomorrow" }, "due: a day, as YYYY-MM-DD"],
      [{ blockers: "#1" }, "blockers: a list"],
      [{ blockers: ["see #1"] }, 'blockers: "see #1" is not an issue, pull request or task (#123, PR #45, task 6789)'],
      [{ blockers: Array.from({ length: 9 }, (_, i) => `#${i + 1}`) }, "blockers: at most 8"],
      [{ note: "x".repeat(2001) }, "note: at most 2000 characters"],
      [{ metadata: ["a"] }, "metadata: names and values"],
      [{ metadata: { "has space": "v" } }, 'metadata: "has space" is not a name (letters, digits, . _ -, up to 40)'],
      [{ metadata: { a: 1 } }, "metadata: a must be words of at most 200 characters"],
      [
        { metadata: Object.fromEntries(Array.from({ length: 13 }, (_, i) => [`k${i}`, "v"])) },
        "metadata: at most 12 names",
      ],
    ] as const) {
      expect(() => filingOptions(args as Record<string, unknown>, REPO)).toThrow(said);
    }
  });

  test("on the issue, only what is not the default; in the task, all of it", () => {
    expect(ownerLines(filingOptions({}, REPO), REPO)).toEqual([]);
    const o = filingOptions(
      {
        urgency: "week",
        importance: "low",
        due: "2026-12-01",
        blockers: ["#3", "PR a/b#4"],
        note: "n",
        metadata: { k: "v", j: "w" },
      },
      REPO,
    );
    expect(ownerLines(o, REPO)).toEqual([
      "**Urgency:** This week · **Importance:** Low · queue priority 70",
      "**Wanted by:** 2026-12-01",
      "**Waits for:** #3, pull request a/b#4",
      "**Note:** n",
      "**Metadata:** k=v; j=w",
    ]);
    expect(ownerBlock(filingOptions({}, REPO), 10)).toEqual({ urgency: "now", importance: "critical", priority: 10 });
    expect(ownerBlock(o, 70)).toEqual({
      urgency: "week",
      importance: "low",
      priority: 70,
      due: "2026-12-01",
      blockers: [
        { kind: "issue", repo: REPO, number: 3 },
        { kind: "pr", repo: "a/b", number: 4 },
      ],
      note: "n",
      metadata: { k: "v", j: "w" },
    });
  });
});

/** The queue's store: a real SQLite with the broker's tables, behind the
 * store's HTTP pipeline. `before` runs ahead of each statement (to make
 * something happen between a read and a write). */
function store(options: { mirror?: boolean } = {}) {
  const db = brokerDb(options);
  const said: string[] = [];
  const state = { before: null as null | ((sql: string) => void), down: false };
  const fetchImpl: Fetch = async (_url, init) => {
    if (state.down) return { ok: false, status: 503, json: async () => ({}) };
    const requests = (
      JSON.parse(init.body ?? "{}").requests as {
        type: string;
        stmt?: { sql: string; args: { value?: string; type: string }[] };
      }[]
    ).filter((r) => r.type === "execute");
    const results = requests.map((r) => {
      const sql = r.stmt?.sql ?? "";
      said.push(sql);
      state.before?.(sql);
      return storeAnswer(db, sql, r.stmt?.args ?? []);
    });
    return { ok: true, status: 200, json: async () => ({ results }) };
  };
  const run: Run = async (argv) =>
    argv[1] === "secrets" ? { code: 0, stdout: "token\n", stderr: "" } : { code: 1, stdout: "", stderr: "no" };
  const ops = new Ops(CFG, run, asThePlatformFetch(fetchImpl) as Fetch);
  const tasks = () => db.query("SELECT * FROM tasks ORDER BY id").all() as Record<string, unknown>[];
  type Event = { worker_id: unknown; task_id: unknown; kind: unknown; detail: unknown; created_at: unknown };
  const events = (): Event[] =>
    (db.query("SELECT worker_id, task_id, kind, detail, created_at FROM events ORDER BY id").all() as Event[]).map(
      (e) => ({
        ...e,
        detail: JSON.parse(String(e.detail)),
      }),
    );
  const put = (row: Record<string, unknown>) => {
    const cols = Object.keys(row);
    db.query(`INSERT INTO tasks (${cols.join(", ")}) VALUES (${cols.map(() => "?").join(", ")})`).run(
      ...(Object.values(row) as never[]),
    );
  };
  return { db, ops, said, state, tasks, events, put };
}

const askPacket = (n: number, opts = filingOptions({}, REPO), reply = false) =>
  promptPacket(
    REPO,
    CFG.queue,
    n,
    "Do the thing",
    NOW.toISOString(),
    "task",
    opts,
    reply ? null : [CFG.promptLabel, `priority/${category(claimPriority(opts.urgency, opts.importance))}`],
    reply,
  );
const KEY = (n: number) => `repo-health:v1:triage:${REPO}:issue:${n}`;
/** A row like the queue's own, for what was there before. */
const older = (id: number, status: string, n: number, more: Record<string, unknown> = {}) => ({
  id,
  queue: CFG.queue,
  title: `older ${id}`,
  payload: JSON.stringify({ kind: "triage", repo: REPO, issue: { number: n } }),
  dedupe_key: KEY(n),
  priority: 30,
  status,
  created_at: "2026-11-20T00:00:00Z",
  updated_at: "2026-11-21T00:00:00Z",
  ...more,
});

describe("filing a task, straight into the queue's store", () => {
  test("the broker's own INSERT writes these columns, in this order; the block, the dependency columns", () => {
    const claim = read(join(BROKER, "claim.rs"));
    const body = claim.slice(claim.indexOf("pub(crate) fn verb_enqueue("));
    const insert = /INSERT INTO tasks \(([^)]*)\)/.exec(body.replace(/\\\n\s*/g, ""));
    expect(insert?.[1].split(",").map((c) => c.trim())).toEqual([...BROKER_COLUMNS]);
    const admin = read(join(BROKER, "admin.rs"));
    const block = admin
      .slice(admin.indexOf("pub(crate) fn verb_block("), admin.indexOf("pub(crate) fn verb_block(") + 6000)
      .replace(/\\\n\s*/g, "");
    for (const col of [
      "blocked_deps = ?",
      "dep_next_recheck_at = ?",
      "dep_blocked_until = ?",
      "dep_recheck_seconds = ?",
    ])
      expect(block).toContain(col);
    expect(block).toContain('"blocked"');
    expect(block).toContain('"reason_class"');
  });

  test("a new prompt is filed as the broker files it: the row, queued, the times to the second, and no event", async () => {
    const s = store();
    const packet = askPacket(501);
    const filed = await fileTask(s.ops, packet, NOW);
    expect(filed).toEqual({ filed: true, task: 1, status: "queued", priority: 10, said: "filed as task 1" });
    const [row] = s.tasks();
    const want = taskRow(packet, CFG.queue);
    expect({
      queue: row.queue,
      title: row.title,
      body: row.body,
      payload: JSON.parse(String(row.payload)),
      dedupe_key: row.dedupe_key,
      priority: row.priority,
      not_before: row.not_before,
      max_attempts: row.max_attempts,
      required_budget_minutes: row.required_budget_minutes,
    }).toEqual(want);
    // As the broker leaves a new task.
    expect([row.status, row.platform, row.progress, row.result, row.attempts, row.worker_id, row.lease_until]).toEqual([
      "queued",
      null,
      "{}",
      "{}",
      0,
      null,
      null,
    ]);
    expect([row.created_at, row.updated_at, row.completed_at]).toEqual([
      "2026-11-24T19:30:00Z",
      "2026-11-24T19:30:00Z",
      null,
    ]);
    expect([row.blocked_deps, row.dep_next_recheck_at, row.dep_blocked_until, row.dep_recheck_seconds]).toEqual([
      null,
      null,
      null,
      null,
    ]);
    expect(s.events()).toEqual([]);
    // The time is the queue's: to the second.
    expect(queueTime(new Date("2026-01-02T03:04:05.999Z"))).toBe("2026-01-02T03:04:05Z");
    // The labels came with the packet: the mirror was not asked.
    expect(s.said.some((sql) => sql.includes("github_mirror_issues"))).toBe(false);
    // The key is read by its index.
    const plan = (
      s.db
        .query(
          `EXPLAIN QUERY PLAN SELECT id, status, priority, completed_at FROM tasks WHERE queue = ? AND dedupe_key = ? ORDER BY id ASC`,
        )
        .all(CFG.queue, KEY(501)) as { detail: string }[]
    )
      .map((r) => r.detail)
      .join(" | ");
    expect(plan).toContain("USING INDEX tasks_queue_dedupe");
  });

  test("filed again, it is the same task: nothing new, and said so", async () => {
    const s = store();
    await fileTask(s.ops, askPacket(502), NOW);
    const again = await fileTask(s.ops, askPacket(502), NOW);
    expect(again).toEqual({
      filed: false,
      task: 1,
      status: "queued",
      priority: 10,
      said: "task 1 is already in the queue",
    });
    expect(s.tasks().length).toBe(1);
  });

  test("a done task with the key is the answer (#13091), unless the packet says why another is wanted: then it is revived from it", async () => {
    const s = store();
    s.put(older(7, "done", 503, { completed_at: "2026-11-22T00:00:00Z" }));
    expect(await fileTask(s.ops, askPacket(503), NOW)).toEqual({
      filed: false,
      task: 7,
      status: "done",
      priority: 30,
      said: "task 7 was done already",
    });
    expect(s.tasks().length).toBe(1);
    // A reply states an override: a new task, linked to the done one, with the broker's three events.
    const reply = askPacket(503, filingOptions({}, REPO), true);
    const filed = await fileTask(s.ops, reply, NOW);
    expect([filed.filed, filed.task, filed.status]).toEqual([true, 8, "queued"]);
    const link = {
      task_id: 7,
      status: "done",
      completed_at: "2026-11-22T00:00:00Z",
      revived_at: "2026-11-24T19:30:00Z",
      via: "enqueue_repo_task_turso",
    };
    expect(JSON.parse(String(s.tasks()[1].progress))).toEqual({ revived_from: link });
    expect(s.events()).toEqual([
      {
        worker_id: "pm-console",
        task_id: 8,
        kind: "task_revived",
        detail: { predecessor: link, dedupe_key: KEY(503) },
        created_at: "2026-11-24T19:30:00Z",
      },
      {
        worker_id: "pm-console",
        task_id: 7,
        kind: "revived_by",
        detail: { successor_task_id: 8, dedupe_key: KEY(503) },
        created_at: "2026-11-24T19:30:00Z",
      },
      {
        worker_id: "pm-console",
        task_id: 8,
        kind: "dedupe_override",
        detail: { reason: "the owner replied at 2026-11-24T19:30:00.123Z", identity: `${REPO}#503:triage` },
        created_at: "2026-11-24T19:30:00Z",
      },
    ]);
  });

  test("a cancelled or failed task with the key is revived from, with no override; the newest finished one is the one", async () => {
    const s = store();
    s.put(older(3, "failed", 504));
    s.put(older(4, "cancelled", 504));
    const filed = await fileTask(s.ops, askPacket(504), NOW);
    expect([filed.filed, filed.task]).toEqual([true, 5]);
    expect(s.events().map((e) => [e.kind, e.task_id])).toEqual([
      ["task_revived", 5],
      ["revived_by", 4],
    ]);
    // A done one among them, behind the newest: the broker takes the done
    // one as holding the work (done_identity_holder), which is its to say.
    const t = store();
    t.put(older(3, "done", 518));
    t.put(older(4, "cancelled", 518));
    await expect(fileTask(t.ops, askPacket(518), NOW)).rejects.toThrow("task 3 already works on #518");
    expect(t.tasks().length).toBe(2);
  });

  test("another task already working on the issue under another key is the broker's to judge: nothing is written", async () => {
    const s = store();
    s.put(older(9, "claimed", 505, { dedupe_key: `${KEY(505)}:dispatch:0123abcd` }));
    await expect(fileTask(s.ops, askPacket(505), NOW)).rejects.toThrow(NeedsBroker);
    await expect(fileTask(s.ops, askPacket(505), NOW)).rejects.toThrow("task 9 already works on #505");
    expect(s.tasks().length).toBe(1);
    // A failed one does not count; a done one does.
    const t = store();
    t.put(older(9, "failed", 506, { dedupe_key: "other" }));
    expect((await fileTask(t.ops, askPacket(506), NOW)).filed).toBe(true);
    const u = store();
    u.put(older(9, "done", 507, { dedupe_key: "other" }));
    await expect(fileTask(u.ops, askPacket(507), NOW)).rejects.toThrow(NeedsBroker);
    // With an override the broker does not look, and neither does this.
    expect((await fileTask(u.ops, askPacket(507, filingOptions({}, REPO), true), NOW)).filed).toBe(true);
  });

  test("filed by something else between the look and the write, it is written once", async () => {
    const s = store();
    s.state.before = (sql) => {
      if (sql.startsWith("INSERT INTO tasks")) {
        s.state.before = null;
        s.put(older(40, "queued", 508));
      }
    };
    expect(await fileTask(s.ops, askPacket(508), NOW)).toEqual({
      filed: false,
      task: 40,
      status: "queued",
      priority: 30,
      said: "task 40 was filed a moment ago",
    });
    expect(s.tasks().map((r) => r.id)).toEqual([40]);
  });

  test("waiting for others, it is filed blocked on them, as a block leaves a task, and the broker's own recheck finds it and frees it", async () => {
    const s = store();
    const opts = filingOptions({ blockers: ["#14432", "PR #14422", "task 23198"] }, REPO);
    const filed = await fileTask(s.ops, askPacket(509, opts), NOW, opts.blockers);
    expect(filed).toEqual({
      filed: true,
      task: 1,
      status: "blocked",
      priority: 10,
      said: "filed as task 1, waiting for 3 others",
    });
    const [row] = s.tasks();
    const deps = [
      {
        kind: "issue",
        state: "open",
        reason: "named by the owner when filing",
        recorded_by: "pm-console",
        recorded_at: "2026-11-24T19:30:00Z",
        last_checked_at: null,
        last_observed: null,
        repo: REPO,
        number: 14432,
      },
      {
        kind: "pr",
        state: "open",
        reason: "named by the owner when filing",
        recorded_by: "pm-console",
        recorded_at: "2026-11-24T19:30:00Z",
        last_checked_at: null,
        last_observed: null,
        repo: REPO,
        number: 14422,
      },
      {
        kind: "task",
        state: "open",
        reason: "named by the owner when filing",
        recorded_by: "pm-console",
        recorded_at: "2026-11-24T19:30:00Z",
        last_checked_at: null,
        last_observed: null,
        task_id: 23198,
      },
    ];
    expect(row.status).toBe("blocked");
    expect(JSON.parse(String(row.blocked_deps))).toEqual(deps);
    // Looked at again at once, every 15 minutes, for seven days at most.
    expect([row.dep_next_recheck_at, row.dep_blocked_until, row.dep_recheck_seconds]).toEqual([
      "2026-11-24T19:30:00Z",
      "2026-12-01T19:30:00Z",
      900,
    ]);
    const marks = {
      blocker: "waits for #14432, pull request #14422, task 23198",
      reason_class: "dependency",
      blocked_via: "owner",
    };
    expect([JSON.parse(String(row.progress)), JSON.parse(String(row.result))]).toEqual([marks, marks]);
    expect(s.events()).toEqual([
      {
        worker_id: "pm-console",
        task_id: 1,
        kind: "blocked",
        // As the broker's block event: the blocker and its reason, and the dependencies.
        detail: {
          blocker: marks.blocker,
          reason_class: "dependency",
          deps,
          dep_blocked_until: "2026-12-01T19:30:00Z",
          dep_recheck_seconds: 900,
        },
        created_at: "2026-11-24T19:30:00Z",
      },
    ]);
    // The broker's recheck, as its source has it, picks this task up.
    const admin = read(join(BROKER, "admin.rs")).replace(/\\\n\s*/g, "");
    const pick =
      /"SELECT \{TASK_COLS\} FROM tasks (WHERE status = 'blocked' AND blocked_deps IS NOT NULL AND \(dep_next_recheck_at IS NULL OR dep_next_recheck_at <= \?\))/.exec(
        admin,
      );
    expect(pick).not.toBeNull();
    expect(s.db.query(`SELECT id FROM tasks ${pick?.[1]}`).all("2026-11-24T19:30:00Z")).toEqual([{ id: 1 }]);
    // And its release (the time run out, here) frees it to be claimed.
    const release =
      /"(UPDATE tasks SET status = 'queued', blocked_deps = NULL, dep_next_recheck_at = NULL, dep_blocked_until = NULL, dep_recheck_seconds = NULL, updated_at = \? WHERE id = \? AND status = 'blocked')"/.exec(
        admin,
      );
    expect(release).not.toBeNull();
    s.db.query(release?.[1] as string).run("2026-12-01T19:30:00Z", 1);
    expect(s.tasks()[0].status).toBe("queued");
  });

  test("with no labels of its own, the issue's labels come from the mirror as the broker takes them; a mirror that cannot be read is passed over", async () => {
    const s = store();
    s.db
      .query("INSERT INTO github_mirror_issues (repo, number, labels, mirrored_at) VALUES (?, ?, ?, ?)")
      .run(REPO, 510, JSON.stringify(["expedite", "kind/bug"]), "2026-11-24T19:00:00Z");
    const packet = triagePacket(REPO, CFG.queue, 510, "Bug: x", "now", NOW.toISOString()) as Record<string, unknown>;
    const filed = await fileTask(s.ops, packet, NOW);
    expect(filed.priority).toBe(15);
    expect(JSON.parse(String(s.tasks()[0].payload)).labels).toEqual(["expedite", "kind/bug"]);
    // No row for the issue: the stated priority.
    expect(
      (
        await fileTask(
          s.ops,
          triagePacket(REPO, CFG.queue, 511, "Bug: y", "now", NOW.toISOString()) as Record<string, unknown>,
          NOW,
        )
      ).priority,
    ).toBe(20);
    // No mirror at all: the same, and the filing goes on.
    const t = store({ mirror: false });
    const plain = await fileTask(
      t.ops,
      triagePacket(REPO, CFG.queue, 512, "Bug: z", "now", NOW.toISOString()) as Record<string, unknown>,
      NOW,
    );
    expect([plain.filed, plain.priority]).toEqual([true, 20]);
  });

  test("what cannot be filed is refused before anything is written; a store that does not answer is thrown as it is", async () => {
    const s = store();
    const big = askPacket(
      513,
      filingOptions(
        {
          note: "x".repeat(2000),
          metadata: Object.fromEntries(Array.from({ length: 12 }, (_, i) => [`k${i}`, "y".repeat(200)])),
        },
        REPO,
      ),
    );
    (big.payload as Record<string, unknown>).evidence = ["z".repeat(8000)];
    await expect(fileTask(s.ops, big, NOW)).rejects.toThrow("too long to file as one task");
    await expect(fileTask(s.ops, { ...askPacket(514), priority: 15 }, NOW)).rejects.toThrow(
      "the task's priority is not one of",
    );
    await expect(fileTask(s.ops, { ...askPacket(515), title: " " }, NOW)).rejects.toThrow("the task has no title");
    await expect(fileTask(s.ops, { ...askPacket(516), dedupe_key: undefined }, NOW)).rejects.toThrow(NeedsBroker);
    expect(s.tasks()).toEqual([]);
    s.state.down = true;
    await expect(fileTask(s.ops, askPacket(517), NOW)).rejects.toThrow("queue store answered http 503");
  });
});
