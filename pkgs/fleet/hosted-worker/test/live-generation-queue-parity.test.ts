// Live-generation parity (#14985, CF-2a of #14942): the queue
// stores against a database carrying the LIVE schema generation —
// the shape the deployed fleet-queue database (and any fresh
// export of the live Turso plane) actually has, where tasks lacks
// manual_only and metadata and defaults max_attempts to 4. The
// other parity suites only ever build databases from the current
// shared DDL, so the generation gap this fixture pins could
// silently reopen there; here each store must first migrate the
// export-shaped table at open (its owning schema path's additive
// ALTERs) and then run the full verb surface — enqueue, claim,
// complete — over it, including claiming and completing a task row
// the export itself carried in.
//
// Oracle: against the fixture database, (a) the pre-open shape is
// the live generation (no manual_only / metadata, max_attempts
// DEFAULT 4 — the fixture cannot drift into the new shape without
// failing here), (b) after the store opens, both columns exist and
// the native-only columns (labels, resolution) still do not — the
// TS stores do not mint native-owned columns, (c) the exported row
// reads back with its stored values untouched (max_attempts stays
// 4: stored data is never rewritten by the migration), (d) a task
// enqueued through the store carries max_attempts 1000 — the store
// supplies it explicitly, so behaviour is aligned even though the
// table default is not, and (e) both the exported row and the new
// row claim and complete through the verb engine. The fixture and
// the determinations behind (b)–(d) live in
// queue-live-generation-fixture.ts.

import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { DatabaseSync } from "node:sqlite";
import { executeQueueVerb, nativeQueueEnv, type QueueEnv } from "../src/queue.ts";
import { NO_DEP_PROBE, type QueueStore } from "../src/queue-store.ts";
import { LibSQLQueueStore, type SyncSqlDb } from "../src/libsql-queue-store.ts";
import { D1QueueStore, type D1Client, type D1Result, type D1Statement } from "../src/d1-queue-store.ts";
import { HranaQueueStore, type HranaClient, type HranaResult, type HranaStatement } from "../src/libsql-hrana-queue-store.ts";
import { iso, type JsonObject } from "../src/queue-validate.ts";
import { BINDING_A } from "./helpers.ts";
import { LIVE_GENERATION_QUEUE_SCHEMA } from "./queue-live-generation-fixture.ts";

const T0 = 1767225600; // 2026-01-01T00:00:00Z
let clock = T0;

function hostedEnv(): QueueEnv {
  return { nowEpoch: () => clock, depProbe: NO_DEP_PROBE, bindingId: BINDING_A };
}

function nativeEnv(): QueueEnv {
  return nativeQueueEnv(() => clock);
}

function fixPacket(issue = 4242): JsonObject {
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
    },
  };
}

// ---------------------------------------------------------------------------
// Transport doubles over node:sqlite — the same seams the per-store
// parity suites inject (statement outcome carries changes /
// lastInsertRowid, never connection-scoped state).
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

