// libSQL parity tests (task 23569, the production-store remainder of
// #12589): the door-acceptance flows of test/queue.test.ts, replayed
// over LibSQLQueueStore on a real file-backed local libSQL/SQLite DB
// carrying the shared fleet-queue schema (#6741) — no fixture, no
// second queue. Oracle: the same verb engine must produce the same
// outcomes (cross-door claim both directions with one timeline,
// exactly one winner for concurrent two-door claims, the losing
// conditional update re-picking to nothing, anonymous refusal at the
// hosted door with the native door unaffected), and the on-disk
// schema must name the native queue tables and columns.
//
// Driver: node:sqlite (the house SqlStorageLike pattern from
// pkgs/id/hosted-worker) over a temp file — the store itself never
// imports it, so the Worker bundle stays runtime-agnostic.

import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { DatabaseSync } from "node:sqlite";
import { dispatch } from "../src/router.ts";
import { executeQueueVerb, nativeQueueEnv, type QueueEnv } from "../src/queue.ts";
import { NO_DEP_PROBE, type QueueStore, type TaskRow } from "../src/queue-store.ts";
import { LibSQLQueueStore, LIBSQL_QUEUE_SCHEMA, type SyncSqlDb } from "../src/libsql-queue-store.ts";
import { StoreQueueRecordSource } from "../src/queue-records.ts";
import { FixtureGitHubWriter } from "../src/github.ts";
import { FixtureSessionRegistry } from "../src/sessions.ts";
import { FixtureMetadataStore } from "../src/stamping.ts";
import { iso, type JsonObject } from "../src/queue-validate.ts";
import { BINDING_A, makeHarness } from "./helpers.ts";

const T0 = 1767225600; // 2026-01-01T00:00:00Z
let clock = T0;

function openStore(): { store: LibSQLQueueStore; db: DatabaseSync; dir: string } {
  clock = T0;
  const dir = mkdtempSync(join(tmpdir(), "fleet-libsql-parity-"));
  const db = new DatabaseSync(join(dir, "fleet-queue.db"));
  const sql: SyncSqlDb = {
    exec(query: string, ...params: (string | number | bigint | boolean | null | ArrayBuffer | Uint8Array)[]) {
      const stmt = db.prepare(query);
      if (/^\s*(select|pragma|values|with)\b/i.test(query)) {
        const rows = stmt.all(...(params as never[])) as Record<string, unknown>[];
        return { toArray: () => rows };
      }
      stmt.run(...(params as never[]));
      return { toArray: () => [] as Record<string, unknown>[] };
    },
  };
  const store = new LibSQLQueueStore(sql, { nowIso: () => iso(clock) });
  return { store, db, dir };
}

function closeOpened(t: ReturnType<typeof openStore>): void {
  t.db.close();
  rmSync(t.dir, { recursive: true, force: true });
}

function hostedEnv(bindingId: string | null = BINDING_A): QueueEnv {
  return { nowEpoch: () => clock, depProbe: NO_DEP_PROBE, bindingId };
}

function nativeEnv(): QueueEnv {
  return nativeQueueEnv(() => clock);
}

type Packet = ReturnType<typeof fixPacket>;

function fixPacket(issue = 4242, payloadExtra: JsonObject = {}, packetExtra: JsonObject = {}): Packet {
  return {
    title: `fix issue ${issue}`,
    priority: 30,
    dedupe_key: `repo-health:v1:fix:developing-today/code:issue:${issue}`,
    payload: {
      schema: "repo-task/v1",
      kind: "small_fix",
      repo: "developing-today/code",
      issue: { number: issue },
      problem: "the thing is broken",
      priority_reason: "it blocks the queue",
      evidence: ["https://example.test/evidence"],
      acceptance: ["the thing works"],
      tests: ["npm test"],
      context_refs: [],
      scope: { include: ["pkgs/fleet"], exclude: ["docs"] },
      risk: { level: "low", flags: [] },
      timebox_minutes: 45,
      dedupe: { checked_at: "2026-01-01T00:00:00Z", basis: "test fixture" },
      ...payloadExtra,
    },
    ...packetExtra,
  };
}

function enqueue(store: QueueStore, env: QueueEnv, packet: JsonObject, extra: JsonObject = {}): JsonObject {
  return executeQueueVerb(store, "enqueue", { queue: "repo-health", packet, ...extra }, env);
}

function claim(store: QueueStore, env: QueueEnv, extra: JsonObject = {}): JsonObject {
  return executeQueueVerb(store, "claim", { worker: "worker-a", ...extra }, env);
}

function verb(store: QueueStore, env: QueueEnv, name: string, req: JsonObject): JsonObject {
  return executeQueueVerb(store, name, req, env);
}

