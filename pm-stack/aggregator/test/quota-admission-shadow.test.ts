import { describe, expect, test } from "bun:test";
import type { AdmissionInput, Binding } from "../src/quota-admission.ts";
import { evaluateAdmissionShadow, type Applicability } from "../src/quota-admission-shadow.ts";

const now = 10_000_000;
const reset = now + 60_000;
const route = { provider: "go", alias: "a", model: "model-a", effort: "low" };
const scope = { provider: route.provider, account: "billing-a", bucket: "plan" };
const required = [
  { bucket: "plan", kind: "weekly", period: "p1", resetsAt: reset },
  { bucket: "plan", kind: "monthly", period: "p1", resetsAt: reset },
];
const receipt = { ...route, account: scope.account, evidenceId: "billing-policy-a", verifiedUntil: reset, required };
const binding: Binding = {
  ...scope,
  aliases: [route.alias],
  windows: required.map(({ kind }) => ({
    kind,
    period: "p1",
    resetsAt: reset,
    freshnessMs: 3_600_000,
    collectionMs: 900_000,
  })),
};
function fixture(weekly = 100): AdmissionInput {
  return {
    generation: "g1",
    bindings: [binding],
    observations: required.map(({ kind }) => ({
      ...scope,
      kind,
      period: "p1",
      id: kind,
      source: "provider",
      measuredAt: now,
      resetsAt: reset,
      percent: kind === "weekly" ? weekly : 30,
      state: "ok",
      denominator: "allowance",
    })),
    stops: [],
    priorWalls: [],
  };
}
const evaluate = (input: AdmissionInput, rows: Applicability[] = [receipt]) =>
  evaluateAdmissionShadow(input, "launch", [route], rows, now);

describe("admission shadow: #10323 all applicable windows and exact billing scope", () => {
  test("weekly wall defeats monthly headroom only for a route that uses it", () => {
    const other = { ...route, model: "model-b" };
    const input = fixture();
    const before = JSON.stringify(input);
    const result = evaluateAdmissionShadow(
      input,
      "launch",
      [route, other],
      [receipt, { ...receipt, ...other, required: [required[1]] }],
      now,
    );
    expect(result.decisions.map((d) => d.verdict)).toEqual(["walled", "eligible"]);
    expect(result.wouldDefer).toBe(false);
    expect(result.generation).toBe("g1");
    expect(JSON.stringify(input)).toBe(before);
  });

  test("missing, duplicate, empty, and effort-mismatched evidence cannot grant eligibility", () => {
    for (const rows of [
      [],
      [receipt, receipt],
      [{ ...receipt, required: [] }],
      [{ ...receipt, effort: "high" }],
      [{ ...receipt, verifiedUntil: now }],
      [{ ...receipt, required: [required[1], required[1]] }],
    ]) {
      expect(evaluate(fixture(0), rows).decisions[0].verdict).toBe("unknown");
      expect(evaluate(fixture(0), rows).wouldDefer).toBe(true);
    }
  });

  test("no implicit sibling account, missing bucket, or missing window substitution", () => {
    for (const patch of [
      { account: "billing-b" },
      { required: [{ ...required[0], bucket: "other" }] },
      { required: [{ ...required[0], kind: "hourly" }] },
      { required: [{ ...required[0], period: "p0" }] },
      { required: [{ ...required[0], resetsAt: reset + 1 }] },
    ]) {
      expect(evaluate(fixture(0), [{ ...receipt, ...patch }]).decisions[0].verdict).toBe("unknown");
    }
    const input = fixture(0);
    input.bindings = [binding, { ...binding, account: "billing-b" }];
    expect(evaluate(input).decisions[0].verdict).toBe("unknown");
  });

  test("stale/missing data cannot clear a prior applicable wall or active scope stop", () => {
    const input = fixture(0);
    input.observations = [];
    input.priorWalls = [{ ...scope, kind: "weekly", period: "p1", resetsAt: reset, evidenceId: "wall" }];
    expect(evaluate(input).decisions[0].windows[0]).toMatchObject({
      bucket: "plan",
      kind: "weekly",
      period: "p1",
      verdict: "walled",
      evidenceIds: ["wall"],
      resetsAt: reset,
    });
    input.stops = [{ ...scope, id: "stop", reason: "steward_hold", effectiveAt: now - 1 }];
    expect(evaluate(input).decisions[0].verdict).toBe("stopped");
    expect(evaluate(input).decisions[0].windows[0].evidenceIds).toEqual(["stop"]);
    input.stops = [{ ...input.stops[0], releasedAt: now }];
    expect(evaluate(input).decisions[0].verdict).toBe("walled");
  });

  test("actual resolver account is compared without rewriting launch or resume decisions", () => {
    const input = fixture(0);
    const run = (actual?: typeof route & { account: string }) =>
      evaluateAdmissionShadow(input, "resume", [route], [receipt], now, actual);
    expect(run().phase).toBe("resume");
    expect(run().comparison).toBe("not_observed");
    expect(run({ ...route, account: scope.account }).comparison).toBe("eligible");
    expect(run({ ...route, account: "billing-b" }).comparison).toBe("account_mismatch");
    expect(run({ ...route, model: "other", account: scope.account }).comparison).toBe("not_candidate");
    expect(run({ ...route, account: "billing-b" }).decisions).toEqual(run().decisions);
    expect(evaluateAdmissionShadow(input, "launch", [], [], now).wouldDefer).toBe(true);
  });
});
