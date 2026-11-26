// D1 session registry tests (#12584's production wiring). Oracles:
// the issue's acceptance read against a database that outlives the
// registry instance (open, drop the instance as an isolate would,
// and a fresh registry over the same D1 database still resolves the
// session; a closed or unknown id is refused with its named error;
// the record read back shows account, lane, model and lane-start
// issues as opened), the fixture's storage contract (src/sessions.ts:
// duplicate ids refused, unknown ids null, copies out), and the
// wiring: buildDeps resolves this registry when the QUEUE_D1 binding
// is present and the fixture otherwise. The D1 binding is the same
// faithful in-process double over a real SQLite file that
// d1-queue-parity.test.ts uses for D1QueueStore, so the production
// D1DatabaseClient is the code under test, never a mocked SQL layer.

import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { DatabaseSync } from "node:sqlite";
import type { AuthDeps } from "../src/auth.ts";
import type { HostedEnv } from "../src/config.ts";
import type { D1DatabaseLike } from "../src/d1-queue-store.ts";
import { D1SessionRegistry, openD1SessionRegistry, readSessionRow } from "../src/d1-session-registry.ts";
import { dispatch } from "../src/router.ts";
import { FixtureSessionRegistry, type SessionRecord } from "../src/sessions.ts";
import { buildDeps } from "../src/worker.ts";
import { BINDING_A, bearerReq, makeHarness } from "./helpers.ts";

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
  constructor(readonly db: DatabaseSync) {}

  prepare(sqlText: string): FakeD1PreparedStatement {
    return new FakeD1PreparedStatement(this.db, sqlText);
  }
}

function withDb(fn: (d1: FakeD1Database) => Promise<void>): () => Promise<void> {
  return async () => {
    const dir = mkdtempSync(join(tmpdir(), "fleet-d1-sessions-"));
    const db = new DatabaseSync(join(dir, "fleet-hosted.db"));
    try {
      await fn(new FakeD1Database(db));
    } finally {
      db.close();
      rmSync(dir, { recursive: true, force: true });
    }
  };
}

function record(overrides: Partial<SessionRecord> = {}): SessionRecord {
  return {
    sessionId: "ses_0123456789abcdef0123456789abcdef",
    bindingId: BINDING_A,
    lane: "code-a",
    model: "model-x",
    openedAt: "2026-11-26T08:00:00.000Z",
    closedAt: null,
    laneStartIssues: ["developing-today/code#12584", "developing-today/code#9081"],
    ...overrides,
  };
}

test(
  "storage contract: the D1 registry answers exactly as the fixture does",
  withDb(async (d1) => {
    const registries = [new FixtureSessionRegistry(), openD1SessionRegistry(d1)];
    for (const registry of registries) {
      const name = registry.constructor.name;
      assert.equal(await registry.get("ses_absent"), null, name);
      assert.equal(await registry.markClosed("ses_absent", "2026-11-26T09:00:00.000Z"), null, name);

      await registry.insert(record());
      await registry.insert(record({ sessionId: "ses_bare", lane: null, model: null, laneStartIssues: [] }));
      assert.deepEqual(await registry.get(record().sessionId), record(), name);
      assert.deepEqual(
        await registry.get("ses_bare"),
        record({ sessionId: "ses_bare", lane: null, model: null, laneStartIssues: [] }),
        name,
      );
      await assert.rejects(() => registry.insert(record({ lane: "other" })), /duplicate session id/, name);
      assert.equal((await registry.get(record().sessionId))?.lane, "code-a", name);

      const copy = await registry.get(record().sessionId);
      copy!.laneStartIssues.push("mutated");
      assert.deepEqual((await registry.get(record().sessionId))?.laneStartIssues, record().laneStartIssues, name);

      const closed = await registry.markClosed(record().sessionId, "2026-11-26T09:00:00.000Z");
      assert.deepEqual(closed, record({ closedAt: "2026-11-26T09:00:00.000Z" }), name);
      assert.equal((await registry.get("ses_bare"))?.closedAt, null, name);
    }
  }),
);

test(
  "records outlive the registry instance: a fresh registry over the same database reads them back",
  withDb(async (d1) => {
    await openD1SessionRegistry(d1).insert(record());
    const later = openD1SessionRegistry(d1);
    assert.deepEqual(await later.get(record().sessionId), record());
    await later.markClosed(record().sessionId, "2026-11-26T09:30:00.000Z");
    assert.equal((await openD1SessionRegistry(d1).get(record().sessionId))?.closedAt, "2026-11-26T09:30:00.000Z");
  }),
);

test(
  "a row the registry did not write fails closed instead of resolving",
  withDb(async (d1) => {
    const registry = openD1SessionRegistry(d1);
    await registry.get("ses_warmup");
    d1.db
      .prepare("INSERT INTO fleet_sessions (session_id, binding_id, opened_at, lane_start_issues) VALUES (?, ?, ?, ?)")
      .run("ses_forged", BINDING_A, "2026-11-26T08:00:00.000Z", '{"not":"a list"}');
    await assert.rejects(() => registry.get("ses_forged"), /lane_start_issues is not a JSON array of strings/);
    assert.throws(
      () => readSessionRow({ ...rowOf(record()), binding_id: 7 }),
      /column binding_id is number, expected text or null/,
    );
    assert.throws(() => readSessionRow({ ...rowOf(record()), opened_at: null }), /column opened_at is null/);
  }),
);

function rowOf(r: SessionRecord): Record<string, unknown> {
  return {
    session_id: r.sessionId,
    binding_id: r.bindingId,
    lane: r.lane,
    model: r.model,
    opened_at: r.openedAt,
    closed_at: r.closedAt,
    lane_start_issues: JSON.stringify(r.laneStartIssues),
  };
}

test(
  "wiring: buildDeps resolves the D1 registry when QUEUE_D1 is bound, the fixture otherwise",
  withDb(async (d1) => {
    assert.ok(buildDeps({ QUEUE_D1: d1 } as HostedEnv).sessions instanceof D1SessionRegistry);
    assert.ok(buildDeps({} as HostedEnv).sessions instanceof FixtureSessionRegistry);
  }),
);

test(
  "acceptance over D1: open, a new isolate resolves the same session; after close it is refused as session_closed",
  withDb(async (d1) => {
    const h = await makeHarness();
    const token = await h.sign(h.rpPayload());
    const depsFor = (): AuthDeps => ({ ...h.deps, sessions: openD1SessionRegistry(d1) });
    const post = (path: string, body: unknown): Promise<Response> =>
      dispatch(
        bearerReq(path, token, {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify(body),
        }),
        depsFor(),
      );

    const openRes = await post("/v1/sessions", {
      lane: "code-a",
      model: "model-x",
      lane_start_issues: ["developing-today/code#12584"],
    });
    assert.equal(openRes.status, 201);
    const opened = (await openRes.json()) as Record<string, unknown>;
    assert.equal(opened["binding_id"], BINDING_A);

    const resolved = await post("/v1/sessions/resolve", { session_id: opened["session_id"] });
    assert.equal(resolved.status, 200);
    assert.deepEqual(await resolved.json(), opened);

    const unknown = await post("/v1/sessions/resolve", { session_id: "ses_neverminted" });
    assert.equal(unknown.status, 404);
    assert.equal(((await unknown.json()) as { error: string }).error, "session_unknown");

    assert.equal((await post("/v1/sessions/close", { session_id: opened["session_id"] })).status, 200);
    const stale = await post("/v1/sessions/resolve", { session_id: opened["session_id"] });
    assert.equal(stale.status, 409);
    assert.equal(((await stale.json()) as { error: string }).error, "session_closed");
  }),
);
