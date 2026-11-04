import { describe, expect, test } from "bun:test";
import { sanitizeRegistry } from "../src/feed.ts";
import type { SessionRecord } from "../src/model.ts";
import {
  identitiesFromRecords,
  isSessionRecord,
  mergeRecords,
  parseLaunchRecords,
  recordFromLaunch,
  recordFromSessionEntry,
} from "../src/registry.ts";
import { sessionRecordIsLive } from "../src/render.ts";

// #3525: isSessionRecord admitted records missing status/agentId/agentLabel
// and records whose tokensTotal was a string, Infinity, negative or an
// object. Consumers dereference all of them unconditionally:
// sessionRecordIsLive calls r.status.toLowerCase(), identitiesFromRecords
// interpolates r.agentLabel, renderSessions sums r.tokensTotal and
// sessionRecordLine formats it with toLocaleString(). One poisoned record
// in the durable store, the feed registry section or a hand-edited launch
// JSONL line crashed the sessions page or poisoned its token total. The
// shared gate now owns the rule (the #2347/#2376 precedent): status,
// agentId and agentLabel must be strings; tokensTotal is optional but,
// when present, must be a finite number >= 0. Reject, never coerce.

function rec(partial: Partial<SessionRecord> & { id: string }): SessionRecord {
  return {
    source: "claude-code",
    label: partial.id,
    agentId: "claude-code",
    agentLabel: "Muse",
    parentId: null,
    status: "succeeded",
    startedAt: "2026-11-01T00:00:00Z",
    lastSeenAt: "2026-11-02T00:00:00Z",
    taskRefs: [],
    ...partial,
  } as SessionRecord;
}

const valid = rec({ id: "ses_valid", tokensTotal: 1200 });

function without(key: string, base: Record<string, unknown>): Record<string, unknown> {
  const copy = { ...base };
  delete copy[key];
  return copy;
}

const validRaw: Record<string, unknown> = {
  id: "ses_raw",
  source: "claude-code",
  label: "Raw",
  agentId: "claude-code",
  agentLabel: "Muse",
  parentId: null,
  status: "succeeded",
  taskRefs: [],
};

describe("status / agent identity are required strings at the gate (#3525)", () => {
  test("gate accepts a well-formed record, with or without tokensTotal", () => {
    expect(isSessionRecord(valid)).toBe(true);
    expect(isSessionRecord(rec({ id: "ses_no_tokens" }))).toBe(true);
    expect(isSessionRecord(rec({ id: "ses_zero_tokens", tokensTotal: 0 }))).toBe(true);
  });

  test("gate rejects missing or non-string status", () => {
    expect(isSessionRecord(without("status", validRaw))).toBe(false);
    expect(isSessionRecord({ ...validRaw, status: 42 })).toBe(false);
    expect(isSessionRecord({ ...validRaw, status: null })).toBe(false);
  });

  test("gate rejects missing or non-string agentId / agentLabel", () => {
    expect(isSessionRecord(without("agentId", validRaw))).toBe(false);
    expect(isSessionRecord(without("agentLabel", validRaw))).toBe(false);
    expect(isSessionRecord({ ...validRaw, agentId: 7 })).toBe(false);
    expect(isSessionRecord({ ...validRaw, agentLabel: { name: "x" } })).toBe(false);
  });
});

describe("tokensTotal, when present, must be a finite number >= 0 (#3525)", () => {
  test("gate rejects string, Infinity, NaN, negative and object tokensTotal", () => {
    expect(isSessionRecord({ ...validRaw, tokensTotal: "5000" })).toBe(false);
    const inf = JSON.parse('{"tokensTotal":1e999}') as { tokensTotal: number };
    expect(inf.tokensTotal).toBe(Infinity);
    expect(isSessionRecord({ ...validRaw, tokensTotal: inf.tokensTotal })).toBe(false);
    expect(isSessionRecord({ ...validRaw, tokensTotal: NaN })).toBe(false);
    expect(isSessionRecord({ ...validRaw, tokensTotal: -500 })).toBe(false);
    expect(isSessionRecord({ ...validRaw, tokensTotal: { x: 1 } })).toBe(false);
    expect(isSessionRecord({ ...validRaw, tokensTotal: null })).toBe(false);
  });
});

describe("real producers still pass the gate unchanged (#3525)", () => {
  test("recordFromSessionEntry output passes, with and without tokens", () => {
    const entry = {
      sessionId: "ses_feed",
      source: "claude-code" as const,
      lastActivityAt: "2026-11-02T00:00:00Z",
      title: "Feed session",
    };
    expect(isSessionRecord(recordFromSessionEntry(entry, 42_000))).toBe(true);
    expect(isSessionRecord(recordFromSessionEntry(entry))).toBe(true);
  });

  test("recordFromLaunch output passes (status defaults to launched)", () => {
    expect(isSessionRecord(recordFromLaunch({ id: "launch_ok" }))).toBe(true);
    expect(isSessionRecord(recordFromLaunch({ id: "launch_tok", tokensTotal: 9000 }))).toBe(true);
  });
});

describe("a poisoned record drops individually on every admission path (#3525)", () => {
  test("store path: filter drops the poisoned records, survivors render", () => {
    const raw: unknown[] = [
      valid,
      without("status", validRaw),
      { ...validRaw, id: "ses_str_tokens", tokensTotal: "5000" },
      rec({ id: "ses_good_2", tokensTotal: 300 }),
    ];
    const admitted = raw.filter(isSessionRecord) as SessionRecord[];
    expect(admitted.map((r) => r.id)).toEqual(["ses_valid", "ses_good_2"]);
    const merged = mergeRecords([], admitted);
    for (const r of merged) expect(() => sessionRecordIsLive(r)).not.toThrow();
    const total = merged.reduce((sum, r) => sum + (r.tokensTotal ?? 0), 0);
    expect(total).toBe(1500);
    expect(identitiesFromRecords(merged).some((i) => i.label.includes("undefined"))).toBe(false);
  });

  test("feed path: sanitizeRegistry drops poisoned entries, keeps well-formed ones", () => {
    const sanitized = sanitizeRegistry([
      valid,
      { ...validRaw, id: "ses_inf", tokensTotal: Infinity },
      rec({ id: "ses_feed_good" }),
    ]);
    expect(sanitized.rows?.map((r) => r.id)).toEqual(["ses_valid", "ses_feed_good"]);
    expect(sanitized.dropped).toBe(1);
  });

  test("launch path: parseLaunchRecords drops non-string status and string tokensTotal lines", () => {
    const jsonl = [
      JSON.stringify({ id: "launch_ok", label: "OK", tokensTotal: 100 }),
      JSON.stringify({ id: "launch_bad_status", label: "Bad", status: 42 }),
      JSON.stringify({ id: "launch_bad_tokens", label: "Bad", tokensTotal: "9000" }),
      '{"id":"launch_inf","label":"Inf","tokensTotal":1e999}',
    ].join("\n");
    const records = parseLaunchRecords(jsonl);
    expect(records.map((r) => r.id)).toEqual(["launch_ok"]);
  });
});