// ---------------------------------------------------------------------------
// Schema parity: the same queue tables and columns as #6741
// ---------------------------------------------------------------------------

const NATIVE_TASK_COLS = (
  "id, queue, title, body, payload, dedupe_key, priority, status, " +
  "not_before, worker_id, claimed_at, lease_until, lane_id, platform, attempts, max_attempts, " +
  "claim_provider, claim_model, claim_variant, claim_session_id, " +
  "progress, result, last_error, park_kind, wait_kind, ready_condition, " +
  "required_budget_minutes, " +
  "last_checked_at, check_outcome, created_at, updated_at, completed_at, " +
  "blocked_deps, dep_next_recheck_at, dep_blocked_until, dep_recheck_seconds"
).split(", ");

const NATIVE_LEDGER_COLS = (
  "id, queue, task_id, assigned_lane, executed_lane, executed_lane_id, " +
  "exclusion_reason, opencode_reason, platform_limit, platform_limit_reason, " +
  "assigned_by, session_id, fallback_reason, " +
  "assigned_at, executed_at, updated_at"
).split(", ");

test("schema: the store carries the native queue tables and columns, no second queue", () => {
  const t = openStore();
  try {
    const tables = (
      t.db.prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name").all() as {
        name: string;
      }[]
    ).map((r) => r.name);
    for (const expected of ["tasks", "leases", "lanes", "workers", "checkpoints", "repo_task_lane_ledger", "events"]) {
      assert.ok(tables.includes(expected), `missing queue table ${expected}`);
    }
    assert.ok(!tables.includes("github_mirror_issues"), "the mirror is the github-ci Worker's table, not this store's");
    const taskCols = (t.db.prepare("PRAGMA table_info(tasks)").all() as { name: string }[]).map((r) => r.name);
    assert.deepEqual(taskCols, NATIVE_TASK_COLS);
    const ledgerCols = (t.db.prepare("PRAGMA table_info(repo_task_lane_ledger)").all() as { name: string }[]).map(
      (r) => r.name,
    );
    assert.deepEqual(ledgerCols, NATIVE_LEDGER_COLS);
    assert.ok(LIBSQL_QUEUE_SCHEMA.join("\n").includes("repo_task_lane_ledger"));
  } finally {
    closeOpened(t);
  }
});

test("store: a full task row round-trips every column through the shared schema", () => {
  const t = openStore();
  try {
    const row = t.store.insertTask({
      queue: "repo-health",
      title: "t",
      body: "b",
      payload: { schema: "repo-task/v1", tags: ["a", "b"], n: 3 },
      dedupe_key: "k1",
      priority: 30,
      status: "claimed",
      not_before: "2026-01-02T00:00:00Z",
      worker_id: "w",
      claimed_at: "2026-01-01T00:00:00Z",
      lease_until: "2026-01-01T01:00:00Z",
      lane_id: "code-i",
      platform: "openchamber",
      attempts: 2,
      max_attempts: 1000,
      claim_provider: "p",
      claim_model: "m",
      claim_variant: "v",
      claim_session_id: "s",
      progress: { stage: "x" },
      result: { ok: true },
      last_error: "e",
      park_kind: "pk",
      wait_kind: "wk",
      ready_condition: { task: 7 },
      required_budget_minutes: 45,
      last_checked_at: "2026-01-01T00:00:00Z",
      check_outcome: "open",
      created_at: "2026-01-01T00:00:00Z",
      updated_at: "2026-01-01T00:00:00Z",
      completed_at: null,
      blocked_deps: [{ kind: "task", task: 9 }],
      dep_next_recheck_at: "2026-01-01T00:05:00Z",
      dep_blocked_until: "2026-01-02T00:00:00Z",
      dep_recheck_seconds: 300,
    });
    assert.deepEqual(t.store.getTask(row.id), row);
    assert.deepEqual(t.store.tasksByDedupe("repo-health", "k1").map((r) => r.id), [row.id]);
    assert.equal(t.store.getTask(row.id + 1), null);
  } finally {
    closeOpened(t);
  }
});

