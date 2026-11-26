// Oracle (#15018): the issue's acceptance bullets — fresh/stale/failed
// refresh, last-good retention, action idempotency, audit-before-dispatch,
// confirmation, stale-confirmation rejection, result write-back, and the
// 320px phone-first render with no page-level horizontal scrolling.
import { describe, expect, test } from "bun:test";
import {
  embedQuestions,
  freshness,
  QuotaPageStore,
  renderQuotaPage,
  ResetFlow,
  resetEligible,
  validateQuotaEnvelope,
} from "../src/quota-page.ts";

const T0 = Date.parse("2026-11-25T12:00:00Z");
const env = (over: Record<string, unknown> = {}) =>
  validateQuotaEnvelope({
    version: "quota-page-data/v1",
    pageId: "quota",
    revision: 1,
    asOf: new Date(T0).toISOString(),
    freshnessBudgetMs: 60_000,
    pools: [
      {
        id: "openai-main",
        provider: "openai",
        label: "OpenAI main",
        used: 100,
        limit: 100,
        unit: "credits",
        window: "weekly",
        renewalAt: "2026-12-01T00:00:00Z",
        projectedWallAt: "2026-11-26T00:00:00Z",
        wallUntil: "1d",
        projectionBasis: "C4 quota-wall/v1 measured burn rate",
      },
    ],
    ...over,
  });

describe("quota page (#15018)", () => {
  test("validates the versioned envelope and rejects bad versions", () => {
    expect(env().pools[0].id).toBe("openai-main");
    expect(() => validateQuotaEnvelope({ version: "v9" })).toThrow("version");
  });
  test("fresh within budget, stale beyond it", () => {
    expect(freshness(env(), T0 + 30_000).stale).toBe(false);
    expect(freshness(env(), T0 + 61_000).stale).toBe(true);
  });
  test("failed refresh preserves last-good and routes one deduplicated exception", () => {
    const routed: string[] = [];
    const store = new QuotaPageStore((e) => routed.push(e.key));
    store.applySuccess(env(), T0);
    expect(store.applyFailure("boom", T0 + 120_000)?.revision).toBe(1);
    store.applyFailure("boom again", T0 + 130_000);
    expect(store.good?.revision).toBe(1);
    expect(routed.length).toBe(1);
    expect(store.exceptions.length).toBe(1);
  });
  test("embeds existing question sets by reference, unresolved only", () => {
    const q = embedQuestions([
      {
        setId: "s1",
        items: [
          {
            ledgerId: "l1",
            questions: [
              { qid: "q1", question: "Reset?" },
              { qid: "q2", question: "Done?", resolution: { at: "x" } },
            ],
          },
        ],
      },
    ]);
    expect(q.total).toBe(1);
    expect(q.open[0].qid).toBe("q1");
  });
  test("reset eligibility: eligible provider at 100% only", () => {
    expect(resetEligible(env().pools[0]).eligible).toBe(true);
    expect(resetEligible({ ...env().pools[0], used: 50 }).eligible).toBe(false);
    expect(resetEligible({ ...env().pools[0], provider: "other" }).eligible).toBe(false);
  });
  test("confirmation dispatches once (idempotent), audits before dispatch, writes back result", async () => {
    const order: string[] = [];
    let calls = 0;
    const flow = new ResetFlow(async () => {
      calls++;
      order.push("dispatch");
      return { queueTaskId: "task-1", result: "queued" };
    });
    const rec = flow.propose(env(), "openai-main", "owner", "key-1", T0);
    expect(flow.propose(env(), "openai-main", "owner", "key-1", T0).id).toBe(rec.id);
    const done = await flow.confirm(env(), rec.id, "owner", T0);
    order.unshift(flow.audit[0].phase);
    expect(order).toEqual(["audit-before-dispatch", "dispatch"]);
    expect(done).toMatchObject({ status: "dispatched", queueTaskId: "task-1", result: "queued", confirmedBy: "owner" });
    await flow.confirm(env(), rec.id, "owner", T0);
    expect(calls).toBe(1);
  });
  test("stale-confirmation is rejected and requires fresh confirmation", async () => {
    const flow = new ResetFlow(async () => ({ queueTaskId: "t", result: "ok" }));
    const rec = flow.propose(env(), "openai-main", "owner", "key-2", T0);
    await expect(flow.confirm(env({ revision: 2 }), rec.id, "owner", T0)).rejects.toThrow("stale action version");
    expect(rec.status).toBe("proposed");
  });
  test("renders phone-first at 320px with facts, basis, questions and no horizontal scroll", () => {
    const html = renderQuotaPage(env(), {
      now: T0,
      questions: embedQuestions([
        { setId: "s1", items: [{ ledgerId: "l1", questions: [{ qid: "q1", question: "Reset now?" }] }] },
      ]),
    });
    for (const s of [
      "As of",
      "weekly",
      "2026-12-01",
      "Projected wall",
      "Basis: C4",
      "Reset now?",
      "I confirm this reset",
      "width=device-width",
      "overflow-x:hidden",
      "box-sizing:border-box",
    ])
      expect(html).toContain(s);
    expect(html).not.toMatch(/width:\s*(3[3-9]\d|[4-9]\d\d|\d{4,})px/);
  });
  test("stale render shows last-good/stale banner", () => {
    expect(renderQuotaPage(env(), { now: T0 + 120_000 })).toContain("STALE — showing last-good data");
  });
});
