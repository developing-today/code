import { describe, expect, test } from "bun:test";
import type { ProjectPoolItem, RefKey } from "../src/model.ts";
import {
  ATTRIBUTION_VERSION,
  projectSpendAttribution,
  type SpendSession,
  type SpendTokens,
} from "../src/spend-attribution.ts";

const R = (n: number): RefKey => `developing-today/code#${n}`;
const tokens = (input: number): SpendTokens => ({
  input,
  output: input + 1,
  reasoning: input + 2,
  cacheRead: input + 3,
  cacheWrite: input + 4,
});

function session(partial: Partial<SpendSession> & Pick<SpendSession, "id">): SpendSession {
  return {
    parentId: null,
    directory: null,
    providerID: "openai",
    modelID: "gpt-test",
    tokens: tokens(1),
    ...partial,
  };
}

function item(partial: Partial<ProjectPoolItem> & Pick<ProjectPoolItem, "number">): ProjectPoolItem {
  const { number, ...rest } = partial;
  return {
    id: `item-${number}`,
    url: "https://example.test/item",
    projectUrl: "https://example.test/project",
    ref: partial.ref ?? R(number),
    repo: "developing-today/code",
    number,
    issueUrl: `https://github.com/developing-today/code/issues/${number}`,
    title: `issue ${number}`,
    state: "open",
    contentKind: "issue",
    status: null,
    area: "display-only-area",
    horizon: null,
    kind: "Task",
    ...rest,
  };
}