test("store: a losing conditional update reports false and leaves the row", () => {
  const t = openStore();
  try {
    const row = t.store.insertTask({
      queue: "repo-health",
      title: null,
      body: null,
      payload: {},
      dedupe_key: null,
      priority: null,
      status: "queued",
      not_before: null,
      worker_id: null,
      claimed_at: null,
      lease_until: null,
      lane_id: null,
      platform: null,
      attempts: 0,
      max_attempts: 1000,
      claim_provider: null,
      claim_model: null,
      claim_variant: null,
      claim_session_id: null,
      progress: {},
      result: {},
      last_error: null,
      park_kind: null,
      wait_kind: null,
      ready_condition: null,
      required_budget_minutes: null,
      last_checked_at: null,
      check_outcome: null,
      created_at: iso(clock),
      updated_at: iso(clock),
      completed_at: null,
      blocked_deps: null,
      dep_next_recheck_at: null,
      dep_blocked_until: null,
      dep_recheck_seconds: null,
    } as Omit<TaskRow, "id">);
    assert.equal(
      t.store.conditionalUpdateTask(row.id, { status: "claimed" }, { worker_id: "x" }),
      false,
    );
    assert.equal(t.store.getTask(row.id)?.worker_id, null);
    assert.equal(
      t.store.conditionalUpdateTask(row.id, { status: "queued", worker_id: null }, { status: "claimed", worker_id: "w" }),
      true,
    );
    assert.equal(t.store.getTask(row.id)?.worker_id, "w");
  } finally {
    closeOpened(t);
  }
});

// ---------------------------------------------------------------------------
// Acceptance 1 + 2 over the real DB
// ---------------------------------------------------------------------------

test("acceptance 1 (libsql): hosted-enqueued work is claimed natively and vice versa, one timeline", () => {
  const t = openStore();
  try {
    const store = t.store;
    const enq = enqueue(store, hostedEnv(), fixPacket());
    const id = (enq["row"] as JsonObject)["id"] as number;
    const claimedNative = claim(store, nativeEnv(), { worker: "native-worker" });
    assert.equal(claimedNative["ok"], true);
    assert.equal((claimedNative["row"] as JsonObject)["worker_id"], "native-worker");
    const hb = verb(store, hostedEnv(), "heartbeat", { task_id: id, worker: "native-worker" });
    assert.equal(hb["ok"], true, "the hosted door heartbeats the native claim: same worker identity");
    const doneNative = verb(store, nativeEnv(), "complete", {
      task_id: id,
      worker: "native-worker",
      result: {
        evidence: "commit abc123",
        disposition: {
          checked_at: iso(clock),
          source: "test",
          issue: { number: 4242, state: "closed" },
          pulls: [],
          verdict: "closed_verified",
        },
      },
    });
    assert.equal(doneNative["ok"], true);

    const enq2 = enqueue(store, nativeEnv(), fixPacket(5151));
    const id2 = (enq2["row"] as JsonObject)["id"] as number;
    const claimedHosted = claim(store, hostedEnv(), { worker: "hosted-worker" });
    assert.equal(claimedHosted["ok"], true);
    assert.equal((claimedHosted["row"] as JsonObject)["id"], id2);

    const timeline = verb(store, hostedEnv(), "timeline", { task_id: id2 });
    const rows = (timeline["row"] as JsonObject)["rows"] as JsonObject[];
    const claimedEvent = rows.find((r) => r["kind"] === "claimed");
    assert.ok(claimedEvent);
    assert.equal((claimedEvent["detail"] as JsonObject)["binding_id"], BINDING_A);
    const timeline1 = verb(store, nativeEnv(), "timeline", { task_id: id });
    const rows1 = (timeline1["row"] as JsonObject)["rows"] as JsonObject[];
    const nativeClaimed = rows1.find((r) => r["kind"] === "claimed");
    assert.ok(nativeClaimed);
    assert.equal((nativeClaimed["detail"] as JsonObject)["binding_id"], undefined);
  } finally {
    closeOpened(t);
  }
});

test("acceptance 2 (libsql): two doors claiming concurrently for one item produce exactly one claim", () => {
  const t = openStore();
  try {
    const store = t.store;
    enqueue(store, hostedEnv(), fixPacket());
    const first = claim(store, hostedEnv(), { worker: "door-one" });
    const second = claim(store, nativeEnv(), { worker: "door-two" });
    assert.equal((first["row"] as JsonObject)["worker_id"], "door-one");
    assert.equal(second["row"], null);
    const claimedEvents = store.allEvents().filter((e) => e.kind === "claimed");
    assert.equal(claimedEvents.length, 1);
  } finally {
    closeOpened(t);
  }
});

