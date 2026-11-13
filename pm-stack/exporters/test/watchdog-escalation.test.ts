// Rung-4 escalation hand-off (#6033) — oracle: the acceptance criteria
// on issue #6033, exercised against fixture health records and counting
// fake effectors. No queue, network, or browser is touched.
import { describe, expect, test } from "bun:test";
import {
  applyEscalationActions,
  buildEscalationPacket,
  ESCALATION_NEEDS_DREW_AFTER_MS,
  planEscalation,
  planRecovery,
  recordEscalation,
  surfacingLine,
  type EscalationContext,
  type EscalationEffectors,
} from "../src/watchdog-escalation.ts";
import { coerceHealth, initialHealth, type SourceHealth } from "../src/watchdog-state.ts";

const CTX: EscalationContext = {
  issueRepo: "developing-today-forks/openchamber",
  stateFilePath: "/home/user/atlas/watchdog-state.json",
  adapterRef: "pm-stack/exporters/adapters/mistral.ts",
  historyRef: "quota-watchdog-history.jsonl (tail)",
};

const T0 = "2026-11-10T00:00:00Z";

function escalatingHealth(): SourceHealth {
  const h = initialHealth("mistral");
  return {
    ...h,
    consecutiveFailures: 9,
    currentClass: "session-expired",
    lastDetail: "login redirected to auth wall",
    lastAttemptAt: T0,
    repairLog: [
      { at: T0, rung: 1, outcome: "failed", detail: "retry: still redirected" },
      { at: T0, rung: 3, outcome: "failed", detail: "re-login: session not accepted" },
      { at: T0, rung: 3, outcome: "failed", detail: "all rungs exhausted; escalating" },
    ],
    incident: {
      id: "mistral:2026-11-09T20:00:00Z",
      openedAt: "2026-11-09T20:00:00Z",
      humanBlocked: false,
      surfacing: "escalate",
      escalation: null,
    },
  };
}

function countingEffectors() {
  const calls: string[] = [];
  const effectors: EscalationEffectors = {
    enqueueTask: async () => {
      calls.push("enqueue");
      return "task-4242";
    },
    openIssue: async () => {
      calls.push("open");
      return 777;
    },
    commentIssue: async () => {
      calls.push("comment");
    },
    closeIssue: async () => {
      calls.push("close");
    },
    completeTask: async () => {
      calls.push("complete");
    },
  };
  return { calls, effectors };
}

describe("planEscalation + dedupe (#6033 acceptance 1)", () => {
  test("first escalation plans exactly one task and one issue; the recorded second creates neither", async () => {
    const health = escalatingHealth();
    const plan = planEscalation(health, CTX);
    expect(plan.deduped).toBe(false);
    expect(plan.actions.map((a) => a.type)).toEqual(["enqueue_task", "open_issue"]);

    const { calls, effectors } = countingEffectors();
    const refs = await applyEscalationActions(plan.actions, effectors);
    expect(calls).toEqual(["enqueue", "open"]);
    expect(refs).toEqual({ queueTaskId: "task-4242", issueNumber: 777 });

    const recorded = recordEscalation(health, CTX, refs, T0);
    expect(recorded.incident?.escalation?.class).toBe("session-expired");
    const again = planEscalation(recorded, CTX);
    expect(again.deduped).toBe(true);
    expect(again.actions).toEqual([]);
  });

  test("a different failure class for the same source is a new pair and plans again", () => {
    const recorded = recordEscalation(escalatingHealth(), CTX, { queueTaskId: "t", issueNumber: 1 }, T0);
    const changed: SourceHealth = { ...recorded, currentClass: "endpoint-changed" };
    const plan = planEscalation(changed, CTX);
    expect(plan.deduped).toBe(false);
    expect(plan.actions).toHaveLength(2);
  });

  test("no plan when the incident is quiet, absent, or human-blocked", () => {
    const quiet = initialHealth("mistral");
    expect(planEscalation(quiet, CTX).actions).toEqual([]);
    const hb = escalatingHealth();
    hb.incident = { ...hb.incident!, humanBlocked: true, surfacing: "human-blocked" };
    expect(planEscalation(hb, CTX).actions).toEqual([]);
  });
});

