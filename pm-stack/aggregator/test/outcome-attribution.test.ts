import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { GithubHotEvent } from "../src/activity.ts";
import {
  OUTCOME_ATTRIBUTION_VERSION,
  OutcomeFactStore,
  projectOutcomeAttribution,
  rollupOutcomeFacts,
  type OutcomeBinding,
  type OutcomeCounts,
} from "../src/outcome-attribution.ts";

/** Fixture hot-list rows in the projector's normalized shape (#5204):
 *  `event_id` is `<delivery>:<sub>` where sub is `issue:<id>`,
 *  `pr:<id>`, `issue_comment:<id>`, `review:<id>`,
 *  `review_comment:<id>`, `commit:<sha>`, or the zero-commit fallback
 *  `push:<after-sha>` (pm-stack/projector/src/projector.ts). */
function hot(partial: Partial<GithubHotEvent> & Pick<GithubHotEvent, "event_id">): GithubHotEvent {
  return {
    seq: 0,
    delivery_id: "d1",
    repo: "developing-today/code",
    event: "issues",
    action: null,
    ref: null,
    actor: "dezren39",
    source_url: null,
    source_timestamp: "2026-11-22T10:00:00Z",
    received_at: "2026-11-22T10:00:05Z",
    ...partial,
  };
}

const ISSUE_URL = "https://github.com/developing-today/code/issues/101";
const PR_URL = "https://github.com/developing-today/code/pull/202";

function fixtureEvents(): GithubHotEvent[] {
  return [
    hot({
      seq: 1,
      event_id: "d1:issue:1001",
      event: "issues",
      action: "opened",
      ref: "developing-today/code#101",
      source_url: ISSUE_URL,
    }),
    hot({
      seq: 2,
      event_id: "d2:issue:1001",
      delivery_id: "d2",
      event: "issues",
      action: "edited",
      ref: "developing-today/code#101",
      source_url: ISSUE_URL,
    }),
    hot({
      seq: 3,
      event_id: "d3:issue:1001",
      delivery_id: "d3",
      event: "issues",
      action: "closed",
      ref: "developing-today/code#101",
      source_url: ISSUE_URL,
    }),
    hot({
      seq: 4,
      event_id: "d4:pr:2002",
      delivery_id: "d4",
      event: "pull_request",
      action: "opened",
      ref: "developing-today/code#202",
      source_url: PR_URL,
    }),
    hot({
      seq: 5,
      event_id: "d5:pr:2002",
      delivery_id: "d5",
      event: "pull_request",
      action: "edited",
      ref: "developing-today/code#202",
      source_url: PR_URL,
    }),
    hot({
      seq: 6,
      event_id: "d6:issue_comment:3003",
      delivery_id: "d6",
      event: "issue_comment",
      action: "created",
      ref: "developing-today/code#101",
      source_url: `${ISSUE_URL}#issuecomment-3003`,
    }),
    hot({
      seq: 7,
      event_id: "d7:review:4004",
      delivery_id: "d7",
      event: "pull_request_review",
      action: "submitted",
      ref: "developing-today/code#202",
      source_url: `${PR_URL}#pullrequestreview-4004`,
    }),
    hot({
      seq: 8,
      event_id: "d8:review_comment:5005",
      delivery_id: "d8",
      event: "pull_request_review_comment",
      action: "created",
      ref: "developing-today/code#202",
      source_url: `${PR_URL}#discussion_r5005`,
    }),
    hot({
      seq: 9,
      event_id: "d9:commit:aaa111",
      delivery_id: "d9",
      event: "push",
      action: "push",
      ref: "refs/heads/felix/example",
      source_url: "https://github.com/developing-today/code/commit/aaa111",
    }),
    hot({
      seq: 10,
      event_id: "d9:commit:bbb222",
      delivery_id: "d9",
      event: "push",
      action: "push",
      ref: "refs/heads/felix/example",
      source_url: "https://github.com/developing-today/code/commit/bbb222",
    }),
    // Zero-commit fallback row: the push exposed no commit list, so the
    // projector emitted one `push:<after>` row. It proves at least one
    // push happened, not an exact commit count -> partial coverage.
    hot({
      seq: 11,
      event_id: "d10:push:ccc333",
      delivery_id: "d10",
      event: "push",
      action: "push",
      ref: "refs/heads/main",
      source_url: "https://github.com/developing-today/code/commit/ccc333",
    }),
    hot({
      seq: 12,
      event_id: "d11:release:6006",
      delivery_id: "d11",
      event: "release",
      action: "published",
      ref: null,
    }),
  ];
}

