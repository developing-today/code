import { describe, expect, test } from "bun:test";
import { isSessionRecord, mergeRecords, parseLaunchRecords } from "../src/registry.ts";
import type { SessionRecord } from "../src/model.ts";

// #2376: a present-but-non-string lastSeenAt/startedAt passed the hygiene
// filter and crashed mergeRecords' string-only sort (localeCompare on a
// number), taking down /sessions and /api/registry.json once such a record
// reached the registry through any entry path (store file, fleet feed,
// Claude launch records). The filter now rejects poisoned records at every
// path; valid records (string or absent timestamps) are untouched.

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
  startedAt: "2026-10-31T00:00:00Z",
  lastSeenAt: "2026-11-01T00:00:00Z",
});

const older = rec({
  id: "ses_older",
  label: "Older session",
  lastSeenAt: "2026-10-01T00:00:00Z",
});

describe("isSessionRecord timestamp hygiene (#2376)", () => {
  test("accepts string or absent timestamps", () => {
    expect(isSessionRecord(valid)).toBe(true);
    expect(isSessionRecord({ id: "a", source: "s", label: "l" })).toBe(true);
    expect(isSessionRecord(older)).toBe(true);
  });

  test("rejects numeric, boolean, and object lastSeenAt", () => {
    expect(isSessionRecord({ ...valid, lastSeenAt: 1730000000 as unknown as string })).toBe(false);
    expect(isSessionRecord({ ...valid, lastSeenAt: true as unknown as string })).toBe(false);
    expect(isSessionRecord({ ...valid, lastSeenAt: { t: 1 } as unknown as string })).toBe(false);
  });

  test("rejects numeric and boolean startedAt", () => {
    expect(isSessionRecord({ ...valid, startedAt: 1730000000 as unknown as string })).toBe(false);
    expect(isSessionRecord({ ...valid, startedAt: false as unknown as string })).toBe(false);
  });
});

describe("poisoned records cannot reach the mergeRecords sort (#2376)", () => {
  // The store-load and feed paths both admit records exclusively through
  // isSessionRecord (sources/live.ts), so filtering a raw parsed array the
  // same way exercises both entry paths.
  test("store/feed path: poisoned records are dropped, survivors merge and sort", () => {
    const raw: unknown[] = [
      valid,
      { id: "ses_poison_num", source: "claude-code", label: "Poison", lastSeenAt: 1730000000 },
      { id: "ses_poison_bool", source: "claude-code", label: "Poison", lastSeenAt: true },
      older,
      { id: "ses_no_ts", source: "claude-code", label: "No timestamps" },
    ];
    const admitted = raw.filter(isSessionRecord);
    expect(admitted.map((r) => r.id)).toEqual(["ses_valid", "ses_older", "ses_no_ts"]);
    let merged: SessionRecord[] = [];
    expect(() => {
      merged = mergeRecords([], admitted);
    }).not.toThrow();
    // Sort order for valid records is unchanged: newest lastSeenAt first,
    // a record without lastSeenAt sorts last.
    expect(merged.map((r) => r.id)).toEqual(["ses_valid", "ses_older", "ses_no_ts"]);
  });

  test("launch path: parseLaunchRecords drops records with non-string timestamps", () => {
    const jsonl = [
      JSON.stringify({
        id: "ses_launch_ok",
        label: "OK",
        startedAt: "2026-10-31T00:00:00Z",
        lastSeenAt: "2026-11-01T00:00:00Z",
      }),
      JSON.stringify({ id: "ses_launch_num", label: "Num", lastSeenAt: 1730000000 }),
      JSON.stringify({ id: "ses_launch_bool", label: "Bool", lastSeenAt: true }),
      JSON.stringify({ id: "ses_launch_bad_start", label: "BadStart", startedAt: 1730000000 }),
      JSON.stringify({ id: "ses_launch_no_ts", label: "NoTs" }),
    ].join("\n");
    const records = parseLaunchRecords(jsonl);
    expect(records.map((r) => r.id)).toEqual(["ses_launch_ok", "ses_launch_no_ts"]);
    expect(() => mergeRecords([], records)).not.toThrow();
  });

  test("valid records with absent lastSeenAt still merge as before", () => {
    const sparse = rec({ id: "ses_valid", label: "Valid session" });
    const merged = mergeRecords([valid], [sparse]);
    expect(merged).toHaveLength(1);
    expect(merged[0].lastSeenAt).toBe("2026-11-01T00:00:00Z");
  });
});
