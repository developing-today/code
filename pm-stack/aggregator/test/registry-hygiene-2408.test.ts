import { describe, expect, test } from "bun:test";
import { isSessionRecord, mergeRecords } from "../src/registry.ts";
import type { SessionRecord } from "../src/model.ts";

// #2408: isSessionRecord did not validate taskRefs, but mergeRecords
// spreads it on a duplicate-key merge — a number/object taskRefs threw
// TypeError (wedging /sessions and /api/registry.json via the durable
// store) and a string silently spread into single-character refs. The
// validator now owns the array-of-strings rule, so a present-but-invalid
// taskRefs rejects the record on every entry path (store load, feed,
// launch records all funnel through isSessionRecord).

function rec(partial: Partial<SessionRecord> & { id: string }): SessionRecord {
  return {
    source: "claude-code",
    label: partial.id,
    agentId: "claude-code",
    agentLabel: "Muse",
    parentId: null,
    status: "succeeded",
    ...partial,
  } as SessionRecord;
}

const valid = rec({
  id: "ses_valid",
  label: "Valid session",
  taskRefs: ["#123"],
  lastSeenAt: "2026-11-01T00:00:00Z",
});

describe("isSessionRecord taskRefs hygiene (#2408)", () => {
  test("accepts absent, empty, and string-array taskRefs", () => {
    expect(isSessionRecord({ id: "a", source: "s", label: "l" })).toBe(true);
    expect(isSessionRecord({ ...valid, taskRefs: [] })).toBe(true);
    expect(isSessionRecord(valid)).toBe(true);
    expect(isSessionRecord({ ...valid, taskRefs: ["#123", "#2408"] })).toBe(true);
  });

  test("rejects non-array taskRefs (number, string, object)", () => {
    expect(isSessionRecord({ ...valid, taskRefs: 42 as unknown as string[] })).toBe(false);
    expect(isSessionRecord({ ...valid, taskRefs: "abc" as unknown as string[] })).toBe(false);
    expect(isSessionRecord({ ...valid, taskRefs: { ref: "#1" } as unknown as string[] })).toBe(false);
  });

  test("rejects arrays containing non-strings", () => {
    expect(isSessionRecord({ ...valid, taskRefs: [42] as unknown as string[] })).toBe(false);
    expect(isSessionRecord({ ...valid, taskRefs: ["#1", null] as unknown as string[] })).toBe(false);
  });
});

describe("poisoned taskRefs cannot reach the mergeRecords spread (#2408)", () => {
  test("duplicate-key merge over the filtered set neither throws nor yields character refs", () => {
    const raw: unknown[] = [
      valid,
      { id: "ses_valid", source: "claude-code", label: "Poisoned twin", taskRefs: "abc" },
      { id: "ses_poison", source: "claude-code", label: "Poison", taskRefs: 42 },
    ];
    const admitted = raw.filter(isSessionRecord);
    expect(admitted.map((r) => r.id)).toEqual(["ses_valid"]);
    let merged: SessionRecord[] = [];
    expect(() => {
      merged = mergeRecords(admitted, [rec({ id: "ses_valid", taskRefs: ["#2408"] })]);
    }).not.toThrow();
    expect(merged).toHaveLength(1);
    expect(merged[0].taskRefs).toEqual(["#123", "#2408"]);
  });

  test("well-formed deduped-union merge is unchanged", () => {
    const prev = rec({ id: "ses_a", taskRefs: ["#1", "#2"] });
    const incoming = rec({ id: "ses_a", taskRefs: ["#2", "#3"] });
    const merged = mergeRecords([prev], [incoming]);
    expect(merged).toHaveLength(1);
    expect(merged[0].taskRefs).toEqual(["#1", "#2", "#3"]);
  });

  test("missing taskRefs stays valid and coalesces to the other side", () => {
    const prev = rec({ id: "ses_b", taskRefs: ["#7"] });
    const incoming = rec({ id: "ses_b" });
    const merged = mergeRecords([prev], [incoming]);
    expect(merged[0].taskRefs).toEqual(["#7"]);
  });
});
