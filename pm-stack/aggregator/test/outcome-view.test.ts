import { describe, expect, test } from "bun:test";
import type { GithubHotEvent } from "../src/activity.ts";
import {
  OUTCOME_ATTRIBUTION_VERSION,
  projectOutcomeAttribution,
  type OutcomeBinding,
  type OutcomeFact,
} from "../src/outcome-attribution.ts";
import { buildOutcomeView, drillOutcomeFacts } from "../src/outcome-view.ts";

/** Fixture hot-list rows in the projector's normalized shape (#5204),
 *  mirroring test/outcome-attribution.test.ts. The oracle is exact:
 *  every count and every drill row below is derived by hand from these
 *  six events and the three bindings. */
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

function fixtureFacts(): OutcomeFact[] {
  const events: GithubHotEvent[] = [
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
      event_id: "d3:pr:2002",
      delivery_id: "d3",
      event: "pull_request",
      action: "opened",
      ref: "developing-today/code#202",
      source_url: PR_URL,
    }),
    hot({
      seq: 4,
      event_id: "d4:issue_comment:3003",
      delivery_id: "d4",
      event: "issue_comment",
      action: "created",
      ref: "developing-today/code#101",
      source_url: `${ISSUE_URL}#issuecomment-3003`,
    }),
    hot({
      seq: 5,
      event_id: "d5:commit:aaa111",
      delivery_id: "d5",
      event: "push",
      action: "push",
      ref: "refs/heads/felix/example",
      source_url: "https://github.com/developing-today/code/commit/aaa111",
    }),
    // Zero-commit fallback row: partial coverage by construction.
    hot({
      seq: 6,
      event_id: "d6:push:ccc333",
      delivery_id: "d6",
      event: "push",
      action: "push",
      ref: "refs/heads/main",
      source_url: "https://github.com/developing-today/code/commit/ccc333",
    }),
  ];
  const bindings: OutcomeBinding[] = [
    {
      eventId: "d1:issue:1001",
      taskId: "task-1",
      sessionId: "sess-1",
      providerID: "anthropic",
      modelID: "claude-x",
      method: "queue-binding",
    },
    {
      eventId: "d2:issue:1001",
      taskId: "task-1",
      sessionId: "sess-1",
      providerID: "anthropic",
      modelID: "claude-x",
      method: "queue-binding",
    },
    {
      eventId: "d3:pr:2002",
      taskId: "task-1",
      sessionId: "sess-2",
      providerID: "anthropic",
      modelID: "claude-x",
      method: "queue-binding",
    },
    {
      eventId: "d5:commit:aaa111",
      taskId: "task-2",
      sessionId: "sess-1",
      providerID: "openai",
      modelID: "gpt-y",
      method: "queue-binding",
    },
  ];
  return projectOutcomeAttribution({ sourceStatus: "ok", events, bindings }).facts;
}

const fid = (eventId: string) => `outcome:${OUTCOME_ATTRIBUTION_VERSION}:${eventId}`;

