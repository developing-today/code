import { describe, expect, test } from "bun:test";
import { buildBriefs, buildItemBrief } from "../src/brief.ts";
import type { Comment, OrchestrationSection, ProjectPoolItem } from "../src/model.ts";

// Fixture tests for the per-item brief (#1279): blocked, in-flight, and
// idle items, plus the honest-unknowns and disagreement contracts.

function item(partial: Partial<ProjectPoolItem> & { number: number }): ProjectPoolItem {
  return {
    id: `PVTI_${partial.number}`,
    url: `https://github.com/orgs/developing-today/projects/4?pane=issue&itemId=${partial.number}`,
    projectUrl: "https://github.com/orgs/developing-today/projects/4",
    ref: `developing-today/code#${partial.number}`,
    repo: "developing-today/code",
    issueUrl: `https://github.com/developing-today/code/issues/${partial.number}`,
    title: `item ${partial.number}`,
    state: "open",
    contentKind: "issue",
    status: null,
    area: null,
    horizon: null,
    kind: "Task",
    ...partial,
  };
}

function comment(partial: Partial<Comment> & { ref: string }): Comment {
  return {
    id: "c1",
    kind: "issue_comment",
    author: "dezren39",
    body: "working on it",
    createdAt: "2026-10-30T10:00:00Z",
    url: "https://github.com/developing-today/code/issues/1#issuecomment-1",
    ...partial,
  };
}

const orchestrationWith = (ref: string): OrchestrationSection => ({
  version: 1,
  generatedAt: "2026-10-30T09:00:00Z",
  runId: "orch-2026-10-30T09:00:00Z",
  provenance: { sources: [] },
  tiers: {
    needsDrew: [],
    needsOrchestrator: [
      {
        id: ref,
        kind: "task",
        title: "in flight",
        source: "github",
        reason: "picked up",
        since: "2026-10-30T08:00:00Z",
        lastSeenAt: "2026-10-30T09:00:00Z",
        ref,
      } as OrchestrationSection["tiers"]["needsOrchestrator"][number],
    ],
  },
  dependencies: [],
  goals: [],
  actions: [],
});

describe("buildItemBrief", () => {
  test("blocked item: blocker named + linked, next action points at it", () => {
    const blocker = item({ number: 10, state: "open" });
    const blockedItem = item({ number: 11, blockedBy: ["developing-today/code#10"] });
    const brief = buildItemBrief(blockedItem, { pool: [blocker, blockedItem] });
    expect(brief.derived).toBe(true);
    expect(brief.blocker.blocked).toBe(true);
    expect(brief.blocker.blockers).toEqual([
      { ref: "developing-today/code#10", url: "https://github.com/developing-today/code/issues/10", state: "open" },
    ]);
    expect(brief.blocker.text).toContain("developing-today/code#10");
    expect(brief.nextAction.text).toContain("developing-today/code#10");
    expect(brief.nextAction.evidence.url).toBe("https://github.com/developing-today/code/issues/10");
  });

  test("blocker that is closed no longer blocks", () => {
    const blocker = item({ number: 10, state: "closed" });
    const blockedItem = item({ number: 11, blockedBy: ["developing-today/code#10"] });
    const brief = buildItemBrief(blockedItem, { pool: [blocker, blockedItem] });
    expect(brief.blocker.blocked).toBe(false);
    expect(brief.blocker.text).toContain("Not blocked");
  });

  test("in-flight item: derived In Progress, last actor from latest comment", () => {
    const it = item({ number: 12, updatedAt: "2026-10-30T10:30:00Z" });
    const brief = buildItemBrief(it, {
      orchestration: orchestrationWith(it.ref),
      comments: [
        comment({ ref: it.ref, author: "alice", createdAt: "2026-10-29T09:00:00Z" }),
        comment({ ref: it.ref, author: "bob", createdAt: "2026-10-30T10:00:00Z", url: "https://example.test/c2" }),
        comment({ ref: "developing-today/code#999", author: "mallory", createdAt: "2026-10-30T11:00:00Z" }),
      ],
    });
    expect(brief.state.value).toBe("In Progress");
    expect(brief.lastActivity.actor).toBe("bob");
    expect(brief.lastActivity.at).toBe("2026-10-30T10:00:00Z");
    expect(brief.lastActivity.evidence?.url).toBe("https://example.test/c2");
    expect(brief.nextAction.text).toContain("in-flight");
  });

  test("idle item with no activity: unknowns render as unknown", () => {
    const brief = buildItemBrief(item({ number: 13 }));
    expect(brief.state.value).toBe("Todo");
    expect(brief.blocker.blocked).toBe(false);
    expect(brief.lastActivity.actor).toBeNull();
    expect(brief.lastActivity.at).toBeNull();
    expect(brief.lastActivity.text).toContain("Unknown");
    expect(brief.lastActivity.evidence).toBeNull();
    expect(brief.nextAction.text).toContain("Triage and pick up");
  });

  test("idle item with only an update time: actor unknown, time kept", () => {
    const brief = buildItemBrief(item({ number: 14, updatedAt: "2026-10-28T00:00:00Z" }));
    expect(brief.lastActivity.actor).toBeNull();
    expect(brief.lastActivity.at).toBe("2026-10-28T00:00:00Z");
    expect(brief.lastActivity.text).toContain("actor unknown");
  });

  test("disagreement is stated with both sides, never resolved", () => {
    const brief = buildItemBrief(item({ number: 15, status: "In Review" }));
    expect(brief.disagreement).not.toBeNull();
    expect(brief.disagreement?.project).toBe("In Review");
    expect(brief.disagreement?.derived).toBe("Todo");
    expect(brief.disagreement?.text).toContain("In Review");
    expect(brief.disagreement?.text).toContain("Todo");
    expect(brief.nextAction.text).toContain("Reconcile");
  });

  test("done item: state Done and no action suggested", () => {
    const brief = buildItemBrief(item({ number: 16, state: "merged", contentKind: "pr" }));
    expect(brief.state.value).toBe("Done");
    expect(brief.nextAction.text).toContain("No action");
  });

  test("every claim carries an evidence link", () => {
    const briefs = buildBriefs([item({ number: 17 }), item({ number: 18, blockedBy: ["developing-today/code#17"] })]);
    for (const brief of briefs.values()) {
      expect(brief.state.evidence.url).toStartWith("https://");
      expect(brief.blocker.evidence.url).toStartWith("https://");
      expect(brief.nextAction.evidence.url).toStartWith("https://");
    }
  });
});
