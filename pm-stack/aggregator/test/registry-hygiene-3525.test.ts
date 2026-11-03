import { describe, expect, test } from "bun:test";
import {
  isSessionRecord,
  mergeRecords,
  parseLaunchRecords,
  recordFromLaunch,
  recordFromSessionEntry,
} from "../src/registry.ts";
import type { SessionRecord } from "../src/model.ts";
import { sessionRecordIsLive } from "../src/render.ts";

// #3525: isSessionRecord never checked status, agentId/agentLabel, or
// tokensTotal, which consumers dereference unconditionally
// (sessionRecordIsLive calls r.status.toLowerCase(); renderSessions sums
// tokensTotal; identitiesFromRecords interpolates agentLabel). A record
// with missing/non-string status crashed the sessions page; a
// string/Infinity/negative/object tokensTotal poisoned the fleet token
// total; a missing agent identity rendered a broken label. The gate now
// requires status/agentId/agentLabel to be strings and tokensTotal to be
// absent or a finite number >= 0 — reject, never coerce (#2376/#2408
// precedent).

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

const valid = rec({ id: "ses_valid", label: "Valid session", tokensTotal: 5000 });

describe("isSessionRecord status hygiene (#3525)", () => {
  test("rejects absent status", () => {
    const { status: _dropped, ...noStatus } = valid;
    expect(isSessionRecord(noStatus)).toBe(false);
  });

  test("rejects non-string status", () => {
    for (const status of [42, true, { s: "idle" }, null, ["idle"]]) {
      expect(isSessionRecord({ ...valid, status: status as unknown as string })).toBe(false);
    }
  });

  test("accepts string status", () => {
    expect(isSessionRecord(valid)).toBe(true);
    expect(isSessionRecord({ ...valid, status: "in-progress" })).toBe(true);
  });
});

describe("isSessionRecord agent-identity hygiene (#3525)", () => {
  test("rejects absent agentId/agentLabel", () => {
    const { agentId: _a, ...noAgentId } = valid;
    expect(isSessionRecord(noAgentId)).toBe(false);
    const { agentLabel: _l, ...noAgentLabel } = valid;
    expect(isSessionRecord(noAgentLabel)).toBe(false);
  });

  test("rejects non-string agentId/agentLabel", () => {
    expect(isSessionRecord({ ...valid, agentId: 42 as unknown as string })).toBe(false);
    expect(isSessionRecord({ ...valid, agentLabel: null as unknown as string })).toBe(false);
    expect(isSessionRecord({ ...valid, agentLabel: { n: "Muse" } as unknown as string })).toBe(false);
  });
});

describe("isSessionRecord tokensTotal hygiene (#3525)", () => {
  test("rejects string, infinite, negative, NaN, null, and object tokensTotal", () => {
    for (const tokensTotal of ["5000", 1e999, Infinity, -Infinity, -500, NaN, null, { x: 1 }, [5000]]) {
      expect(isSessionRecord({ ...valid, tokensTotal: tokensTotal as unknown as number })).toBe(false);
    }
  });

  test("accepts absent, zero, and positive finite tokensTotal", () => {
    const { tokensTotal: _dropped, ...noTokens } = valid;
    expect(isSessionRecord(noTokens)).toBe(true);
    expect(isSessionRecord({ ...valid, tokensTotal: 0 })).toBe(true);
    expect(isSessionRecord({ ...valid, tokensTotal: 5000 })).toBe(true);
  });
});

describe("poisoned #3525 records drop individually, survivors stay live and numeric (#3525)", () => {
  test("store/feed path: each poisoned shape drops while neighbors merge, stay live, and sum finite", () => {
    const raw: unknown[] = [
      valid,
      { ...valid, id: "ses_no_status", status: undefined },
      { ...valid, id: "ses_num_status", status: 42 },
      { ...valid, id: "ses_str_tokens", tokensTotal: "5000" },
      { ...valid, id: "ses_inf_tokens", tokensTotal: 1e999 },
      { ...valid, id: "ses_neg_tokens", tokensTotal: -500 },
      { ...valid, id: "ses_obj_tokens", tokensTotal: { x: 1 } },
      { ...valid, id: "ses_no_agent", agentId: undefined },
      rec({ id: "ses_no_tokens", label: "No token total" }),
    ];
    const admitted = raw.filter(isSessionRecord) as SessionRecord[];
    expect(admitted.map((r) => r.id)).toEqual(["ses_valid", "ses_no_tokens"]);
    const merged = mergeRecords([], admitted);
    expect(merged).toHaveLength(2);
    // The crash site: every admitted record has a string status, so the
    // live check cannot throw.
    expect(() => merged.map((r) => sessionRecordIsLive(r))).not.toThrow();
    // The fleet total: every admitted tokensTotal is absent or a finite
    // number >= 0, so the sum stays numeric.
    const total = merged.reduce((sum, r) => sum + (r.tokensTotal ?? 0), 0);
    expect(typeof total).toBe("number");
    expect(Number.isFinite(total)).toBe(true);
    expect(total).toBe(5000);
  });

  test("launch path: parseLaunchRecords admits well-formed lines and drops poisoned #3525 fields", () => {
    const jsonl = [
      JSON.stringify({ id: "ses_launch_ok", label: "OK", tokensTotal: 1200 }),
      JSON.stringify({ id: "ses_launch_no_tokens", label: "NoTokens" }),
      JSON.stringify({ id: "ses_launch_num_status", label: "NumStatus", status: 42, tokensTotal: "9000" }),
      JSON.stringify({ id: "ses_launch_str_tokens", label: "StrTokens", tokensTotal: "9000" }),
      JSON.stringify({ id: "ses_launch_inf_tokens", label: "InfTokens", tokensTotal: 1e999 }),
      JSON.stringify({ id: "ses_launch_neg_tokens", label: "NegTokens", tokensTotal: -500 }),
    ].join("\n");
    const records = parseLaunchRecords(jsonl);
    expect(records.map((r) => r.id)).toEqual(["ses_launch_ok", "ses_launch_no_tokens"]);
    expect(() => mergeRecords([], records)).not.toThrow();
  });
});

describe("both producers still emit gate-accepted records (#3525)", () => {
  test("recordFromSessionEntry passes with and without tokensTotal", () => {
    const base = {
      sessionId: "ses_live",
      source: "opencode-sandbox" as const,
      title: "Working developing-today/code#3525",
      lastActivityAt: "2026-11-02T00:00:00Z",
    };
    expect(isSessionRecord(recordFromSessionEntry(base))).toBe(true);
    expect(isSessionRecord(recordFromSessionEntry(base, 3210))).toBe(true);
    expect(typeof recordFromSessionEntry(base, 3210).status).toBe("string");
  });

  test("recordFromLaunch defaults to a string status and passes the gate", () => {
    const fromLaunch = recordFromLaunch({ id: "launch-1", label: "Launch" });
    expect(fromLaunch.status).toBe("launched");
    expect(isSessionRecord(fromLaunch)).toBe(true);
  });
});