describe("outcome view (#10910 rollup view)", () => {
  test("totals and groups match the hand-derived oracle", () => {
    const view = buildOutcomeView(fixtureFacts());
    expect(view.state).toBe("ok");
    expect(view.attributionVersion).toBe(OUTCOME_ATTRIBUTION_VERSION);
    expect(view.totalFacts).toBe(6);
    expect(view.totalByKind).toEqual({
      "issue-created": 1,
      "pr-created": 1,
      comment: 1,
      "description-edited": 1,
      "commit-pushed": 2,
      other: 0,
    });
    expect(view.partialCoverage).toBe(1);
    expect(view.sourceGaps).toBe(0);

    expect(view.byJob.map((g) => [g.key, g.total])).toEqual([
      ["task-1", 3],
      ["task-2", 1],
    ]);
    expect(view.byJob[0].factIds).toEqual([fid("d1:issue:1001"), fid("d2:issue:1001"), fid("d3:pr:2002")]);

    // The model key is the readable pair; the NUL-joined rollup key
    // never appears in the view.
    expect(view.byModel.map((g) => [g.key, g.providerID, g.modelID, g.total])).toEqual([
      ["anthropic/claude-x", "anthropic", "claude-x", 3],
      ["openai/gpt-y", "openai", "gpt-y", 1],
    ]);

    expect(view.bySession.map((g) => [g.key, g.total])).toEqual([
      ["sess-1", 3],
      ["sess-2", 1],
    ]);

    expect(view.unattributed.total).toBe(2);
    expect(view.unattributed.byReason).toEqual([
      { reason: "no-binding", count: 2, factIds: [fid("d4:issue_comment:3003"), fid("d6:push:ccc333")] },
    ]);
  });

  test("an empty ledger is absence, not zero activity", () => {
    const view = buildOutcomeView([]);
    expect(view.state).toBe("empty");
    expect(view.totalByKind).toBeNull();
    expect(view.totalFacts).toBe(0);
    expect(view.byJob).toEqual([]);
    expect(view.sourceGaps).toBe(0);
  });

  test("a gap-only ledger renders as a gap, never as zeros", () => {
    const gapFacts = projectOutcomeAttribution({ sourceStatus: "error", events: [] }).facts;
    const view = buildOutcomeView(gapFacts);
    expect(view.state).toBe("source-gap");
    expect(view.totalByKind).toBeNull();
    expect(view.sourceGaps).toBe(1);
    expect(drillOutcomeFacts(gapFacts, { dimension: "job", key: "task-1" })).toEqual([]);
  });

  test("recorded gaps stay visible after later sweeps land facts", () => {
    const gapFacts = projectOutcomeAttribution({ sourceStatus: "missing", events: [] }).facts;
    const view = buildOutcomeView([...gapFacts, ...fixtureFacts()]);
    expect(view.state).toBe("ok");
    expect(view.sourceGaps).toBe(1);
    expect(view.totalFacts).toBe(6);
  });

  test("superseded-version facts are not this version's evidence", () => {
    const stale = fixtureFacts().map((fact) => ({ ...fact, attributionVersion: "v0" }));
    const view = buildOutcomeView(stale);
    expect(view.state).toBe("empty");
    expect(view.totalByKind).toBeNull();
    expect(drillOutcomeFacts(stale, { dimension: "issue", subjectRef: "developing-today/code#101" })).toEqual([]);
  });
});

describe("outcome drill-through (#10910)", () => {
  const facts = fixtureFacts();

  test("job drill returns the exact facts with their evidence links", () => {
    const rows = drillOutcomeFacts(facts, { dimension: "job", key: "task-1" });
    expect(rows.map((r) => r.eventId)).toEqual(["d1:issue:1001", "d2:issue:1001", "d3:pr:2002"]);
    expect(rows[0]).toMatchObject({
      factId: fid("d1:issue:1001"),
      outcomeKind: "issue-created",
      subjectRef: "developing-today/code#101",
      sourceUrl: ISSUE_URL,
      taskId: "task-1",
      sessionId: "sess-1",
      providerID: "anthropic",
      modelID: "claude-x",
    });
    expect(rows[2].sourceUrl).toBe(PR_URL);
  });

  test("issue drill crosses kinds and attribution states for one subject", () => {
    const rows = drillOutcomeFacts(facts, { dimension: "issue", subjectRef: "developing-today/code#101" });
    expect(rows.map((r) => r.eventId)).toEqual(["d1:issue:1001", "d2:issue:1001", "d4:issue_comment:3003"]);
    expect(rows.map((r) => r.outcomeKind)).toEqual(["issue-created", "description-edited", "comment"]);
    expect(rows[2].method).toBe("unattributed");
    expect(rows[2].sourceUrl).toBe(`${ISSUE_URL}#issuecomment-3003`);
  });

  test("model and session drills select exactly their group members", () => {
    expect(drillOutcomeFacts(facts, { dimension: "model", providerID: "openai", modelID: "gpt-y" }).map((r) => r.eventId)).toEqual([
      "d5:commit:aaa111",
    ]);
    expect(drillOutcomeFacts(facts, { dimension: "session", key: "sess-2" }).map((r) => r.eventId)).toEqual(["d3:pr:2002"]);
    expect(drillOutcomeFacts(facts, { dimension: "model", providerID: "anthropic", modelID: "gpt-y" })).toEqual([]);
  });

  test("unattributed drill keeps partial coverage visible", () => {
    const rows = drillOutcomeFacts(facts, { dimension: "unattributed", reason: null });
    expect(rows.map((r) => r.eventId)).toEqual(["d4:issue_comment:3003", "d6:push:ccc333"]);
    expect(rows[1].coverage).toBe("partial");
    expect(drillOutcomeFacts(facts, { dimension: "unattributed", reason: "no-binding" })).toHaveLength(2);
    expect(drillOutcomeFacts(facts, { dimension: "unattributed", reason: "source-gap" })).toEqual([]);
  });
});
