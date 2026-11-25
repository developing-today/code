// Parity for the D1 queue store (#14866 — the fleet queues' move to
// Workers + D1): D1QueueStore against the acceptance scenarios
// libsql-queue-parity.test.ts pins for the embedded store and
// libsql-hrana-parity.test.ts pins for the remote store, plus the
// two proofs only this door can give: the production binding
// adapter (D1DatabaseClient over the D1Database prepare/bind/all/run
// API, driven here by a faithful in-process double over a real
// SQLite file) and cross-store interop (rows written through the D1
// store read back identically through the embedded LibSQL store on
// the same database, and vice versa — one schema, one queue, three
// transports).
//
// Oracle: the same verb engine must produce the same outcomes over
// every store (cross-door claim both directions with one timeline,
// exactly one winner for concurrent two-door claims, the losing
// conditional update re-picking to nothing), and buildDeps must
// select the D1 store when the QUEUE_D1 binding is present. A real
// D1 run belongs to the #14866 deployment slice; this suite proves
// the store and its adapter against the documented D1 API shape
// over a real SQLite engine, never a mocked SQL layer.

import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { DatabaseSync } from "node:sqlite";
import { executeQueueVerb, nativeQueueEnv, type QueueEnv } from "../src/queue.ts";
import { NO_DEP_PROBE, type QueueStore, type TaskRow } from "../src/queue-store.ts";
import { LibSQLQueueStore, type SyncSqlDb } from "../src/libsql-queue-store.ts";
import {
  D1QueueStore,
  openD1QueueStore,
  type D1Client,
  type D1DatabaseLike,
  type D1MetaLike,
  type D1PreparedStatementLike,
  type D1Result,
  type D1Statement,
} from "../src/d1-queue-store.ts";
import { buildDeps } from "../src/worker.ts";
import type { HostedEnv } from "../src/config.ts";
import { iso, type JsonObject } from "../src/queue-validate.ts";
import { BINDING_A } from "./helpers.ts";

const T0 = 1767225600; // 2026-01-01T00:00:00Z
let clock = T0;

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

async function enqueue(
  store: QueueStore,
  env: QueueEnv,
  packet: JsonObject,
  extra: JsonObject = {},
): Promise<JsonObject> {
  return await executeQueueVerb(store, "enqueue", { queue: "repo-health", packet, ...extra }, env);
}

async function claim(store: QueueStore, env: QueueEnv, extra: JsonObject = {}): Promise<JsonObject> {
  return await executeQueueVerb(store, "claim", { worker: "worker-a", ...extra }, env);
}

async function verb(store: QueueStore, env: QueueEnv, name: string, req: JsonObject): Promise<JsonObject> {
  return await executeQueueVerb(store, name, req, env);
}

// ---------------------------------------------------------------------------
// The in-process D1 client: node:sqlite behind the D1Client seam,
// reporting D1-shaped results (changes / last_row_id from the
// statement outcome itself, never connection-scoped state).
// ---------------------------------------------------------------------------

class SqliteD1Client implements D1Client {
  constructor(private readonly db: DatabaseSync) {}

  async execute(stmt: D1Statement): Promise<D1Result> {
    const prepared = this.db.prepare(stmt.sql);
    const args = stmt.args.map((a) => (typeof a === "boolean" ? (a ? 1 : 0) : a)) as never[];
    if (/^\s*(select|pragma|values|with)\b/i.test(stmt.sql)) {
      const rows = prepared.all(...args) as Record<string, unknown>[];
      return { rows, affectedRowCount: 0, lastInsertRowId: null };
    }
    const info = prepared.run(...args) as { changes: number | bigint; lastInsertRowid: number | bigint };
    return { rows: [], affectedRowCount: Number(info.changes), lastInsertRowId: Number(info.lastInsertRowid) };
  }
}

// ---------------------------------------------------------------------------
// The D1 binding double: the documented D1Database API
// (prepare → bind → all/run, meta carrying changes + last_row_id)
// over the same node:sqlite file, so the production
// D1DatabaseClient — the code a deployed Worker runs — is what the
// binding suite exercises.
// ---------------------------------------------------------------------------

class FakeD1PreparedStatement {
  private values: unknown[] = [];

  constructor(
    private readonly db: DatabaseSync,
    private readonly sqlText: string,
  ) {}

  bind(...values: unknown[]): FakeD1PreparedStatement {
    this.values = values;
    return this;
  }

