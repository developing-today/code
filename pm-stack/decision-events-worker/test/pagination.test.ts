// Read-back pagination regression (#9614): paging through a group of
// events that share one timestamp must return every event exactly
// once. Exercises the real Worker fetch handler and its real SQL
// against an in-memory SQLite database standing in for D1 (D1 is
// SQLite-backed; the fake runs the handler's actual query strings).
import { Database } from "bun:sqlite";
import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import worker from "../src/index.ts";

const SECRET = "test-secret";
const TS = "2026-11-12T10:00:00.000Z";

class FakeStatement {
  constructor(
    private db: Database,
    private sql: string,
    private args: unknown[] = [],
  ) {}
  bind(...args: unknown[]): FakeStatement {
    return new FakeStatement(this.db, this.sql, args);
  }
  async all<T>(): Promise<{ results: T[] }> {
    return { results: this.db.query(this.sql).all(...(this.args as never[])) as T[] };
  }
  async first<T>(): Promise<T | null> {
    return (this.db.query(this.sql).get(...(this.args as never[])) as T) ?? null;
  }
}

class FakeD1 {
  constructor(private db: Database) {}
  prepare(sql: string): FakeStatement {
    return new FakeStatement(this.db, sql);
  }
}

function makeEnv(rows: [string, string, string][]): unknown {
  const db = new Database(":memory:");
  const schema = readFileSync(join(import.meta.dir, "..", "schema.sql"), "utf8");
  db.exec(schema);
  const insert = db.query(
    "INSERT INTO decision_set_events (event_id, set_id, question_ref, type, payload, ts) VALUES (?, ?, ?, ?, ?, ?)",
  );
  // Inserted out of order on purpose: ordering is the query's job.
  for (const [eventId, setId, ts] of rows) {
    insert.run(eventId, setId, "ledger-1#1", "question-answered", JSON.stringify({ optionId: "a" }), ts);
  }
  return { DB: new FakeD1(db), SET_EVENTS: {}, WEBHOOK_SECRET: SECRET };
}

async function getEvents(
  env: unknown,
  query: string,
): Promise<{
  status: number;
  body: { count?: number; events?: { eventId: string }[]; nextAfter?: string | null; error?: string };
}> {
  const res = await worker.fetch(
    new Request(`http://worker.test/sets/set-1/events?${query}`, {
      headers: { Authorization: `Bearer ${SECRET}` },
    }),
    env as never,
  );
  return { status: res.status, body: await res.json() };
}

const ROWS: [string, string, string][] = [
  ["e3", "set-1", TS],
  ["e1", "set-1", TS],
  ["e5", "set-1", "2026-11-12T11:00:00.000Z"],
  ["e2", "set-1", TS],
  ["e4", "set-1", "2026-11-12T11:00:00.000Z"],
  ["other-set-event", "set-2", TS],
];

describe("GET /sets/<setId>/events pagination (#9614)", () => {
  test("pages through a same-timestamp group: every event exactly once, in (ts, event_id) order", async () => {
    const env = makeEnv(ROWS);
    const seen: string[] = [];
    let after: string | null = null;
    for (let page = 0; page < 10; page++) {
      const q = after === null ? "limit=2" : `limit=2&after=${encodeURIComponent(after)}`;
      const { status, body } = await getEvents(env, q);
      expect(status).toBe(200);
      const ids = (body.events ?? []).map((e) => e.eventId);
      if (ids.length === 0) {
        expect(body.nextAfter).toBeNull();
        break;
      }
      seen.push(...ids);
      after = body.nextAfter ?? null;
      expect(after).not.toBeNull();
      // A page that ends inside the same-ts group must hand back the
      // composite cursor, not the bare timestamp.
      if (page === 0) expect(after).toBe(`${TS}|e2`);
    }
    expect(seen).toEqual(["e1", "e2", "e3", "e4", "e5"]);
    expect(new Set(seen).size).toBe(seen.length);
  });

  test("legacy bare-ts after keeps its original meaning (strictly after that ts)", async () => {
    const env = makeEnv(ROWS);
    const { status, body } = await getEvents(env, `limit=10&after=${encodeURIComponent(TS)}`);
    expect(status).toBe(200);
    expect((body.events ?? []).map((e) => e.eventId)).toEqual(["e4", "e5"]);
  });

  test("a malformed after cursor is a 400, not a silent first page", async () => {
    const env = makeEnv(ROWS);
    for (const bad of ["not-a-date", `${TS}|`, `not-a-date|e1`, `${TS}|has space`]) {
      const { status } = await getEvents(env, `after=${encodeURIComponent(bad)}`);
      expect(status).toBe(400);
    }
  });
});
