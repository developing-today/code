import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  decisionEventInsertStatement,
  emptySyncState,
  loadSyncState,
  markAcked,
  pendingForLeg,
  readOutboxEvents,
  saveSyncState,
  syncDecisionEvents,
  validateDecisionEvent,
  type DecisionSetEvent,
  type FetchFn,
} from "../src/decision-events-sync.ts";

const ev = (eventId: string, extra: Partial<DecisionSetEvent> = {}): DecisionSetEvent => ({
  eventId,
  setId: "set-1",
  questionRef: "ledger-1#1",
  type: "question-answered",
  payload: { optionId: "a" },
  ts: "2026-11-12T10:00:00.000Z",
  ...extra,
});

let dir: string;
beforeEach(async () => {
  dir = await mkdtemp(join(tmpdir(), "decision-events-"));
});
afterEach(async () => {
  await rm(dir, { recursive: true, force: true });
});

describe("validateDecisionEvent", () => {
  test("accepts a full event and defaults payload/questionRef", () => {
    const v = validateDecisionEvent({ eventId: "e1", setId: "s1", type: "set-raised", ts: "2026-11-12T10:00:00Z" });
    expect(v.ok).toBe(true);
    if (v.ok) {
      expect(v.value.questionRef).toBeNull();
      expect(v.value.payload).toEqual({});
    }
  });
  test("rejects missing ids, bad type, bad ts, array payload", () => {
    expect(validateDecisionEvent({ setId: "s", type: "x", ts: "2026-11-12T10:00:00Z" }).ok).toBe(false);
    expect(validateDecisionEvent({ eventId: "e", setId: "s", type: "has space", ts: "2026-11-12T10:00:00Z" }).ok).toBe(
      false,
    );
    expect(validateDecisionEvent({ eventId: "e", setId: "s", type: "x", ts: "2026-02-30T10:00:00Z" }).ok).toBe(false);
    expect(
      validateDecisionEvent({ eventId: "e", setId: "s", type: "x", ts: "2026-11-12T10:00:00Z", payload: [1] }).ok,
    ).toBe(false);
  });
});

describe("readOutboxEvents", () => {
  test("parses JSONL, dedupes by eventId, reports bad lines with numbers", async () => {
    const file = join(dir, "outbox.jsonl");
    await writeFile(
      file,
      [
        JSON.stringify(ev("e1")),
        "",
        "not json",
        JSON.stringify(ev("e1")),
        JSON.stringify({ eventId: "e2" }),
        JSON.stringify(ev("e3", { type: "draft-changed", payload: { draft: "typing…" } })),
      ].join("\n"),
      "utf8",
    );
    const read = await readOutboxEvents(file);
    expect(read.events.map((e) => e.eventId)).toEqual(["e1", "e3"]);
    expect(read.duplicates).toEqual(["e1"]);
    expect(read.errors.map((e) => e.line)).toEqual([3, 5]);
  });
  test("missing file is an empty outbox", async () => {
    const read = await readOutboxEvents(join(dir, "nope.jsonl"));
    expect(read.events).toEqual([]);
    expect(read.errors).toEqual([]);
  });
});

/** Fake libSQL pipeline server: records statements, acks rows_affected
 * 1 for a fresh event_id and 0 for one already "stored" — the oracle
 * for INSERT OR IGNORE idempotence. */
function fakeTurso() {
  const stored = new Set<string>();
  const statements: { sql: string; args?: { type: string; value?: string }[] }[] = [];
  const fetchFn: FetchFn = async (_url, init) => {
    const body = JSON.parse(String(init.body)) as {
      requests: { type: string; stmt?: { sql: string; args?: { type: string; value?: string }[] } }[];
    };
    const results = body.requests
      .filter((r) => r.type === "execute")
      .map((r) => {
        statements.push(r.stmt as { sql: string });
        let affected = 0;
        if (r.stmt?.sql.startsWith("INSERT OR IGNORE")) {
          const id = r.stmt.args?.[0]?.value as string;
          if (!stored.has(id)) {
            stored.add(id);
            affected = 1;
          }
        }
        // Mirror the real libSQL envelope: type "ok", affected_row_count.
        return { type: "ok", response: { type: "execute", result: { rows: [], affected_row_count: affected } } };
      });
    return new Response(JSON.stringify({ results }), { status: 200 });
  };
  return { stored, statements, fetchFn };
}

