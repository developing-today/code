import { test } from "node:test";
import assert from "node:assert/strict";
import { DatabaseSync } from "node:sqlite";
import { SYNC_COORD_SCHEMA, SyncCoord, SyncCoordConflict, type SyncWatermark } from "../src/sync-coord.ts";
import type { SqlStorageLike } from "../src/bridge-store.ts";

function makeSql(): SqlStorageLike {
  const db = new DatabaseSync(":memory:");
  for (const statement of SYNC_COORD_SCHEMA) db.exec(statement);
  return {
    exec(query, ...params) {
      const statement = db.prepare(query);
      if (/^\s*select/i.test(query)) {
        const rows = statement.all(...(params as never[])) as Record<string, unknown>[];
        return { toArray: () => rows };
      }
      statement.run(...(params as never[]));
      return { toArray: () => [] };
    },
  };
}

const initial: SyncWatermark[] = [{ author: "author-a", next_seq: 3, head: "event-2" }];

test("SyncCoord owns a tenant/peer session and durably advances its watermark", () => {
  let now = 1000;
  const sql = makeSql();
  const first = new SyncCoord(sql, "tenant-a", () => now);
  const owned = first.own("peer-a", "session-1", initial);
  assert.equal(owned.tenant_id, "tenant-a");
  assert.equal(owned.state, "active");
  assert.deepEqual(owned.watermark, initial);

  assert.throws(() => first.own("peer-a", "session-2", []), SyncCoordConflict);

  now += 10;
  const advanced = first.advance("peer-a", "session-1", [
    { author: "author-a", next_seq: 4, head: "event-3" },
  ]);
  assert.deepEqual(advanced.watermark, [{ author: "author-a", next_seq: 4, head: "event-3" }]);
  first.end("peer-a", "session-1");

  // A new coordinator over the same tenant database observes the durable
  // resume point, and only the ended session may release its ownership.
  const restarted = new SyncCoord(sql, "tenant-a", () => now);
  const saved = restarted.get("peer-a");
  assert.equal(saved?.state, "idle");
  assert.deepEqual(saved?.watermark, [{ author: "author-a", next_seq: 4, head: "event-3" }]);
  assert.throws(() => restarted.advance("peer-a", "session-1", []), SyncCoordConflict);
  assert.equal(restarted.own("peer-a", "session-2", []).state, "active");
});

test("SyncCoord expires abandoned live ownership without losing its watermark", () => {
  let now = 1000;
  const coord = new SyncCoord(makeSql(), "tenant-a", () => now);
  coord.own("peer-a", "abandoned", initial);
  now += 24 * 60 * 60 * 1000 + 1;
  const replacement = coord.own("peer-a", "recovered", []);
  assert.equal(replacement.session_id, "recovered");
  assert.deepEqual(replacement.watermark, initial, "expired ownership does not discard the resume watermark");
});