describe("packet completeness (#6033 acceptance 2)", () => {
  test("packet carries outcome, failure context, capability, acceptance, stop conditions", () => {
    const packet = buildEscalationPacket(escalatingHealth(), CTX);
    expect(packet.outcomeWanted).toContain("mistral");
    expect(packet.outcomeWanted).toContain("latest.json");
    expect(packet.failure.class).toBe("session-expired");
    expect(packet.failure.consecutiveFailures).toBe(9);
    expect(packet.failure.repairLog).toHaveLength(3);
    expect(packet.failure.stateFilePath).toBe(CTX.stateFilePath);
    expect(packet.failure.adapterRef).toBe(CTX.adapterRef);
    expect(packet.failure.historyRef).toBe(CTX.historyRef);
    expect(packet.requiredCapability).toContain("browser-enabled");
    expect(packet.requiredCapability).toContain(":19222");
    expect(packet.acceptance).toContain("auto-closes");
    expect(packet.stopConditions.join(" ")).toContain("human-only challenge");
    expect(packet.stopConditions.join(" ")).toContain("origin allowlist");
    expect(packet.dedupeKey).toBe("watchdog-escalation:mistral:session-expired");
  });
});

describe("recovery (#6033 acceptance 3)", () => {
  test("producer success plans comment + close + task completion against the recorded refs", async () => {
    const recorded = recordEscalation(
      escalatingHealth(),
      CTX,
      { queueTaskId: "task-4242", issueNumber: 777 },
      T0,
    );
    const actions = planRecovery(recorded, "2026-11-10T06:00:00Z");
    expect(actions.map((a) => a.type)).toEqual(["comment_issue", "close_issue", "complete_task"]);
    const { calls, effectors } = countingEffectors();
    await applyEscalationActions(actions, effectors);
    expect(calls).toEqual(["comment", "close", "complete"]);
  });

  test("no recovery actions when nothing was escalated", () => {
    expect(planRecovery(escalatingHealth(), T0)).toEqual([]);
    expect(planRecovery(initialHealth("mistral"), T0)).toEqual([]);
  });
});

describe("surfacing (#6033 acceptance 4 + 5)", () => {
  test("human-blocked surfaces the same tick, with a one-line reason", () => {
    const hb = escalatingHealth();
    hb.incident = { ...hb.incident!, humanBlocked: true, surfacing: "human-blocked" };
    const line = surfacingLine(hb, Date.parse(T0));
    expect(line).toContain("NEEDS DREW");
    expect(line).toContain("human-blocked");
    expect(line).not.toContain("\n");
  });

  test("ordinary escalation is quiet before 24h and surfaces at the threshold", () => {
    const recorded = recordEscalation(
      escalatingHealth(),
      CTX,
      { queueTaskId: "task-4242", issueNumber: 777 },
      T0,
    );
    const t0 = Date.parse(T0);
    expect(surfacingLine(recorded, t0 + ESCALATION_NEEDS_DREW_AFTER_MS - 1)).toBeNull();
    const line = surfacingLine(recorded, t0 + ESCALATION_NEEDS_DREW_AFTER_MS);
    expect(line).toContain("NEEDS DREW");
    expect(line).toContain("777");
  });
});

describe("state coercion keeps the escalation record", () => {
  test("coerceHealth round-trips a valid escalation and drops a malformed one", () => {
    const recorded = recordEscalation(
      escalatingHealth(),
      CTX,
      { queueTaskId: "task-4242", issueNumber: 777 },
      T0,
    );
    const back = coerceHealth("mistral", JSON.parse(JSON.stringify(recorded)));
    expect(back.incident?.escalation?.queueTaskId).toBe("task-4242");
    expect(back.incident?.escalation?.issueNumber).toBe(777);

    const corrupt = JSON.parse(JSON.stringify(recorded));
    corrupt.incident.escalation = { class: "not-a-class", since: 5 };
    const dropped = coerceHealth("mistral", corrupt);
    expect(dropped.incident?.escalation).toBeNull();
  });
});
