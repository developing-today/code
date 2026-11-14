import { describe, expect, test } from "bun:test";
import { groupBySet, isAuthorized, parseEventBatch, timingSafeEqual, validateDecisionEvent } from "../src/events.ts";
import worker, { DecisionSetEvents, type Env } from "../src/index.ts";

const ev = (eventId: string, setId = "set-1") => ({
  eventId,
  setId,
  questionRef: "ledger-1#1",
  type: "question-answered",
  payload: { optionId: "a" },
  ts: "2026-11-12T10:00:00.000Z",
});

describe("validateDecisionEvent", () => {
  test("accepts the wire shape, defaults payload", () => {
    const v = validateDecisionEvent({ eventId: "e1", setId: "s1", type: "set-raised", ts: "2026-11-12T10:00:00Z" });
    expect(v.ok).toBe(true);
    if (v.ok) expect(v.value.payload).toEqual({});
  });
  test("rejects bad ids/types/timestamps", () => {
    expect(validateDecisionEvent({ ...ev("e"), eventId: "has space" }).ok).toBe(false);
    expect(validateDecisionEvent({ ...ev("e"), type: "" }).ok).toBe(false);
    expect(validateDecisionEvent({ ...ev("e"), ts: "not-a-date" }).ok).toBe(false);
    expect(validateDecisionEvent({ ...ev("e"), ts: "2026-11-12" }).ok).toBe(false);
  });
});

describe("parseEventBatch", () => {
  test("accepts {events}, bare arrays, and single events", () => {
    expect(parseEventBatch({ events: [ev("e1")] }).ok).toBe(true);
    expect(parseEventBatch([ev("e1")]).ok).toBe(true);
    expect(parseEventBatch(ev("e1")).ok).toBe(true);
    expect(parseEventBatch({ nope: 1 }).ok).toBe(false);
    expect(parseEventBatch({ events: [] }).ok).toBe(false);
  });
  test("reports per-index errors and in-batch duplicates without dropping valid events", () => {
    const parsed = parseEventBatch({ events: [ev("e1"), { bad: true }, ev("e1"), ev("e2", "set-2")] });
    expect(parsed.ok).toBe(true);
    if (parsed.ok) {
      expect(parsed.value.events.map((e) => e.eventId)).toEqual(["e1", "e2"]);
      expect(parsed.value.errors.map((e) => e.index)).toEqual([1, 2]);
    }
  });
});

describe("auth helpers", () => {
  test("timingSafeEqual", () => {
    expect(timingSafeEqual("abc", "abc")).toBe(true);
    expect(timingSafeEqual("abc", "abd")).toBe(false);
    expect(timingSafeEqual("abc", "abcd")).toBe(false);
  });
  test("isAuthorized accepts Bearer or X-Webhook-Secret, fails closed unset", () => {
    const bearer = new Headers({ Authorization: "Bearer s3cr3t" });
    const header = new Headers({ "X-Webhook-Secret": "s3cr3t" });
    const wrong = new Headers({ Authorization: "Bearer nope" });
    expect(isAuthorized(bearer, "s3cr3t")).toBe(true);
    expect(isAuthorized(header, "s3cr3t")).toBe(true);
    expect(isAuthorized(wrong, "s3cr3t")).toBe(false);
    expect(isAuthorized(bearer, undefined)).toBe(false);
  });
});

describe("groupBySet", () => {
  test("groups preserving order", () => {
    const parsed = parseEventBatch([ev("e1"), ev("e2", "set-2"), ev("e3")]);
    if (!parsed.ok) throw new Error("parse failed");
    const groups = groupBySet(parsed.value.events);
    expect([...groups.keys()]).toEqual(["set-1", "set-2"]);
    expect(groups.get("set-1")?.map((e) => e.eventId)).toEqual(["e1", "e3"]);
  });
});

describe("DecisionSetEvents & POST /events fan-out retry", () => {
  function makeMockEnv() {
    const d1Events = new Set<string>();
    const doStorage = new Map<string, unknown>();
    const doInstance = new DecisionSetEvents({
      storage: {
        get: async <T>(k: string) => doStorage.get(k) as T | undefined,
        put: async (k: string, v: unknown) => {
          doStorage.set(k, v);
        },
      },
      getWebSockets: () => [],
    } as unknown as DurableObjectState);

    let failNextIngest = false;
    const env = {
      WEBHOOK_SECRET: "s3cr3t",
      DB: {
        prepare: () => ({
          bind: (eventId: string) => ({ eventId }),
        }),
        batch: async (stmts: Array<{ eventId: string }>) =>
          stmts.map((s) => {
            if (d1Events.has(s.eventId)) return { meta: { changes: 0 } };
            d1Events.add(s.eventId);
            return { meta: { changes: 1 } };
          }),
      } as unknown as D1Database,
      SET_EVENTS: {
        idFromName: (name: string) => name as unknown as DurableObjectId,
        get: () => ({
          fetch: async (url: string, init?: RequestInit) => {
            if (failNextIngest) {
              failNextIngest = false;
              return new Response("boom", { status: 500 });
            }
            return doInstance.fetch(new Request(url, init));
          },
        }),
      } as unknown as DurableObjectNamespace,
    } satisfies Env;

    return {
      env,
      setFailNextIngest: (v: boolean) => {
        failNextIngest = v;
      },
    };
  }

  test("returns 502 on DO ingest failure and fans out exactly once on retry", async () => {
    const { env, setFailNextIngest } = makeMockEnv();
    const post = () =>
      worker.fetch(
        new Request("https://worker/events", {
          method: "POST",
          headers: { Authorization: "Bearer s3cr3t", "Content-Type": "application/json" },
          body: JSON.stringify({ events: [ev("e1")] }),
        }),
        env,
      );

    setFailNextIngest(true);
    const first = await post();
    expect(first.status).toBe(502);
    const firstBody = (await first.json()) as { inserted: number; fanout: Record<string, { error?: string }> };
    expect(firstBody.inserted).toBe(1);
    expect(firstBody.fanout["set-1"]?.error).toContain("500");

    // Retry: D1 reports 0 changes, DO ingest succeeds and fans out e1 once.
    const retry = await post();
    expect(retry.status).toBe(200);
    const retryBody = (await retry.json()) as {
      inserted: number;
      duplicates: number;
      fanout: Record<string, { fannedOut: number; deduplicated: number }>;
    };
    expect(retryBody.inserted).toBe(0);
    expect(retryBody.duplicates).toBe(1);
    expect(retryBody.fanout["set-1"]?.fannedOut).toBe(1);
    expect(retryBody.fanout["set-1"]?.deduplicated).toBe(0);

    // Second retry: DO deduplicates e1, fannedOut remains 1.
    const secondRetry = await post();
    expect(secondRetry.status).toBe(200);
    const secondBody = (await secondRetry.json()) as {
      fanout: Record<string, { fannedOut: number; deduplicated: number }>;
    };
    expect(secondBody.fanout["set-1"]?.fannedOut).toBe(1);
    expect(secondBody.fanout["set-1"]?.deduplicated).toBe(1);
  });
});