const ZERO_COUNTS: OutcomeCounts = {
  "issue-created": 0,
  "pr-created": 0,
  comment: 0,
  "description-edited": 0,
  "commit-pushed": 0,
  other: 0,
};

describe("outcome attribution projection (#2072 slice 4, #10910 spec)", () => {
  test("classifies every supported hot-list kind and keeps exact evidence links", () => {
    const projection = projectOutcomeAttribution({ sourceStatus: "ok", events: fixtureEvents() });
    expect(projection.attributionVersion).toBe(OUTCOME_ATTRIBUTION_VERSION);
    expect(projection.facts).toHaveLength(12);
    expect(projection.totalByKind).toEqual({
      ...ZERO_COUNTS,
      "issue-created": 1,
      "pr-created": 1,
      "description-edited": 2,
      comment: 3,
      "commit-pushed": 3,
      other: 2,
    });
    const byId = new Map(projection.facts.map((fact) => [fact.eventId, fact]));
    expect(byId.get("d1:issue:1001")).toMatchObject({
      factId: `outcome:${OUTCOME_ATTRIBUTION_VERSION}:d1:issue:1001`,
      outcomeKind: "issue-created",
      subjectKind: "issue",
      subjectRef: "developing-today/code#101",
      sourceUrl: ISSUE_URL,
      coverage: "complete",
    });
    expect(byId.get("d7:review:4004")).toMatchObject({ outcomeKind: "comment", subjectKind: "review" });
    expect(byId.get("d8:review_comment:5005")).toMatchObject({
      outcomeKind: "comment",
      subjectKind: "review-comment",
    });
    expect(byId.get("d9:commit:aaa111")).toMatchObject({ outcomeKind: "commit-pushed", coverage: "complete" });
    // The fallback push row counts once but flags the coverage partial:
    // it must never be rendered as an exact commit count.
    expect(byId.get("d10:push:ccc333")).toMatchObject({ outcomeKind: "commit-pushed", coverage: "partial" });
    expect(projection.partialCoverage).toBe(1);
    expect(byId.get("d11:release:6006")).toMatchObject({ outcomeKind: "other", subjectKind: "unknown" });
  });

  test("is deterministic and dedupes replayed event ids inside one sweep", () => {
    const events = fixtureEvents();
    const withReplay = [...events, { ...events[0]!, seq: 99 }];
    const first = projectOutcomeAttribution({ sourceStatus: "ok", events: withReplay });
    const second = projectOutcomeAttribution({ sourceStatus: "ok", events: withReplay });
    expect(second).toEqual(first);
    expect(first.facts).toHaveLength(12);
    expect(first.totalByKind?.["issue-created"]).toBe(1);
  });

  test("attaches job/model/session identities only through an authoritative binding", () => {
    const binding: OutcomeBinding = {
      eventId: "d1:issue:1001",
      taskId: "20799",
      jobId: null,
      sessionId: "ses_child",
      rootSessionId: "ses_root",
      parentSessionId: null,
      providerID: "anthropic",
      modelID: "claude-test",
      method: "queue-binding",
    };
    const projection = projectOutcomeAttribution({
      sourceStatus: "ok",
      events: fixtureEvents(),
      bindings: [binding],
    });
    const bound = projection.facts.find((fact) => fact.eventId === "d1:issue:1001");
    expect(bound).toMatchObject({
      taskId: "20799",
      sessionId: "ses_child",
      rootSessionId: "ses_root",
      providerID: "anthropic",
      modelID: "claude-test",
      method: "queue-binding",
      reason: null,
    });
    expect(projection.byJob.get("20799")).toEqual({ ...ZERO_COUNTS, "issue-created": 1 });
    expect(projection.bySession.get("ses_child")).toEqual({ ...ZERO_COUNTS, "issue-created": 1 });
    expect(projection.byModel.get("anthropic\u0000claude-test")).toEqual({
      ...ZERO_COUNTS,
      "issue-created": 1,
    });
    // Everything else stays unattributed with a reason; the GitHub actor
    // is never promoted into a model or session guess.
    expect(projection.unattributedByReason.get("no-binding")).toBe(11);
    for (const fact of projection.facts) {
      if (fact.eventId === "d1:issue:1001") continue;
      expect(fact.method).toBe("unattributed");
      expect(fact.reason).toBe("no-binding");
      expect(fact.modelID).toBeNull();
      expect(fact.sessionId).toBeNull();
    }
  });

  test("a missing or failed source is a source gap, never a zero-activity sweep", () => {
    for (const sourceStatus of ["missing", "error"] as const) {
      const projection = projectOutcomeAttribution({ sourceStatus, events: fixtureEvents() });
      expect(projection.totalByKind).toBeNull();
      expect(projection.facts).toHaveLength(1);
      expect(projection.facts[0]).toMatchObject({ method: "source-gap", reason: "source-gap" });
      expect(projection.byJob.size).toBe(0);
    }
    const empty = projectOutcomeAttribution({ sourceStatus: "ok", events: [] });
    expect(empty.totalByKind).toEqual(ZERO_COUNTS);
    expect(empty.facts).toHaveLength(0);
  });

  test("rejects malformed hot-list rows instead of minting facts from them", () => {
    expect(() => projectOutcomeAttribution({ sourceStatus: "ok", events: [hot({ event_id: "  " })] })).toThrow(
      "invalid_hot_event_id",
    );
    expect(() =>
      projectOutcomeAttribution({ sourceStatus: "ok", events: [hot({ event_id: "x", repo: "no-slash" })] }),
    ).toThrow("invalid_hot_event_repo");
  });
});

