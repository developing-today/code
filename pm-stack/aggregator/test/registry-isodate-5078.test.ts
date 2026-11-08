import { describe, expect, test } from "bun:test";
import { isSessionRecord } from "../src/registry.ts";
import type { SessionRecord } from "../src/model.ts";

// #5078: isSessionRecord gated lastSeenAt/startedAt on typeof string only
// (the #2376 type fix), never on date validity. Impossible dates
// (2026-02-30) roll to other instants in registryActivityMs, and a
// non-ISO stamp ("March 2, 2026") string-compares after every ISO stamp
// in mergeRecords, winning recency forever. Present timestamps must now
// be real ISO dates via the existing isIsoDate helper; absent timestamps
// stay allowed.

function rec(partial: Partial<SessionRecord> & { id: string }): SessionRecord {
  return {
    source: "claude-code",
    label: partial.id,
    agentId: "claude-code",
    agentLabel: "Muse",
    parentId: null,
    status: "succeeded",
    taskRefs: [],
    ...partial,
  } as SessionRecord;
}

const valid = rec({
  id: "ses_valid",
  startedAt: "2026-10-31T00:00:00Z",
  lastSeenAt: "2026-11-01T00:00:00Z",
});

const INVALID_STAMPS = ["2026-02-30", "2026-04-31", "March 2, 2026", "not-a-date"];

describe("isSessionRecord timestamp validity (#5078)", () => {
  test("rejects impossible and non-ISO lastSeenAt", () => {
    for (const stamp of INVALID_STAMPS) {
      expect(isSessionRecord({ ...valid, lastSeenAt: stamp })).toBe(false);
    }
  });

  test("rejects impossible and non-ISO startedAt", () => {
    for (const stamp of INVALID_STAMPS) {
      expect(isSessionRecord({ ...valid, startedAt: stamp })).toBe(false);
    }
  });

  test("accepts valid ISO timestamps, including offsets and fractions", () => {
    expect(isSessionRecord(valid)).toBe(true);
    expect(isSessionRecord({ ...valid, lastSeenAt: "2026-11-01T00:00:00.123+05:30" })).toBe(true);
    expect(isSessionRecord({ ...valid, startedAt: "2026-10-31T00:00:00-04:00" })).toBe(true);
    expect(isSessionRecord({ ...valid, lastSeenAt: "2026-11-01" })).toBe(true);
  });

  test("accepts absent timestamps", () => {
    expect(
      isSessionRecord({
        id: "a",
        source: "s",
        label: "l",
        status: "succeeded",
        agentId: "claude-code",
        agentLabel: "Muse",
        taskRefs: [],
      }),
    ).toBe(true);
    const { lastSeenAt: _ls, startedAt: _st, ...absent } = valid;
    expect(isSessionRecord(absent)).toBe(true);
  });
});
