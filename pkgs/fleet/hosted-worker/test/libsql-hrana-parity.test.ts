// Parity for the remote queue store (#14592): HranaQueueStore —
// the async Hrana opener over a remote Turso DB — against the
// acceptance scenarios libsql-queue-parity.test.ts pins for the
// embedded store, plus the two proofs only this door can give:
// the wire transport itself (real HTTP against a real sqld /
// `turso dev` endpoint) and cross-door file interop (rows written
// over Hrana read back identically through the embedded store on
// the same file, and vice versa — one schema, one queue, two
// transports).
//
// Two suites:
//  - Transport-shaped parity (always runs): HranaQueueStore over
//    an in-process HranaClient backed by node:sqlite — the same
//    seam the production store injects, with the fake peer
//    reporting affected-row counts and insert ids the way sqld
//    does. Proves the store's SQL, codecs and claim primitive.
//  - Wire parity (endpoint-gated): the real HranaHttpClient
//    against HOSTED_WORKER_HRANA_URL when set, else a spawned
//    `turso dev` on a temp file when the turso CLI is on PATH,
//    else skipped with the reason named. Never a mocked fetch:
//    the wire format is the thing under test.

import assert from "node:assert/strict";
import { test } from "node:test";
import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { DatabaseSync } from "node:sqlite";
import { executeQueueVerb, nativeQueueEnv, type QueueEnv } from "../src/queue.ts";
import { NO_DEP_PROBE, type QueueStore, type TaskRow } from "../src/queue-store.ts";
import { LibSQLQueueStore, type SyncSqlDb } from "../src/libsql-queue-store.ts";
import {
  HranaQueueStore,
  openHranaQueueStore,
  type HranaClient,
  type HranaResult,
  type HranaStatement,
} from "../src/libsql-hrana-queue-store.ts";
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
// The in-process Hrana peer: node:sqlite behind the HranaClient
// seam, reporting sqld-shaped results.
// ---------------------------------------------------------------------------

class SqliteHranaClient implements HranaClient {
  constructor(private readonly db: DatabaseSync) {}