  async all(): Promise<{ results: Record<string, unknown>[]; meta: { changes: number; last_row_id: number } }> {
    const rows = this.db.prepare(this.sqlText).all(...(this.values as never[])) as Record<string, unknown>[];
    return { results: rows, meta: { changes: 0, last_row_id: 0 } };
  }

  async run(): Promise<{ meta: { changes: number; last_row_id: number } }> {
    const info = this.db.prepare(this.sqlText).run(...(this.values as never[])) as {
      changes: number | bigint;
      lastInsertRowid: number | bigint;
    };
    return { meta: { changes: Number(info.changes), last_row_id: Number(info.lastInsertRowid) } };
  }
}

class FakeD1Database implements D1DatabaseLike {
  constructor(private readonly db: DatabaseSync) {}

  prepare(sqlText: string): FakeD1PreparedStatement {
    return new FakeD1PreparedStatement(this.db, sqlText);
  }
}

function openStore(): { store: D1QueueStore; db: DatabaseSync; dir: string } {
  clock = T0;
  const dir = mkdtempSync(join(tmpdir(), "fleet-d1-parity-"));
  const db = new DatabaseSync(join(dir, "fleet-queue.db"));
  const store = new D1QueueStore(new SqliteD1Client(db), { nowIso: () => iso(clock) });
  return { store, db, dir };
}

function closeOpened(t: { db: DatabaseSync; dir: string }): void {
  t.db.close();
  rmSync(t.dir, { recursive: true, force: true });
}

// ---------------------------------------------------------------------------
// Store surface + parity scenarios over the injected transport
// ---------------------------------------------------------------------------

test("d1 store: the full QueueStore surface round-trips over the transport seam", async () => {
  const t = openStore();
  try {
    const row = await t.store.insertTask({
      queue: "repo-health",
      manual_only: false,
      title: "fix issue 4242",
      body: "b",
      payload: fixPacket().payload,
      metadata: {},
      dedupe_key: "repo-health:v1:fix:developing-today/code:issue:4242",
      priority: 30,
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
      progress: { stage: "x" },
      result: { ok: true },
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
    });
    assert.equal(row.id > 0, true);
    assert.deepEqual(await t.store.getTask(row.id), row);
    assert.deepEqual(
      (await t.store.tasksByDedupe("repo-health", row.dedupe_key as string)).map((r) => r.id),
      [row.id],
    );
    assert.equal((await t.store.allTasks()).length, 1);

    // The conditional update is the claim primitive: a stale guard
    // loses and leaves the row; the live guard wins.
    assert.equal(await t.store.conditionalUpdateTask(row.id, { status: "claimed" }, { status: "claimed" }), false);
    assert.equal((await t.store.getTask(row.id))?.status, "queued");
    assert.equal(
      await t.store.conditionalUpdateTask(
        row.id,
        { status: "queued", worker_id: null },
        { status: "claimed", worker_id: "w" },
      ),
      true,
    );
    assert.equal((await t.store.getTask(row.id))?.worker_id, "w");
    // A JSON-column guard canonicalizes both sides (claim-race rule).
    assert.equal(
      await t.store.conditionalUpdateTask(row.id, { payload: fixPacket().payload }, { last_checked_at: iso(clock) }),
      true,
    );

    await t.store.updateTask(row.id, { last_checked_at: iso(clock + 1) });
    assert.equal((await t.store.getTask(row.id))?.last_checked_at, iso(clock + 1));

    const ledger = await t.store.insertLedger({
      queue: "repo-health",
      task_id: row.id,
      assigned_lane: "lane-1",
      executed_lane: null,
      executed_lane_id: null,
      exclusion_reason: null,
      opencode_reason: null,
      platform_limit: null,
      platform_limit_reason: null,
      assigned_by: "test",
      session_id: "ses_1",
      fallback_reason: null,
      assigned_at: iso(clock),
      executed_at: null,
      updated_at: iso(clock),
    });
    assert.equal(ledger.task_id, row.id);
    await t.store.updateLedger(row.id, { executed_lane: "lane-1" });
    assert.equal((await t.store.getLedger(row.id))?.executed_lane, "lane-1");

    await t.store.upsertLease(
      { task_id: row.id, worker_id: "w", lease_until: iso(clock + 600), heartbeat_at: iso(clock) },
      { keepWorker: false },
    );
    await t.store.upsertLease(
      { task_id: row.id, worker_id: "other", lease_until: iso(clock + 900), heartbeat_at: iso(clock + 5) },
      { keepWorker: true },
    );
    await t.store.deleteExpiredLeases(iso(clock + 1000));
    await t.store.deleteLease(row.id);

    assert.equal(await t.store.getLane("lane-1"), null);
    await t.store.insertLane({ lane_id: "lane-1", worker_id: "w", queue: "repo-health", enabled: 1 });
    assert.equal((await t.store.getLane("lane-1"))?.worker_id, "w");
    await t.store.touchWorker("w", true, iso(clock));
    await t.store.upsertCheckpoint("w", "cursor", { page: 2 });

    const ev = await t.store.appendEvent("w", row.id, "claimed", { via: "test" }, iso(clock));
    assert.equal(ev.kind, "claimed");
    assert.equal((await t.store.eventsForTask(row.id)).length, 1);
    assert.equal((await t.store.allEvents()).length, 1);
    assert.equal(await t.store.deleteEventsBefore(iso(clock + 1)), 1);

    await assert.rejects(() => t.store.mirrorLabels("developing-today/code", 4242), /no such table/);
    t.db
      .prepare(
        "CREATE TABLE github_mirror_issues (repo TEXT NOT NULL, number INTEGER NOT NULL, labels TEXT NOT NULL, mirrored_at TEXT NOT NULL)",
      )
      .run();
    assert.equal(await t.store.mirrorLabels("developing-today/code", 4242), null);
    t.db
      .prepare("INSERT INTO github_mirror_issues (repo, number, labels, mirrored_at) VALUES (?, ?, ?, ?)")
      .run("developing-today/code", 4242, JSON.stringify(["expedite", "next-up"]), "2026-01-01T00:00:00Z");
    assert.deepEqual(await t.store.mirrorLabels("developing-today/code", 4242), ["expedite", "next-up"]);
    t.db.prepare("DROP TABLE github_mirror_issues").run();
    assert.deepEqual(await t.store.describe(), { backend: "d1", projectSource: "fleet-queue", netloc: null });
  } finally {
    closeOpened(t);
  }
});