describe("outcome fact store", () => {
  let dir: string;
  let file: string;

  beforeEach(async () => {
    dir = await mkdtemp(join(tmpdir(), "outcome-facts-"));
    file = join(dir, "outcome-facts.jsonl");
  });

  afterEach(async () => {
    await rm(dir, { recursive: true, force: true });
  });

  test("appends idempotently, advances the cursor only after durable writes, and rebuilds counts", async () => {
    const store = new OutcomeFactStore(file);
    const events = fixtureEvents();
    const first = await store.applySweep({ sourceStatus: "ok", events });
    expect(first.appended).toBe(12);
    expect(first.cursor).toBe(12);
    expect(first.projection.totalByKind?.["commit-pushed"]).toBe(3);

    // Replay of the same sweep adds no fact and no count.
    const replay = await store.applySweep({ sourceStatus: "ok", events });
    expect(replay.appended).toBe(0);
    expect(replay.cursor).toBe(12);

    // A failed sweep appends no event facts, does not advance the
    // cursor, and records the gap fact exactly once across failures.
    const failed = await store.applySweep({ sourceStatus: "error", events: [] });
    expect(failed.appended).toBe(1);
    expect(failed.cursor).toBe(12);
    const failedAgain = await store.applySweep({ sourceStatus: "error", events: [] });
    expect(failedAgain.appended).toBe(0);
    expect(failedAgain.cursor).toBe(12);

    // Rebuild from the ledger alone reproduces the projected counts.
    const facts = await store.readFacts();
    expect(facts).toHaveLength(13);
    const rebuilt = rollupOutcomeFacts(facts);
    expect(rebuilt.totalByKind).toEqual(first.projection.totalByKind);
    expect(rebuilt.partialCoverage).toBe(1);

    const reopened = new OutcomeFactStore(file);
    expect(await reopened.cursor()).toBe(12);
  });

  test("a missing ledger reads as empty; a corrupt line is an explicit error", async () => {
    const store = new OutcomeFactStore(file);
    expect(await store.readFacts()).toEqual([]);
    expect(await store.cursor()).toBe(0);
    await writeFile(file, `${JSON.stringify({ factId: "x" })}\nnot json\n`, "utf8");
    await expect(store.readFacts()).rejects.toThrow("outcome_fact_store_corrupt");
  });

  test("the persisted ledger is append-only JSONL of the projected facts", async () => {
    const store = new OutcomeFactStore(file);
    await store.applySweep({ sourceStatus: "ok", events: fixtureEvents() });
    const lines = (await readFile(file, "utf8")).trim().split("\n");
    expect(lines).toHaveLength(12);
    for (const line of lines) {
      const fact = JSON.parse(line) as { factId: string; eventId: string };
      expect(fact.factId).toBe(`outcome:${OUTCOME_ATTRIBUTION_VERSION}:${fact.eventId}`);
    }
  });
});