describe("spend attribution projection", () => {
  test("prefers an exact lane binding and exposes issue, epic, and explicit project facts", () => {
    const projection = projectSpendAttribution({
      sourceStatus: "ok",
      sessions: [session({ id: "root", directory: "/home/user/git/fleet/12-99-work", tokens: tokens(3) })],
      bindings: [{ sessionId: "root", taskId: "12", issueRef: R(2) }],
      pool: [item({ number: 1, kind: "Epic" }), item({ number: 2, epicRef: R(1) })],
      projectByRef: new Map([[R(2), "pm-stack"]]),
    });

    expect(projection.facts[0]).toMatchObject({
      attributionVersion: ATTRIBUTION_VERSION,
      method: "queue-binding",
      issueRef: R(2),
      epicRef: R(1),
      projectId: "pm-stack",
      tokens: tokens(3),
    });
    expect(projection.byIssue.get(R(2))).toEqual(tokens(3));
    expect(projection.byEpic.get(R(1))).toEqual(tokens(3));
    expect(projection.byProject.get("pm-stack")).toEqual(tokens(3));
    expect(projection.reconciled).toBe(true);
  });

  test("resolves a fleet worktree only when its issue number maps uniquely in the pool", () => {
    const input = {
      sourceStatus: "ok" as const,
      sessions: [session({ id: "root", directory: "~/git/fleet/12-42-task-work", tokens: tokens(2) })],
      bindings: [],
      pool: [item({ number: 42 })],
    };
    const projection = projectSpendAttribution({
      ...input,
    });
    expect(projection.facts[0]).toMatchObject({ method: "fleet-worktree", issueRef: R(42) });
    expect(projection.reconciled).toBe(true);
    expect(projectSpendAttribution(input)).toEqual(projection);

    const ambiguous = projectSpendAttribution({
      ...input,
      pool: [
        item({ number: 42 }),
        item({ number: 42, ref: "developing-today/opencode#42", repo: "developing-today/opencode" }),
      ],
    });
    expect(ambiguous.facts[0]).toMatchObject({ method: "unattributed", issueRef: null, reason: "no-worktree" });
  });

  test("rolls nested sessions into their root once and reconciles every token dimension", () => {
    const projection = projectSpendAttribution({
      sourceStatus: "ok",
      sessions: [
        session({ id: "root", tokens: tokens(1) }),
        session({ id: "child", parentId: "root", tokens: tokens(2) }),
        session({ id: "grandchild", parentId: "child", tokens: tokens(4) }),
      ],
      bindings: [{ sessionId: "root", taskId: "1", issueRef: R(9) }],
      pool: [item({ number: 9 })],
    });
    const fact = projection.facts[0]!;
    expect(fact.sessionIds).toEqual(["child", "grandchild", "root"]);
    expect(fact.tokens).toEqual({ input: 7, output: 10, reasoning: 13, cacheRead: 16, cacheWrite: 19 });
    expect(projection.sourceTotals).toEqual(fact.tokens);
    expect(projection.reconciled).toBe(true);
  });

  test("keeps a child whose session parent is outside the fetched pool explicit", () => {
    const projection = projectSpendAttribution({
      sourceStatus: "ok",
      sessions: [session({ id: "child", parentId: "missing-root", tokens: tokens(5) })],
      bindings: [],
      pool: [],
    });
    expect(projection.facts[0]).toMatchObject({
      rootSessionId: "missing-root",
      method: "unattributed",
      reason: "parent-left-pool",
      tokens: tokens(5),
    });
    expect(projection.reconciled).toBe(true);
  });

  test("distinguishes no binding from a fleet directory that cannot resolve", () => {
    const projection = projectSpendAttribution({
      sourceStatus: "ok",
      sessions: [
        session({ id: "interactive", directory: "/home/user/project", tokens: tokens(1) }),
        session({ id: "unknown-fleet", directory: "~/git/fleet/12-404-no-board-item", tokens: tokens(2) }),
      ],
      bindings: [],
      pool: [],
    });
    expect(projection.facts.map((fact) => fact.reason)).toEqual(["no-binding", "no-worktree"]);
    expect(projection.unattributedByReason.get("no-binding")).toEqual(tokens(1));
    expect(projection.unattributedByReason.get("no-worktree")).toEqual(tokens(2));
    expect(projection.reconciled).toBe(true);
  });

  test("marks an incomplete GitHub parent chain without guessing an epic or project", () => {
    const projection = projectSpendAttribution({
      sourceStatus: "ok",
      sessions: [session({ id: "root", directory: "~/git/fleet/12-42-task-work", tokens: tokens(2) })],
      bindings: [{ sessionId: "root", taskId: "12", issueRef: R(42) }],
      pool: [item({ number: 42, parentRef: R(1) })],
      projectByRef: new Map([[R(42), "pm-stack"]]),
    });
    expect(projection.facts[0]).toMatchObject({
      issueRef: R(42),
      epicRef: null,
      projectId: "pm-stack",
      reason: "parent-left-pool",
    });
    expect(projection.byIssue.get(R(42))).toEqual(tokens(2));
    expect(projection.byEpic.size).toBe(0);
    expect(projection.reconciled).toBe(true);
  });

  test("splits a root group by provider and model while keeping root issue attribution", () => {
    const projection = projectSpendAttribution({
      sourceStatus: "ok",
      sessions: [
        session({ id: "root", providerID: "openai", modelID: "gpt-test", tokens: tokens(1) }),
        session({ id: "child", parentId: "root", providerID: "anthropic", modelID: "claude-test", tokens: tokens(2) }),
      ],
      bindings: [{ sessionId: "root", taskId: "1", issueRef: R(9) }],
      pool: [item({ number: 9 })],
    });
    expect(projection.facts).toHaveLength(2);
    expect(projection.facts[0]).toMatchObject({
      providerID: "anthropic",
      modelID: "claude-test",
      sessionIds: ["child"],
      issueRef: R(9),
      method: "queue-binding",
      tokens: tokens(2),
    });
    expect(projection.facts[1]).toMatchObject({
      providerID: "openai",
      modelID: "gpt-test",
      sessionIds: ["root"],
      issueRef: R(9),
      method: "queue-binding",
      tokens: tokens(1),
    });
    expect(projection.byIssue.get(R(9))).toEqual({ input: 3, output: 5, reasoning: 7, cacheRead: 9, cacheWrite: 11 });
    expect(projection.sourceTotals).toEqual({ input: 3, output: 5, reasoning: 7, cacheRead: 9, cacheWrite: 11 });
    expect(projection.reconciled).toBe(true);
  });

  test("consults the exact binding for a root outside the source window", () => {
    const projection = projectSpendAttribution({
      sourceStatus: "ok",
      sessions: [session({ id: "child", parentId: "missing-root", tokens: tokens(5) })],
      bindings: [{ sessionId: "missing-root", taskId: "7", issueRef: R(7) }],
      pool: [item({ number: 7 })],
    });
    expect(projection.facts[0]).toMatchObject({
      rootSessionId: "missing-root",
      method: "queue-binding",
      issueRef: R(7),
      reason: "parent-left-pool",
      tokens: tokens(5),
    });
    expect(projection.byIssue.get(R(7))).toEqual(tokens(5));
    expect(projection.reconciled).toBe(true);
  });

  test("represents missing or failed usage as unknown source-gap, not zero spend", () => {
    for (const sourceStatus of ["missing", "error"] as const) {
      const projection = projectSpendAttribution({ sourceStatus, sessions: [], bindings: [], pool: [] });
      expect(projection.sourceTotals).toBeNull();
      expect(projection.reconciled).toBeNull();
      expect(projection.facts[0]).toMatchObject({ reason: "source-gap", tokens: null });
    }
    const emptySuccess = projectSpendAttribution({ sourceStatus: "ok", sessions: [], bindings: [], pool: [] });
    expect(emptySuccess.sourceTotals).toEqual({ input: 0, output: 0, reasoning: 0, cacheRead: 0, cacheWrite: 0 });
    expect(emptySuccess.reconciled).toBe(true);
  });
});