test("acceptance 1 (d1): hosted-enqueued work is claimed natively and vice versa, one timeline", async () => {
  const t = openStore();
  const secondDoor = new D1QueueStore(new SqliteD1Client(t.db), { nowIso: () => iso(clock) });
  try {
    const enq = await enqueue(t.store, hostedEnv(), fixPacket());
    assert.equal(enq["ok"], true);
    const claimed = await claim(secondDoor, nativeEnv(), {});
    assert.equal(claimed["ok"], true);
    const task = claimed["row"] as JsonObject;
    assert.equal(task["status"], "claimed");

    const enq2 = await enqueue(secondDoor, nativeEnv(), fixPacket(4243));
    assert.equal(enq2["ok"], true);
    const claimed2 = await claim(t.store, hostedEnv(), {});
    assert.equal(claimed2["ok"], true);

    const kinds = (await t.store.allEvents()).map((e) => e.kind);
    assert.deepEqual(kinds.filter((k) => k === "claimed").length, 2);
  } finally {
    closeOpened(t);
  }
});

class RacingD1Store extends D1QueueStore {
  raced = false;
  override async conditionalUpdateTask(id: number, guard: Partial<TaskRow>, set: Partial<TaskRow>): Promise<boolean> {
    if (!this.raced && set.status === "claimed" && guard.status === "queued") {
      this.raced = true;
      await super.conditionalUpdateTask(
        id,
        { status: "queued" },
        { status: "claimed", worker_id: "racer", lease_until: "2999-01-01T00:00:00Z" },
      );
    }
    return super.conditionalUpdateTask(id, guard, set);
  }
}

test("acceptance 2 race at the seam (d1): a preempted conditional update re-picks and gets nothing", async () => {
  const t = openStore();
  const racing = new RacingD1Store(new SqliteD1Client(t.db));
  try {
    const enq = await enqueue(racing, hostedEnv(), fixPacket());
    assert.equal(enq["ok"], true);
    const res = await claim(racing, hostedEnv(), { worker: "door-one" });
    assert.equal(racing.raced, true);
    assert.equal(res["row"], null, "the losing door claims nothing");
    assert.equal((await racing.allTasks())[0]?.worker_id, "racer");
    const claimedEvents = (await racing.allEvents()).filter((e) => e.kind === "claimed");
    assert.equal(claimedEvents.length, 0);
  } finally {
    closeOpened(t);
  }
});

// ---------------------------------------------------------------------------
// The binding suite: the production adapter over the D1 API double.
// ---------------------------------------------------------------------------

