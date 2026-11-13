import { describe, expect, test } from "bun:test";
import { groupBySet, isAuthorized, parseEventBatch, timingSafeEqual, validateDecisionEvent } from "../src/events.ts";

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
