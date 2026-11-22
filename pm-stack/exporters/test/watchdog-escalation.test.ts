// Rung-4 escalation hand-off (#6033) — oracle: the acceptance criteria
// on issue #6033, exercised against fixture health records and counting
// fake effectors. No queue, network, or browser is touched.
import { describe, expect, test } from "bun:test";
import {
  applyEscalationActions,
  buildEscalationPacket,
  buildQueueTaskWrapper,
  EscalationPartialError,
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
  };
  return { calls, effectors };
}

describe("planEscalation + dedupe (#6033 acceptance 1)", () => {
  test("first escalation plans exactly one task and one issue; the recorded second creates neither", async () => {
    const health = escalatingHealth();
    const plan = planEscalation(health, CTX);
    expect(plan.deduped).toBe(false);
    expect(plan.actions.map((a) => a.type)).toEqual(["open_issue", "enqueue_task"]);

    const { calls, effectors } = countingEffectors();
    const refs = await applyEscalationActions(plan.actions, effectors);
    expect(calls).toEqual(["open", "enqueue"]);
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
    expect(packet.acceptance).toContain("authorized disposition");
    expect(packet.stopConditions.join(" ")).toContain("human-only challenge");
    expect(packet.stopConditions.join(" ")).toContain("origin allowlist");
    expect(packet.dedupeKey).toBe("watchdog-escalation:mistral:session-expired");
    expect(packet.issueRepo).toBe(CTX.issueRepo);
    // A fresh plan has no incident issue yet: it is created first in
    // the hand-off and its number is stamped in at apply time.
    expect(packet.issueNumber).toBeNull();
  });
});

describe("queue task wrapper — the repo-task/v1 contract (task 20118)", () => {
  test("apply stamps the confirmed issue number into the enqueue packet", async () => {
    const seen: (number | null)[] = [];
    const effectors: EscalationEffectors = {
      enqueueTask: async (packet) => {
        seen.push(packet.issueNumber);
        return "task-4242";
      },
      openIssue: async () => 777,
      commentIssue: async () => {},
    };
    const plan = planEscalation(escalatingHealth(), CTX);
    const refs = await applyEscalationActions(plan.actions, effectors);
    expect(seen).toEqual([777]);
    expect(refs).toEqual({ queueTaskId: "task-4242", issueNumber: 777 });
  });

  test("the wrapper names the incident issue in its dedupe key and payload identity", () => {
    const packet = { ...buildEscalationPacket(escalatingHealth(), CTX), issueNumber: 777 };
    const w = buildQueueTaskWrapper(packet, T0);
    expect(w.dedupe_key).toBe("repo-health:v1:fix:developing-today-forks/openchamber:issue:777");
    expect(w.priority).toBe(30);
    const p = w.payload as Record<string, any>;
    expect(p.schema).toBe("repo-task/v1");
    expect(p.kind).toBe("small_fix");
    expect(p.repo).toBe(CTX.issueRepo);
    expect(p.issue).toEqual({ number: 777 });
    expect(typeof p.problem).toBe("string");
    expect((p.problem as string).length).toBeGreaterThan(0);
    expect(typeof p.priority_reason).toBe("string");
    for (const f of ["evidence", "acceptance", "tests", "context_refs"]) {
      expect(Array.isArray(p[f])).toBe(true);
      expect((p[f] as unknown[]).length).toBeGreaterThan(0);
      expect((p[f] as unknown[]).every((e) => typeof e === "string")).toBe(true);
    }
    expect((p.scope as any).include.length).toBeGreaterThan(0);
    expect((p.scope as any).exclude.length).toBeGreaterThan(0);
    expect((p.risk as any).level).toBe("low");
    expect(p.timebox_minutes).toBeGreaterThan(0);
    expect((p.dedupe as any).checked_at).toBe(T0);
    expect(((p.dedupe as any).basis as string).length).toBeGreaterThan(0);
    // The full escalation context survives inside the payload, so a
    // browser-enabled agent can start from the claimed task alone.
    expect((p.escalation_packet as any).dedupeKey).toBe("watchdog-escalation:mistral:session-expired");
    expect((p.escalation_packet as any).failure.stateFilePath).toBe(CTX.stateFilePath);
    expect((p.escalation_packet as any).requiredCapability).toContain("browser-enabled");
  });

  test("the wrapper refuses a packet whose incident issue does not exist yet", () => {
    const packet = buildEscalationPacket(escalatingHealth(), CTX);
    expect(packet.issueNumber).toBeNull();
    expect(() => buildQueueTaskWrapper(packet, T0)).toThrow(/no incident issue number/);
  });

  test("a resume packet carries the recorded issue number, so the retry needs no new issue", () => {
    const recorded = recordEscalation(escalatingHealth(), CTX, { queueTaskId: null, issueNumber: 777 }, T0);
    const plan = planEscalation(recorded, CTX);
    expect(plan.actions.map((a) => a.type)).toEqual(["enqueue_task"]);
    const action = plan.actions[0] as Extract<(typeof plan.actions)[number], { type: "enqueue_task" }>;
    expect(action.packet.issueNumber).toBe(777);
    expect(action.packet.issueRepo).toBe(CTX.issueRepo);
  });
});

