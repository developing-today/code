// GET /sets listing regression (#13491): the edge cannot read open
// questions from the decision-events Worker until the Worker can say
// which sets it holds events for. Exercises the real Worker fetch
// handler and its real SQL against an in-memory SQLite database
// standing in for D1 (D1 is SQLite-backed; the fake runs the
// handler's actual query strings), following test/pagination.test.ts.
import { Database } from "bun:sqlite";
import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import worker from "../src/index.ts";

const SECRET = "test-secret";

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

function makeEnv(rows: [string, string, string, string][]): unknown {
  const db = new Database(":memory:");
  const schema = readFileSync(join(import.meta.dir, "..", "schema.sql"), "utf8");
  db.exec(schema);
  const insert = db.query(
    "INSERT INTO decision_set_events (event_id, set_id, question_ref, type, payload, ts) VALUES (?, ?, ?, ?, ?, ?)",
  );
  for (const [eventId, setId, type, ts] of rows) {
    insert.run(eventId, setId, "ledger-1#1", type, JSON.stringify({}), ts);
  }
  return { DB: new FakeD1(db), SET_EVENTS: {}, WEBHOOK_SECRET: SECRET };
}

const get = (env: unknown, path: string, secret: string | null = SECRET) =>
  (worker.fetch as (request: Request, env: unknown) => Promise<Response>)(
    new Request(`https://worker${path}`, {
      headers: secret === null ? {} : { Authorization: `Bearer ${secret}` },
    }),
    env,
  );

describe("GET /sets", () => {
  test("an empty store lists no sets", async () => {
    const res = await get(makeEnv([]), "/sets");
    expect(res.status).toBe(200);
    expect(await res.json()).toEqual({ count: 0, sets: [] });
  });

  test("lists every set with its count and latest event, newest set first", async () => {
    const env = makeEnv([
      ["e1", "set-a", "set-raised", "2026-11-12T10:00:00.000Z"],
      ["e2", "set-b", "set-raised", "2026-11-12T11:00:00.000Z"],
      ["e3", "set-a", "question-answered", "2026-11-12T12:00:00.000Z"],
    ]);
    const res = await get(env, "/sets");
    expect(res.status).toBe(200);
    expect(await res.json()).toEqual({
      count: 2,
      sets: [
        {
          setId: "set-a",
          events: 2,
          firstTs: "2026-11-12T10:00:00.000Z",
          lastTs: "2026-11-12T12:00:00.000Z",
          lastEventId: "e3",
          lastType: "question-answered",
        },
        {
          setId: "set-b",
          events: 1,
          firstTs: "2026-11-12T11:00:00.000Z",
          lastTs: "2026-11-12T11:00:00.000Z",
          lastEventId: "e2",
          lastType: "set-raised",
        },
      ],
    });
  });

  test("the latest event wins ties on ts by event_id", async () => {
    const env = makeEnv([
      ["e-b", "set-a", "question-answered", "2026-11-12T10:00:00.000Z"],
      ["e-a", "set-a", "set-raised", "2026-11-12T10:00:00.000Z"],
    ]);
    const res = await get(env, "/sets");
    expect(res.status).toBe(200);
    const body = (await res.json()) as {
      sets: { setId: string; lastEventId: string; lastType: string }[];
    };
    expect(body.sets).toEqual([
      {
        setId: "set-a",
        events: 2,
        firstTs: "2026-11-12T10:00:00.000Z",
        lastTs: "2026-11-12T10:00:00.000Z",
        lastEventId: "e-b",
        lastType: "question-answered",
      },
    ]);
  });

  test("needs the secret, like the per-set reads", async () => {
    const env = makeEnv([["e1", "set-a", "set-raised", "2026-11-12T10:00:00.000Z"]]);
    expect((await get(env, "/sets", null)).status).toBe(401);
    expect((await get(env, "/sets", "wrong")).status).toBe(401);
  });
});