  async execute(stmt: HranaStatement): Promise<HranaResult> {
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

function openPeerStore(): { store: HranaQueueStore; db: DatabaseSync; dir: string } {
  clock = T0;
  const dir = mkdtempSync(join(tmpdir(), "fleet-hrana-parity-"));
  const db = new DatabaseSync(join(dir, "fleet-queue.db"));
  const store = new HranaQueueStore(new SqliteHranaClient(db), { nowIso: () => iso(clock) });
  return { store, db, dir };
}

// ---------------------------------------------------------------------------
// Store surface + parity scenarios over the injected transport
// ---------------------------------------------------------------------------

test("hrana store: the full QueueStore surface round-trips over the transport seam", async () => {
  const t = openPeerStore();
  try {
    const row = await t.store.insertTask({
      queue: "repo-health",
      title: "fix issue 4242",
      body: "b",
      payload: fixPacket().payload,
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
    assert.deepEqual(await t.store.describe(), { backend: "libsql", projectSource: "fleet-queue", netloc: null });
  } finally {
    t.db.close();
    rmSync(t.dir, { recursive: true, force: true });
  }
});

test("acceptance 1 (hrana): hosted-enqueued work is claimed natively and vice versa, one timeline", async () => {
  const t = openPeerStore();
  const secondDoor = new HranaQueueStore(new SqliteHranaClient(t.db), { nowIso: () => iso(clock) });
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
    t.db.close();
    rmSync(t.dir, { recursive: true, force: true });
  }
});

class RacingHranaStore extends HranaQueueStore {
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

test("acceptance 2 race at the seam (hrana): a preempted conditional update re-picks and gets nothing", async () => {
  const t = openPeerStore();
  const racing = new RacingHranaStore(new SqliteHranaClient(t.db));
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
    t.db.close();
    rmSync(t.dir, { recursive: true, force: true });
  }
});

// ---------------------------------------------------------------------------
// Wire parity: the real HTTP transport against a real endpoint.
// ---------------------------------------------------------------------------

interface WireEndpoint {
  url: string;
  file: string | null;
  stop: () => void;
  start: () => Promise<void>;
}

let wire: WireEndpoint | null | undefined;

async function wireEndpoint(): Promise<WireEndpoint | null> {
  if (wire !== undefined) return wire;
  const envUrl = process.env["HOSTED_WORKER_HRANA_URL"];
  if (envUrl) {
    wire = { url: envUrl, file: null, stop: () => {}, start: async () => {} };
    return wire;
  }
  if (spawnSync("turso", ["--version"], { stdio: "ignore" }).status !== 0) {
    wire = null;
    return wire;
  }
  const dir = mkdtempSync(join(tmpdir(), "fleet-hrana-wire-"));
  const file = join(dir, "fleet-queue.db");
  const port = 18700 + (process.pid % 200);
  const url = `http://127.0.0.1:${port}`;
  let child: ChildProcess | null = null;
  const start = async (): Promise<void> => {
    child = spawn("turso", ["dev", "--db-file", file, "--port", String(port)], { stdio: "ignore" });
    const deadline = Date.now() + 20000;
    for (;;) {
      if (child.exitCode !== null) throw new Error("turso dev exited during startup");
      try {
        const res = await fetch(`${url}/v2/pipeline`, {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({
            requests: [{ type: "execute", stmt: { sql: "SELECT 1", args: [] } }, { type: "close" }],
          }),
        });
        if (res.ok) return;
      } catch {
        // not up yet
      }
      if (Date.now() > deadline) throw new Error("turso dev did not become ready");
      await new Promise((r) => setTimeout(r, 200));
    }
  };
  const stop = (): void => {
    child?.kill("SIGTERM");
    child = null;
  };
  try {
    await start();
  } catch {
    // turso dev is present but its server cannot start here (e.g.
    // sqld missing from PATH): the endpoint is unavailable, which
    // is a skip, not a failure.
    stop();
    wire = null;
    return wire;
  }
  wire = { url, file, stop, start };
  process.on("exit", () => {
    stop();
    rmSync(dir, { recursive: true, force: true });
  });
  return wire;
}

test("wire: two doors over real Hrana HTTP enqueue, claim and complete on one DB", async (t) => {
  const ep = await wireEndpoint();
  if (!ep) {
    t.skip("no Hrana endpoint: HOSTED_WORKER_HRANA_URL unset and no local turso dev / sqld server available");
    return;
  }
  clock = T0;
  const doorA = openHranaQueueStore(ep.url);
  const doorB = openHranaQueueStore(ep.url);
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
  assert.deepEqual(await doorA.describe(), {
    backend: "libsql",
    projectSource: "fleet-queue",
    netloc: new URL(ep.url).host,
  });
});

test("wire: cross-door file interop — Hrana-written rows read identically through the embedded store", async (t) => {
  const ep = await wireEndpoint();
  if (!ep) {
    t.skip("no Hrana endpoint: HOSTED_WORKER_HRANA_URL unset and no local turso dev / sqld server available");
    return;
  }
  if (!ep.file) {
    t.skip("cross-door file interop needs the spawned turso dev file (HOSTED_WORKER_HRANA_URL points elsewhere)");
    return;
  }
  clock = T0;
  const hrana = openHranaQueueStore(ep.url);
  const enq = await enqueue(hrana, hostedEnv(), fixPacket(6160));
  assert.equal(enq["ok"], true);
  const hranaRow = (await hrana.allTasks()).find((r) => r.title === "fix issue 6160");
  assert.ok(hranaRow);

  // Stop the server; the embedded store opens the same file and
  // must read the Hrana-written row field-for-field.
  ep.stop();
  await new Promise((r) => setTimeout(r, 300));
  const db = new DatabaseSync(ep.file);
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
  const embedded = new LibSQLQueueStore(sql, { nowIso: () => iso(clock) });
  try {
    const embeddedRow = await embedded.getTask(hranaRow.id);
    assert.deepEqual(embeddedRow, hranaRow);
    const enq2 = await enqueue(embedded, nativeEnv(), fixPacket(6161));
    assert.equal(enq2["ok"], true);
  } finally {
    db.close();
  }

  // Restart the server on the same file: the embedded-written row
  // reads back identically over the wire.
  await ep.start();
  const back = (await hrana.allTasks()).find((r) => r.title === "fix issue 6161");
  assert.ok(back);
  assert.equal(back.status, "queued");
  assert.equal(back.dedupe_key, "repo-health:v1:fix:developing-today/code:issue:6161");
});
