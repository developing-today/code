// Bounded door hot paths (#14987 CF-2b): enqueue dedupe, lease
// expiry, and health must not load the full task/event tables —
// a production-shaped D1 (378 MB / ~23k rows) exceeds the 50 MB
// D1 response cap on SELECT * . Oracles: the bounded readers
// return the same rows the full scans would (identity collision,
// expired lease, health aggregates), and verbHealth answers from
// aggregates alone. Parity runs over Fixture, LibSQL, and D1
// (FakeD1Database double) stores.

import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { DatabaseSync } from "node:sqlite";
import { executeQueueVerb, LIVE_STATUSES, type QueueEnv } from "../src/queue.ts";
import { FixtureQueueStore, NO_DEP_PROBE, type QueueStore } from "../src/queue-store.ts";
import { LibSQLQueueStore, type SyncSqlDb } from "../src/libsql-queue-store.ts";
import { openD1QueueStore } from "../src/d1-queue-store.ts";
import { FakeD1Database } from "./d1-double.ts";
import { iso, type JsonObject } from "../src/queue-validate.ts";
import { BINDING_A } from "./helpers.ts";

const T0 = 1767225600; // 2026-01-01T00:00:00Z
let clock = T0;

function hostedEnv(): QueueEnv {
  return { nowEpoch: () => clock, depProbe: NO_DEP_PROBE, bindingId: BINDING_A };
}

function fixPacket(issue = 4242, payloadExtra: JsonObject = {}, packetExtra: JsonObject = {}): JsonObject {
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

function triagePacket(issue = 777): JsonObject {
  const p = fixPacket(issue);
  p["dedupe_key"] = `repo-health:v1:triage:developing-today/code:issue:${issue}`;
  (p["payload"] as JsonObject)["kind"] = "triage";
  (p["payload"] as JsonObject)["timebox_minutes"] = 20;
  return p;
}

function openLibSQL(): { store: QueueStore; db: DatabaseSync; dir: string } {
  const dir = mkdtempSync(join(tmpdir(), "fleet-hotpath-libsql-"));
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
  return { store: new LibSQLQueueStore(sql, { nowIso: () => iso(clock) }), db, dir };
}

function openD1(): { store: QueueStore; db: DatabaseSync; dir: string } {
  const dir = mkdtempSync(join(tmpdir(), "fleet-hotpath-d1-"));
  const db = new DatabaseSync(join(dir, "fleet-queue.db"));
  const store = openD1QueueStore(new FakeD1Database(db) as never, { nowIso: () => iso(clock) });
  return { store, db, dir };
}

async function scenario(store: QueueStore, label: string): Promise<void> {
  clock = T0;
  const env = hostedEnv();

  // Two live rows of different kinds.
  const enq1 = await executeQueueVerb(store, "enqueue", { queue: "repo-health", packet: fixPacket(101) }, env);
  assert.equal(enq1["ok"], true, `${label}: first enqueue ok`);
  const id1 = (enq1["row"] as JsonObject)["id"] as number;
  const enq2 = await executeQueueVerb(store, "enqueue", { queue: "repo-health", packet: triagePacket(202) }, env);
  assert.equal(enq2["ok"], true, `${label}: triage enqueue ok`);

  // Bounded identity scan: same issue, different dedupe_key, no
  // override — must collide with the live small_fix row (the path
  // that used to scan allTasks and blow the D1 cap).
  const dup = fixPacket(
    101,
    {},
    { dedupe_key: "repo-health:v1:fix:developing-today/code:issue:101:dispatch:abcdef12" },
  );
  const enqDup = await executeQueueVerb(store, "enqueue", { queue: "repo-health", packet: dup }, env);
  assert.equal(enqDup["ok"], true, `${label}: duplicate enqueue ok`);
  assert.equal((enqDup["row"] as JsonObject)["id"] as number, id1, `${label}: duplicate collides`);
  assert.ok((enqDup as JsonObject)["work_identity_collision"], `${label}: collision reported`);

  // liveIdentityCandidates: kind-filtered, live-status only.
  const smallFix = await store.liveIdentityCandidates("small_fix", [...LIVE_STATUSES]);
  assert.ok(
    smallFix.some((t) => t.id === id1),
    `${label}: small_fix candidate present`,
  );
  assert.ok(
    smallFix.every((t) => (t.payload as JsonObject)["kind"] === "small_fix"),
    `${label}: kind filter`,
  );
  const triage = await store.liveIdentityCandidates("triage", [...LIVE_STATUSES]);
  assert.equal(triage.length, 1, `${label}: one triage candidate`);
  const none = await store.liveIdentityCandidates("prompt", [...LIVE_STATUSES]);
  assert.equal(none.length, 0, `${label}: no prompt candidates`);
  const terminalOnly = await store.liveIdentityCandidates("small_fix", ["done", "cancelled", "failed"]);
  assert.equal(terminalOnly.length, 0, `${label}: no live rows in terminal statuses`);

  // Lease expiry: claim, lapse the lease, bounded scan finds it.
  const claimed = await executeQueueVerb(store, "claim", { worker: "worker-a", queue: "repo-health" }, env);
  assert.equal(claimed["ok"], true, `${label}: claim ok`);
  const claimedId = (claimed["row"] as JsonObject)["id"] as number;
  const fresh = await store.claimedExpired(iso(clock));
  assert.equal(fresh.length, 0, `${label}: nothing expired yet`);
  clock += 7200; // +2h past the default lease
  const expired = await store.claimedExpired(iso(clock));
  assert.ok(
    expired.some((t) => t.id === claimedId),
    `${label}: expired lease found bounded`,
  );

  // Health aggregates: bounded snapshot matches the verb response.
  await store.appendEvent("worker-a", claimedId, "completed", {}, iso(clock));
  const health = await executeQueueVerb(store, "health", { queue: "repo-health" }, env);
  assert.equal(health["ok"], true, `${label}: health ok`);
  const rows = health["rows"] as Array<Record<string, unknown>>;
  assert.equal(rows.length, 1, `${label}: one queue row`);
  assert.equal(rows[0]["queue"], "repo-health");
  assert.ok((rows[0]["total"] as number) >= 2, `${label}: total counts tasks`);
  assert.equal(rows[0]["terminal_events_7d"], 1, `${label}: terminal 7d counts the completion`);
  assert.equal(rows[0]["terminal_events_24h"], 1, `${label}: terminal 24h counts the completion`);

  const snap = await store.healthSnapshot("repo-health", iso(clock - 24 * 3600), iso(clock - 7 * 86400));
  const queued = snap.statusCounts.find((r) => r.status === "queued");
  assert.ok(queued && queued.count >= 1, `${label}: snapshot has queued count`);
  assert.equal(
    snap.terminal7.reduce((n, r) => n + r.count, 0),
    1,
    `${label}: snapshot terminal7`,
  );
  assert.equal(
    snap.terminal24.reduce((n, r) => n + r.count, 0),
    1,
    `${label}: snapshot terminal24`,
  );
}

test("bounded hot paths match over the fixture store", async () => {
  await scenario(new FixtureQueueStore(), "fixture");
});

test("bounded hot paths match over the LibSQL store", async () => {
  const opened = openLibSQL();
  try {
    await scenario(opened.store, "libsql");
  } finally {
    opened.db.close();
    rmSync(opened.dir, { recursive: true, force: true });
  }
});

test("bounded hot paths match over the D1 store", async () => {
  const opened = openD1();
  try {
    await scenario(opened.store, "d1");
  } finally {
    opened.db.close();
    rmSync(opened.dir, { recursive: true, force: true });
  }
});