describe("recovery (#6033 acceptance 3)", () => {
  test("producer success records recovery without closing the issue or completing its task", async () => {
    const recorded = recordEscalation(escalatingHealth(), CTX, { queueTaskId: "task-4242", issueNumber: 777 }, T0);
    const actions = planRecovery(recorded, "2026-11-10T06:00:00Z");
    expect(actions.map((a) => a.type)).toEqual(["comment_issue"]);
    expect(actions).toEqual([
      expect.objectContaining({
        type: "comment_issue",
        issueNumber: 777,
        body: expect.stringContaining("issue remains open for authorized disposition"),
      }),
    ]);
    const { calls, effectors } = countingEffectors();
    await applyEscalationActions(actions, effectors);
    expect(calls).toEqual(["comment"]);
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
    const recorded = recordEscalation(escalatingHealth(), CTX, { queueTaskId: "task-4242", issueNumber: 777 }, T0);
    const t0 = Date.parse(T0);
    expect(surfacingLine(recorded, t0 + ESCALATION_NEEDS_DREW_AFTER_MS - 1)).toBeNull();
    const line = surfacingLine(recorded, t0 + ESCALATION_NEEDS_DREW_AFTER_MS);
    expect(line).toContain("NEEDS DREW");
    expect(line).toContain("777");
  });
});

describe("state coercion keeps the escalation record", () => {
  test("coerceHealth round-trips a valid escalation and drops a malformed one", () => {
    const recorded = recordEscalation(escalatingHealth(), CTX, { queueTaskId: "task-4242", issueNumber: 777 }, T0);
    const back = coerceHealth("mistral", JSON.parse(JSON.stringify(recorded)));
    expect(back.incident?.escalation?.queueTaskId).toBe("task-4242");
    expect(back.incident?.escalation?.issueNumber).toBe(777);

    const corrupt = JSON.parse(JSON.stringify(recorded));
    corrupt.incident.escalation = { class: "not-a-class", since: 5 };
    const dropped = coerceHealth("mistral", corrupt);
    expect(dropped.incident?.escalation).toBeNull();
  });
});

describe("Codex findings on PR #8357 (class change, partial progress, surfacing reason)", () => {
  test("a class change preserves the earlier hand-off; recovery records both without disposition; changing back dedupes", () => {
    const first = recordEscalation(escalatingHealth(), CTX, { queueTaskId: "task-a", issueNumber: 111 }, T0);
    const changed: SourceHealth = { ...first, currentClass: "endpoint-changed" };
    const second = recordEscalation(changed, CTX, { queueTaskId: "task-b", issueNumber: 222 }, T0);
    expect(second.incident?.escalation?.class).toBe("endpoint-changed");
    expect(second.incident?.escalation?.previous).toHaveLength(1);
    expect(second.incident?.escalation?.previous?.[0]).toMatchObject({
      class: "session-expired",
      queueTaskId: "task-a",
      issueNumber: 111,
    });

    const actions = planRecovery(second, "2026-11-10T06:00:00Z");
    expect(actions.map((a) => a.type)).toEqual(["comment_issue", "comment_issue"]);
    expect(actions.map((a) => a.type === "comment_issue" ? a.issueNumber : null)).toEqual([222, 111]);

    const backAgain: SourceHealth = { ...second, currentClass: "session-expired" };
    const plan = planEscalation(backAgain, CTX);
    expect(plan.deduped).toBe(true);
    expect(plan.actions).toEqual([]);
  });

  test("a failed enqueueTask surfaces the confirmed issue number; the retry plans only the missing task", async () => {
    const health = escalatingHealth();
    const plan = planEscalation(health, CTX);
    const failing: EscalationEffectors = {
      enqueueTask: async () => {
        throw new Error("queue unavailable");
      },
      openIssue: async () => 777,
      commentIssue: async () => {},
    };
    const err = await applyEscalationActions(plan.actions, failing).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(EscalationPartialError);
    expect((err as EscalationPartialError).refs).toEqual({
      queueTaskId: null,
      issueNumber: 777,
    });

    const recorded = recordEscalation(health, CTX, (err as EscalationPartialError).refs, T0);
    const retry = planEscalation(recorded, CTX);
    expect(retry.deduped).toBe(false);
    expect(retry.actions.map((a) => a.type)).toEqual(["enqueue_task"]);

    const completed = recordEscalation(recorded, CTX, { queueTaskId: "task-4242", issueNumber: null }, T0);
    expect(completed.incident?.escalation?.queueTaskId).toBe("task-4242");
    expect(completed.incident?.escalation?.issueNumber).toBe(777);
    expect(completed.incident?.escalation?.since).toBe(T0);
    expect(planEscalation(completed, CTX).deduped).toBe(true);
  });

  test("human-blocked surfacing names the persisted challenge reason, not the producer detail", () => {
    const hb = escalatingHealth();
    hb.lastDetail = "HTTP 401";
    hb.repairLog = [...hb.repairLog, { at: T0, rung: 4, outcome: "failed", detail: "passkey required on the laptop" }];
    hb.incident = { ...hb.incident!, humanBlocked: true, surfacing: "human-blocked" };
    const line = surfacingLine(hb, Date.parse(T0));
    expect(line).toContain("passkey required on the laptop");
    expect(line).not.toContain("HTTP 401");
  });

  test("coerceHealth round-trips the previous hand-offs", () => {
    const first = recordEscalation(escalatingHealth(), CTX, { queueTaskId: "task-a", issueNumber: 111 }, T0);
    const changed: SourceHealth = { ...first, currentClass: "endpoint-changed" };
    const second = recordEscalation(changed, CTX, { queueTaskId: "task-b", issueNumber: 222 }, T0);
    const back = coerceHealth("mistral", JSON.parse(JSON.stringify(second)));
    expect(back.incident?.escalation?.previous).toHaveLength(1);
    expect(back.incident?.escalation?.previous?.[0].queueTaskId).toBe("task-a");
  });
});