function syncSqlDb(db: DatabaseSync): SyncSqlDb {
  return {
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
}

// ---------------------------------------------------------------------------
// The scenario: one export-shaped database, one store, the full
// verb surface. Shared by the three store tests below.
// ---------------------------------------------------------------------------

function taskColumns(db: DatabaseSync): Map<string, unknown> {
  const rows = db.prepare("SELECT name, dflt_value FROM pragma_table_info('tasks')").all() as {
    name: string;
    dflt_value: unknown;
  }[];
  return new Map(rows.map((r) => [r.name, r.dflt_value]));
}

async function runLiveGenerationScenario(open: (db: DatabaseSync) => QueueStore): Promise<void> {
  clock = T0;
  const dir = mkdtempSync(join(tmpdir(), "fleet-live-generation-"));
  const db = new DatabaseSync(join(dir, "fleet-queue.db"));
  try {
    for (const stmt of LIVE_GENERATION_QUEUE_SCHEMA) {
      db.prepare(stmt).run();
    }

    // (a) The fixture really is the live generation.
    const before = taskColumns(db);
    assert.equal(before.has("manual_only"), false, "fixture tasks must lack manual_only before the store opens");
    assert.equal(before.has("metadata"), false, "fixture tasks must lack metadata before the store opens");
    assert.equal(String(before.get("max_attempts")), "4", "fixture tasks must default max_attempts to 4");

    // A task row the export carried in, written with only the
    // live-generation columns and max_attempts left to the table
    // default — the row shape the cutover inherits.
    const exportedPacket = fixPacket(31337);
    const seeded = db
      .prepare(
        `INSERT INTO tasks (queue, title, body, payload, dedupe_key, priority, status,
           progress, result, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, 'queued', '{}', '{}', ?, ?)`,
      )
      .run(
        "repo-health",
        "fix issue 31337",
        "exported from the live plane",
        JSON.stringify(exportedPacket["payload"]),
        exportedPacket["dedupe_key"] as string,
        30,
        iso(clock),
        iso(clock),
      );
    const exportedId = Number(seeded.lastInsertRowid);

    const store = open(db);

    // First contact migrates: the exported row reads back through
    // the store (this call is what runs the async stores' schema
    // init; the embedded store migrated in its constructor).
    const exported = await store.getTask(exportedId);
    assert.ok(exported, "the exported row must read back through the store");
    // (c) Stored values untouched; the migrated columns read their
    // defaults.
    assert.equal(exported.manual_only, false);
    assert.deepEqual(exported.metadata, {});
    assert.equal(exported.max_attempts, 4, "the migration must not rewrite the exported row's stored max_attempts");

    // (b) The migration landed exactly the two TS-owned columns.
    const after = taskColumns(db);
    assert.equal(after.has("manual_only"), true, "the store must ALTER in manual_only at open");
    assert.equal(after.has("metadata"), true, "the store must ALTER in metadata at open");
    assert.equal(after.has("labels"), false, "TS stores must not mint the native-owned labels column");
    assert.equal(after.has("resolution"), false, "TS stores must not mint the native-owned resolution columns");

    // (d) Enqueue through the verb engine: the new row's
    // max_attempts is the store-supplied 1000, not the table's 4.
    const enq = await executeQueueVerb(store, "enqueue", { queue: "repo-health", packet: fixPacket() }, hostedEnv());
    assert.equal(enq["ok"], true);
    const fresh = (await store.allTasks()).find((r) => r.title === "fix issue 4242");
    assert.ok(fresh, "the enqueued task must be readable through the store");
    assert.equal(fresh.max_attempts, 1000, "a store-enqueued task carries max_attempts 1000 despite the table default 4");

    // (e) Claim both rows (the exported one and the new one) and
    // complete both through the verb engine.
    const claimedIds = new Set<number>();
    for (let i = 0; i < 2; i++) {
      const claimed = await executeQueueVerb(
        store,
        "claim",
        { queue: "repo-health", worker: "worker-a" },
        nativeEnv(),
      );
      assert.equal(claimed["ok"], true);
      const row = claimed["row"] as JsonObject;
      claimedIds.add(row["id"] as number);
    }
    assert.deepEqual(claimedIds, new Set([exportedId, fresh.id]));

    for (const [taskId, issue] of [
      [exportedId, 31337],
      [fresh.id, 4242],
    ] as const) {
      const done = await executeQueueVerb(
        store,
        "complete",
        {
          task_id: taskId,
          worker: "worker-a",
          result: {
            evidence: "live-generation parity (#14985)",
            disposition: {
              checked_at: iso(clock),
              source: "test",
              issue: { number: issue, state: "closed" },
              pulls: [],
              verdict: "closed_verified",
            },
          },
        },
        nativeEnv(),
      );
      assert.equal(done["ok"], true, `complete must succeed for task ${taskId}`);
      const row = await store.getTask(taskId);
      assert.equal(row?.status, "done");
      assert.notEqual(row?.completed_at, null);
      const kinds = (await store.eventsForTask(taskId)).map((e) => e.kind);
      assert.ok(kinds.includes("claimed"), `task ${taskId} must carry a claimed event`);
      assert.ok(kinds.includes("completed"), `task ${taskId} must carry a completed event`);
    }
  } finally {
    db.close();
    rmSync(dir, { recursive: true, force: true });
  }
}

test("live generation (d1): the D1 store migrates an export-shaped database at open and runs the full verb surface", async () => {
  await runLiveGenerationScenario((db) => new D1QueueStore(new SqliteD1Client(db), { nowIso: () => iso(clock) }));
});

test("live generation (libsql): the embedded store migrates an export-shaped database at open and runs the full verb surface", async () => {
  await runLiveGenerationScenario((db) => new LibSQLQueueStore(syncSqlDb(db), { nowIso: () => iso(clock) }));
});

test("live generation (hrana): the remote store migrates an export-shaped database at open and runs the full verb surface", async () => {
  await runLiveGenerationScenario((db) => new HranaQueueStore(new SqliteHranaClient(db), { nowIso: () => iso(clock) }));
});