describe("turso leg", () => {
  test("insert statement is INSERT OR IGNORE with typed args, payload as JSON text", () => {
    const stmt = decisionEventInsertStatement(ev("e9"));
    expect(stmt.sql).toContain("INSERT OR IGNORE INTO decision_set_events");
    expect(stmt.args?.map((a) => (a.type === "text" ? a.value : a.type))).toEqual([
      "e9",
      "set-1",
      "ledger-1#1",
      "question-answered",
      JSON.stringify({ optionId: "a" }),
      "2026-11-12T10:00:00.000Z",
    ]);
  });

  test("sync inserts once; a second sync of the same events inserts nothing and sends nothing", async () => {
    const turso = fakeTurso();
    const state = emptySyncState();
    const events = [ev("e1"), ev("e2")];
    const first = await syncDecisionEvents({
      events,
      state,
      legs: ["turso"],
      tursoUrl: "https://example.invalid",
      tursoToken: "t",
      fetchFn: turso.fetchFn,
    });
    expect(first[0]).toMatchObject({ pending: 2, attempted: 2, inserted: 2 });
    expect(turso.statements.some((s) => s.sql.startsWith("CREATE TABLE IF NOT EXISTS"))).toBe(true);

    const second = await syncDecisionEvents({
      events,
      state,
      legs: ["turso"],
      tursoUrl: "https://example.invalid",
      tursoToken: "t",
      fetchFn: turso.fetchFn,
    });
    expect(second[0]).toMatchObject({ pending: 0, attempted: 0, inserted: 0 });

    // Even with state lost, the sink itself dedupes (INSERT OR IGNORE).
    const third = await syncDecisionEvents({
      events,
      state: emptySyncState(),
      legs: ["turso"],
      tursoUrl: "https://example.invalid",
      tursoToken: "t",
      fetchFn: turso.fetchFn,
    });
    expect(third[0]).toMatchObject({ attempted: 2, inserted: 0 });
    expect(turso.stored.size).toBe(2);
  });

  test("a failing pipeline leaves events pending and reports the error", async () => {
    const state = emptySyncState();
    const failFetch: FetchFn = async () => new Response("nope", { status: 500 });
    const reports = await syncDecisionEvents({
      events: [ev("e1")],
      state,
      legs: ["turso"],
      tursoUrl: "https://example.invalid",
      tursoToken: "t",
      fetchFn: failFetch,
    });
    expect(reports[0]?.error).toContain("HTTP 500");
    expect(pendingForLeg([ev("e1")], state, "turso").length).toBe(1);
  });

  test("a missing token is a named leg failure, not a throw", async () => {
    const reports = await syncDecisionEvents({
      events: [ev("e1")],
      state: emptySyncState(),
      legs: ["turso"],
      tursoToken: null,
      fetchFn: fakeTurso().fetchFn,
    });
    expect(reports[0]?.error).toContain("turso.fleet-platform-rw-token");
  });
});

describe("webhook leg", () => {
  test("posts batches with the bearer secret and counts inserted", async () => {
    const seen: { auth?: string; events: unknown[] }[] = [];
    const fetchFn: FetchFn = async (_url, init) => {
      const headers = init.headers as Record<string, string>;
      const body = JSON.parse(String(init.body)) as { events: unknown[] };
      seen.push({ auth: headers.Authorization, events: body.events });
      return new Response(JSON.stringify({ accepted: body.events.length, inserted: body.events.length }), {
        status: 200,
      });
    };
    const state = emptySyncState();
    const reports = await syncDecisionEvents({
      events: [ev("e1")],
      state,
      legs: ["webhook"],
      webhookUrl: "https://worker.example/events",
      webhookSecret: "s3cr3t",
      fetchFn,
    });
    expect(reports[0]).toMatchObject({ attempted: 1, inserted: 1 });
    expect(seen[0]?.auth).toBe("Bearer s3cr3t");
    expect(pendingForLeg([ev("e1")], state, "webhook").length).toBe(0);
  });

  test("one leg failing does not block the other", async () => {
    const turso = fakeTurso();
    const failFetch: FetchFn = async (url, init) =>
      String(url).includes("worker.example") ? new Response("down", { status: 503 }) : turso.fetchFn(url, init);
    const state = emptySyncState();
    const reports = await syncDecisionEvents({
      events: [ev("e1")],
      state,
      legs: ["turso", "webhook"],
      tursoUrl: "https://example.invalid",
      tursoToken: "t",
      webhookUrl: "https://worker.example/events",
      webhookSecret: "s",
      fetchFn: failFetch,
    });
    expect(reports[0]?.error).toBeUndefined();
    expect(reports[1]?.error).toContain("HTTP 503");
    expect(pendingForLeg([ev("e1")], state, "turso").length).toBe(0);
    expect(pendingForLeg([ev("e1")], state, "webhook").length).toBe(1);
  });
});

describe("sync state", () => {
  test("round-trips through disk; corrupt state resets to empty", async () => {
    const path = join(dir, "state.json");
    const state = emptySyncState();
    markAcked(state, "turso", [ev("e1")]);
    await saveSyncState(path, state);
    const loaded = await loadSyncState(path);
    expect(loaded.acked.turso).toEqual(["e1"]);
    await writeFile(path, "{broken", "utf8");
    expect((await loadSyncState(path)).acked.turso).toEqual([]);
  });
});

describe("Track A outbox format (reconciliation with PR #9343)", () => {
  test("reads Track A outbox lines: envelope unwrapped, set.created accepted", async () => {
    const file = join(dir, "outbox.jsonl");
    await writeFile(
      file,
      JSON.stringify({
        eventId: "a1",
        setId: "3df0fc87-dded-4ecf-be73-c3f51b1c1def",
        type: "set.created",
        payload: { via: "api/decision-sets" },
        ts: "2026-11-13T05:37:12.808Z",
        outbox: { status: "pending", enqueuedAt: "2026-11-13T05:37:12.808Z", sink: "turso-cloudflare" },
      }) + "\n",
      "utf8",
    );
    const read = await readOutboxEvents(file);
    expect(read.errors).toEqual([]);
    expect(read.events).toHaveLength(1);
    expect(read.events[0]).toMatchObject({ eventId: "a1", type: "set.created", questionRef: null });
  });

  test("maps Track A object questionRef {ledgerId, qid} onto the qid string", () => {
    const v = validateDecisionEvent({
      eventId: "a2",
      setId: "s1",
      type: "answer",
      ts: "2026-11-12T10:00:00.000Z",
      questionRef: { ledgerId: "ledger-1", qid: "ledger-1#2", questionIndex: 2 },
      payload: {},
    });
    expect(v.ok).toBe(true);
    if (v.ok) expect(v.value.questionRef).toBe("ledger-1#2");
  });
});