test("binding: two doors over the D1 binding adapter enqueue, claim and complete on one DB", async () => {
  clock = T0;
  const dir = mkdtempSync(join(tmpdir(), "fleet-d1-binding-"));
  const db = new DatabaseSync(join(dir, "fleet-queue.db"));
  try {
    const doorA = openD1QueueStore(new FakeD1Database(db), { nowIso: () => iso(clock) });
    const doorB = openD1QueueStore(new FakeD1Database(db), { nowIso: () => iso(clock) });
    const enq = await enqueue(doorA, hostedEnv(), fixPacket(5150));
    assert.equal(enq["ok"], true);
    const claimed = await claim(doorB, nativeEnv(), {});
    assert.equal(claimed["ok"], true);
    const task = claimed["row"] as JsonObject;
    assert.equal(task["title"], "fix issue 5150");
    const done = await verb(doorB, nativeEnv(), "complete", {
      task_id: task["id"],
      worker: "worker-a",
      result: {
        evidence: "commit abc123",
        disposition: {
          checked_at: iso(clock),
          source: "test",
          issue: { number: 5150, state: "closed" },
          pulls: [],
          verdict: "closed_verified",
        },
      },
    });
    assert.equal(done["ok"], true);
    const seen = await doorA.allEvents();
    assert.deepEqual(
      seen.map((e) => e.kind).filter((k) => ["claimed", "completed"].includes(k)),
      ["claimed", "completed"],
    );
    assert.deepEqual(await doorA.describe(), { backend: "d1", projectSource: "fleet-queue", netloc: null });
  } finally {
    db.close();
    rmSync(dir, { recursive: true, force: true });
  }
});

test("interop: D1-written rows read identically through the embedded store on the same database", async () => {
  clock = T0;
  const dir = mkdtempSync(join(tmpdir(), "fleet-d1-interop-"));
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
  try {
    const d1 = new D1QueueStore(new SqliteD1Client(db), { nowIso: () => iso(clock) });
    const enq = await enqueue(d1, hostedEnv(), fixPacket(6160));
    assert.equal(enq["ok"], true);
    const d1Row = (await d1.allTasks()).find((r) => r.title === "fix issue 6160");
    assert.ok(d1Row);

    const embedded = new LibSQLQueueStore(sql, { nowIso: () => iso(clock) });
    const embeddedRow = await embedded.getTask(d1Row.id);
    assert.deepEqual(embeddedRow, d1Row);
    const enq2 = await enqueue(embedded, nativeEnv(), fixPacket(6161));
    assert.equal(enq2["ok"], true);

    const back = (await d1.allTasks()).find((r) => r.title === "fix issue 6161");
    assert.ok(back);
    assert.equal(back.status, "queued");
    assert.equal(back.dedupe_key, "repo-health:v1:fix:developing-today/code:issue:6161");
  } finally {
    db.close();
    rmSync(dir, { recursive: true, force: true });
  }
});

test("wiring: buildDeps selects the D1 store when the QUEUE_D1 binding is present, the fixture otherwise", async () => {
  const dir = mkdtempSync(join(tmpdir(), "fleet-d1-wiring-"));
  const db = new DatabaseSync(join(dir, "fleet-queue.db"));
  try {
    const withD1 = buildDeps({ QUEUE_D1: new FakeD1Database(db) } as HostedEnv);
    assert.deepEqual(await withD1.queueStore.describe(), { backend: "d1", projectSource: "fleet-queue", netloc: null });
    const without = buildDeps({} as HostedEnv);
    assert.equal((await without.queueStore.describe()).backend, "hosted-fixture");
  } finally {
    db.close();
    rmSync(dir, { recursive: true, force: true });
  }
});


// ---------------------------------------------------------------------------
// Real D1 (gated, #14870): the production D1DatabaseClient over a
// D1Database-shaped binding whose statements travel the D1 HTTP API
// to the deployed fleet-queue database — the same code path a
// deployed Worker runs, against the real database, before any
// cutover. The Hrana wire suite's gating pattern: with
// HOSTED_WORKER_D1_DATABASE_ID / CLOUDFLARE_ACCOUNT_ID /
// CLOUDFLARE_API_TOKEN unset this is a skip, never a failure. The
// suite works only in its own queue (d1-parity-smoke) and leaves
// its completed task there as the run's record.
// ---------------------------------------------------------------------------

interface RealD1Config {
  accountId: string;
  databaseId: string;
  token: string;
}