test("acceptance 2 race at the seam (libsql): a preempted conditional update re-picks and gets nothing", () => {
  const t = openStore();
  try {
    class RacingStore extends LibSQLQueueStore {
      raced = false;
      override conditionalUpdateTask(
        id: number,
        guard: Partial<TaskRow>,
        set: Partial<TaskRow>,
      ): boolean {
        if (!this.raced && set.status === "claimed" && guard.status === "queued") {
          this.raced = true;
          super.conditionalUpdateTask(
            id,
            { status: "queued" },
            {
              status: "claimed",
              worker_id: "racer",
              lease_until: "2999-01-01T00:00:00Z",
            },
          );
        }
        return super.conditionalUpdateTask(id, guard, set);
      }
    }
    // Re-open the same file through the racing subclass: one shared
    // backend, two writers — the interleaving the native backend
    // arbitrates with its conditional UPDATE.
    const racing = new RacingStore(
      {
        exec: (query: string, ...params: (string | number | bigint | boolean | null | ArrayBuffer | Uint8Array)[]) => {
          const stmt = t.db.prepare(query);
          if (/^\s*(select|pragma|values|with)\b/i.test(query)) {
            const rows = stmt.all(...(params as never[])) as Record<string, unknown>[];
            return { toArray: () => rows };
          }
          stmt.run(...(params as never[]));
          return { toArray: () => [] as Record<string, unknown>[] };
        },
      },
      { nowIso: () => iso(clock) },
    );
    enqueue(racing, hostedEnv(), fixPacket());
    const res = claim(racing, hostedEnv(), { worker: "door-one" });
    assert.equal(racing.raced, true);
    assert.equal(res["row"], null, "the losing door claims nothing");
    assert.equal(racing.allTasks()[0].worker_id, "racer");
  } finally {
    closeOpened(t);
  }
});

// ---------------------------------------------------------------------------
// Mirror read model: null without a row, labels with one, throw on failure
// ---------------------------------------------------------------------------

const MIRROR_DDL = `CREATE TABLE IF NOT EXISTS github_mirror_issues (
  repo TEXT NOT NULL, number INTEGER NOT NULL,
  title TEXT, body_hash TEXT, state TEXT,
  labels TEXT NOT NULL DEFAULT '[]',
  assignees TEXT NOT NULL DEFAULT '[]',
  comments INTEGER NOT NULL DEFAULT 0,
  comments_gh_updated_at TEXT,
  latest_agent_note_id INTEGER, latest_agent_note_url TEXT,
  author TEXT, html_url TEXT,
  gh_created_at TEXT, gh_updated_at TEXT,
  mirrored_at TEXT NOT NULL,
  PRIMARY KEY (repo, number))`;

test("mirror (libsql): no row reads null, a row reads its labels, a missing table throws", () => {
  const t = openStore();
  try {
    assert.throws(() => t.store.mirrorLabels("developing-today/code", 4242), /no such table/);
    t.db.prepare(MIRROR_DDL).run();
    assert.equal(t.store.mirrorLabels("developing-today/code", 4242), null);
    t.db
      .prepare("INSERT INTO github_mirror_issues (repo, number, labels, mirrored_at) VALUES (?, ?, ?, ?)")
      .run("developing-today/code", 4242, JSON.stringify(["expedite", "next-up"]), "2026-01-01T00:00:00Z");
    assert.deepEqual(t.store.mirrorLabels("developing-today/code", 4242), ["expedite", "next-up"]);
    t.db.prepare("DROP TABLE github_mirror_issues").run();
    assert.throws(() => t.store.mirrorLabels("developing-today/code", 4242), /no such table/);
  } finally {
    closeOpened(t);
  }
});

// ---------------------------------------------------------------------------
// Router-level: acceptance 3 over the libSQL store
// ---------------------------------------------------------------------------

const h = await makeHarness();

async function routerDeps(store: QueueStore, writer: FixtureGitHubWriter) {
  const sessions = new FixtureSessionRegistry();
  return {
    deps: {
      ...h.deps,
      sessions,
      metadataStore: new FixtureMetadataStore(),
      queueStore: store,
      queueRecords: new StoreQueueRecordSource(store, sessions),
      githubComments: writer,
      now: () => clock * 1000,
    },
    sign: h.sign,
    rpPayload: h.rpPayload,
  };
}

function post(path: string, token: string | null, body?: unknown): Request {
  return new Request(`https://fleet-hosted.test${path}`, {
    method: "POST",
    headers: {
      ...(token ? { authorization: `Bearer ${token}` } : {}),
      "content-type": "application/json",
    },
    body: typeof body === "string" ? body : JSON.stringify(body ?? {}),
  });
}

test("acceptance 3 (libsql): an anonymous claim at the hosted door is refused; the native door is unaffected", async () => {
  const t = openStore();
  try {
    const writer = new FixtureGitHubWriter();
    const { deps } = await routerDeps(t.store, writer);
    const anon = await dispatch(post("/v1/queue/claim", null, { worker: "anon" }), deps);
    assert.equal(anon.status, 401);
    const enq = enqueue(t.store, nativeEnv(), fixPacket());
    assert.equal(enq["ok"], true);
    const claimed = claim(t.store, nativeEnv(), { worker: "native-worker" });
    assert.equal(claimed["ok"], true);
    assert.equal(describeHealth(t.store)["backend"], "libsql");
  } finally {
    closeOpened(t);
  }
});

function describeHealth(store: QueueStore): { backend: string } {
  return store.describe();
}
