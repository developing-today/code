import { describe, expect, test } from "bun:test";
import { sanitizeRegistry } from "../src/feed.ts";
import type { SessionRecord } from "../src/model.ts";
import { isSessionRecord, mergeRecords, recordFromLaunch, recordFromSessionEntry } from "../src/registry.ts";
import { parseRegistryQuery, queryRegistry } from "../src/registry-view.ts";
import { sessionRecordLine } from "../src/render.ts";

// #4284: SessionRecord.taskRefs is required, but isSessionRecord admitted a
// record with taskRefs absent. mergeRecords preserved undefined on the
// new-key path, and the admitted record then threw in queryRegistry for
// ?q= (spread of undefined) and ?task= (.some on undefined) as well as in
// sessionRecordLine (.flatMap on undefined) and the item-page linked-
// sessions filter (.some on undefined). The shared admission gate now
// rejects absent taskRefs, so the poisoned shape can no longer reach the
// query/render path through either untrusted admission path (the durable
// store file and the fleet feed registry section, both filtered with
// isSessionRecord in sources/live.ts).

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

const noRefs = {
  id: "ses_no_refs",
  source: "claude-code",
  label: "No taskRefs",
  agentId: "claude-code",
  agentLabel: "Muse",
  parentId: null,
  status: "succeeded",
  startedAt: "2026-11-01T00:00:00Z",
  lastSeenAt: "2026-11-02T00:00:00Z",
};

describe("absent taskRefs is rejected at the shared gate (#4284)", () => {
  test("gate rejects an otherwise-well-formed record without taskRefs", () => {
    expect(isSessionRecord(noRefs)).toBe(false);
  });

  test("gate still accepts [] and valid refs", () => {
    expect(isSessionRecord(rec({ id: "empty" }))).toBe(true);
    expect(isSessionRecord(rec({ id: "linked", taskRefs: ["developing-today/code#4284"] }))).toBe(true);
  });
});

describe("a taskRefs-less record drops individually through admission (#4284)", () => {
  test("store path: the bad record is filtered while well-formed neighbors survive", () => {
    const raw: unknown[] = [
      rec({ id: "ses_good" }),
      noRefs,
      rec({ id: "ses_linked", taskRefs: ["developing-today/code#4284"] }),
    ];
    // Same filter the durable store load applies (sources/live.ts).
    const admitted = raw.filter(isSessionRecord);
    expect(admitted.map((r) => (r as SessionRecord).id)).toEqual(["ses_good", "ses_linked"]);
    const merged = mergeRecords([], admitted as SessionRecord[]);
    expect(merged.map((r) => r.id)).toEqual(["ses_good", "ses_linked"]);
    for (const r of merged) expect(Array.isArray(r.taskRefs)).toBe(true);
  });

  test("feed path: the bad registry entry is skipped while well-formed entries merge", () => {
    // Feed sanitization uses the same gate the live registry later applies.
    const sanitized = sanitizeRegistry([rec({ id: "ses_feed_good" }), noRefs]);
    expect(sanitized.rows?.map((r) => r.id)).toEqual(["ses_feed_good"]);
    expect(sanitized.dropped).toBe(1);
    const merged = mergeRecords([], sanitized.rows ?? []);
    expect(merged).toHaveLength(1);
    expect(merged[0].taskRefs).toEqual([]);
  });
});

describe("admitted records no longer crash query/render (#4284)", () => {
  test("?q=, ?task=, sessionRecordLine, and the item-page session filter do not throw", () => {
    const raw: unknown[] = [rec({ id: "ses_good", label: "probe session" }), noRefs];
    const admitted = raw.filter(isSessionRecord) as SessionRecord[];
    expect(() => {
      queryRegistry(admitted, parseRegistryQuery(new URLSearchParams("q=probe&window=all")));
      queryRegistry(admitted, parseRegistryQuery(new URLSearchParams("task=code&window=all")));
      for (const r of admitted) sessionRecordLine(r);
      // Same filter the item page applies to linked sessions (server.ts).
      admitted.filter((r) => r.taskRefs.some((ref) => ["developing-today/code#4284"].includes(ref)));
    }).not.toThrow();
    expect(queryRegistry(admitted, parseRegistryQuery(new URLSearchParams("q=probe&window=all"))).matched).toBe(1);
  });
});

describe("both producers still emit gate-accepted records (#4284)", () => {
  test("recordFromSessionEntry and recordFromLaunch pass the gate", () => {
    const fromEntry = recordFromSessionEntry({
      sessionId: "ses_live",
      source: "opencode-sandbox",
      title: "Working developing-today/code#4284",
      lastActivityAt: "2026-11-02T00:00:00Z",
    });
    expect(isSessionRecord(fromEntry)).toBe(true);
    expect(Array.isArray(fromEntry.taskRefs)).toBe(true);
    const fromLaunch = recordFromLaunch({ id: "launch-1", label: "Launch" });
    expect(isSessionRecord(fromLaunch)).toBe(true);
    expect(fromLaunch.taskRefs).toEqual([]);
  });

  test("oversized URLs in session fields do not poison task refs (#4890)", () => {
    const fromEntry = recordFromSessionEntry({
      sessionId: "ses_oversized_refs",
      source: "opencode-sandbox",
      title:
        "bad https://github.com/developing-today/code/issues/99999999999999999999 " +
        "good https://github.com/developing-today/code/issues/123",
      pr:
        "bad https://github.com/developing-today/code/pull/9007199254740992 " +
        "good https://github.com/developing-today/code/pull/124",
      branch:
        "bad https://github.com/developing-today/code/issues/99999999999999999999 " +
        "good https://github.com/developing-today/code/pull/125",
      lastActivityAt: "2026-11-02T00:00:00Z",
    });

    expect(fromEntry.taskRefs.sort()).toEqual([
      "developing-today/code#123",
      "developing-today/code#124",
      "developing-today/code#125",
    ]);
    expect(isSessionRecord(fromEntry)).toBe(true);
  });
});