function realD1Config(): RealD1Config | null {
  const databaseId = process.env["HOSTED_WORKER_D1_DATABASE_ID"];
  const accountId = process.env["CLOUDFLARE_ACCOUNT_ID"];
  const token = process.env["CLOUDFLARE_API_TOKEN"];
  if (!databaseId || !accountId || !token) return null;
  return { accountId, databaseId, token };
}

class HttpD1PreparedStatement implements D1PreparedStatementLike {
  private values: unknown[] = [];

  constructor(
    private readonly cfg: RealD1Config,
    private readonly sqlText: string,
  ) {}

  bind(...values: unknown[]): D1PreparedStatementLike {
    this.values = values;
    return this;
  }

  private async query(): Promise<{ results: Record<string, unknown>[]; meta: D1MetaLike }> {
    const params = this.values.map((v) => {
      if (v === null || typeof v === "number" || typeof v === "string") return v;
      throw new Error(`real-D1 gate: unsupported bind value of type ${typeof v}`);
    });
    const res = await fetch(
      `https://api.cloudflare.com/client/v4/accounts/${this.cfg.accountId}/d1/database/${this.cfg.databaseId}/query`,
      {
        method: "POST",
        headers: { authorization: `Bearer ${this.cfg.token}`, "content-type": "application/json" },
        body: JSON.stringify({ sql: this.sqlText, params }),
      },
    );
    const body = (await res.json()) as {
      success: boolean;
      errors?: unknown;
      result?: { results: Record<string, unknown>[]; meta: D1MetaLike }[];
    };
    if (!body.success || !body.result || body.result.length === 0) {
      throw new Error(`real-D1 query failed: ${JSON.stringify(body.errors)}`);
    }
    return body.result[0] as { results: Record<string, unknown>[]; meta: D1MetaLike };
  }

  async all(): Promise<{ results: Record<string, unknown>[]; meta: D1MetaLike }> {
    const r = await this.query();
    return { results: r.results, meta: r.meta };
  }

  async run(): Promise<{ meta: D1MetaLike }> {
    const r = await this.query();
    return { meta: r.meta };
  }
}

class HttpD1Database implements D1DatabaseLike {
  constructor(private readonly cfg: RealD1Config) {}

  prepare(sqlText: string): D1PreparedStatementLike {
    return new HttpD1PreparedStatement(this.cfg, sqlText);
  }
}

test("real D1: two doors over the production binding adapter enqueue, claim and complete on the deployed database", async (t) => {
  const cfg = realD1Config();
  if (!cfg) {
    t.skip("no real D1 configured: HOSTED_WORKER_D1_DATABASE_ID / CLOUDFLARE_ACCOUNT_ID / CLOUDFLARE_API_TOKEN unset");
    return;
  }
  clock = T0;
  const queue = "d1-parity-smoke";
  // A run-unique synthetic issue keeps the dedupe key canonical and
  // the suite re-runnable against the shared database.
  const issue = 700000 + (Date.now() % 100000);
  const doorA = openD1QueueStore(new HttpD1Database(cfg), { nowIso: () => iso(clock) });
  const doorB = buildDeps({ QUEUE_D1: new HttpD1Database(cfg) } as HostedEnv).queueStore;

  const enq = await executeQueueVerb(doorA, "enqueue", { queue, packet: fixPacket(issue) }, hostedEnv());
  assert.equal(enq["ok"], true);
  const claimed = await executeQueueVerb(doorB, "claim", { queue, worker: "worker-a" }, nativeEnv());
  assert.equal(claimed["ok"], true);
  const task = claimed["row"] as JsonObject;
  assert.equal(task["title"], `fix issue ${issue}`);
  const done = await verb(doorB, nativeEnv(), "complete", {
    task_id: task["id"],
    worker: "worker-a",
    result: {
      evidence: "real-D1 parity run (#14870)",
      disposition: {
        checked_at: iso(clock),
        source: "test",
        issue: { number: issue, state: "closed" },
        pulls: [],
        verdict: "closed_verified",
      },
    },
  });
  assert.equal(done["ok"], true);
  const kinds = (await doorA.allEvents())
    .filter((e) => e.task_id === task["id"])
    .map((e) => e.kind)
    .filter((k) => ["claimed", "completed"].includes(k));
  assert.deepEqual(kinds, ["claimed", "completed"]);
  assert.deepEqual(await doorA.describe(), { backend: "d1", projectSource: "fleet-queue", netloc: null });
  const health = await verb(doorA, hostedEnv(), "health", {});
  assert.equal(health["backend"], "d1");
});
