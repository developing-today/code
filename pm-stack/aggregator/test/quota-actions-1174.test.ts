import { describe, expect, test } from "bun:test";

import { buildQuotaActions, quotaActionFlags, quotaActionTierItems } from "../src/derive.ts";
import { renderTodayBody } from "../src/render.ts";
import type { AttentionView, QuotaPool, QuotaView, TodayView } from "../src/model.ts";

const PLANS = {
  "opencode-go.personal": {
    cancelled: true,
    periodEndsAt: "2026-11-02",
    renewalAction: "Burn before Lab per rotation order",
  },
  "opencode-go.lab": {
    cancelled: true,
    periodEndsAt: "2026-11-06",
  },
};

function pool(over: Partial<QuotaPool> = {}): QuotaPool {
  return {
    id: "opencode-go:personal",
    provider: "OpenCode Go",
    plan: "Personal",
    state: "healthy",
    windows: [],
    ...over,
  };
}

function quota(pools: QuotaPool[]): QuotaView {
  return { pools, burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } };
}

const EMPTY_TODAY: TodayView = { objectives: [], momentum: [], fromOrchestrator: false };
const EMPTY_ATTENTION: AttentionView = {
  drew: [],
  orchestrator: [],
  watchdog: [],
  drewExtra: [],
  orchestratorExtra: [],
  all: [],
  expiredCount: 0,
  tierSource: { present: false, state: "absent" },
};

describe("buildQuotaActions (#1174)", () => {
  test("cancelled period inside 14 days yields an expiry action with the renewal step", () => {
    const actions = buildQuotaActions(quota([pool()]), PLANS, Date.parse("2026-10-26T12:00:00Z"));
    expect(actions).toHaveLength(1);
    const [a] = actions;
    expect(a.kind).toBe("expiry");
    expect(a.id).toBe("opencode-go:personal:expiry");
    expect(a.title).toContain("2026-11-02");
    expect(a.detail).toContain("Cancelled");
    expect(a.detail).toContain("Burn before Lab per rotation order");
    expect(a.deadline).not.toBeNull();
    expect(a.href).toBe("/quota");
  });

  test("expired period yields no entry", () => {
    const actions = buildQuotaActions(quota([pool()]), PLANS, Date.parse("2026-11-03T12:00:00Z"));
    expect(actions).toHaveLength(0);
  });

  test("period more than 14 days out yields no entry", () => {
    const actions = buildQuotaActions(quota([pool()]), PLANS, Date.parse("2026-10-01T12:00:00Z"));
    expect(actions).toHaveLength(0);
  });

  test("missing plan fact yields no expiry entry", () => {
    const actions = buildQuotaActions(quota([pool()]), {}, Date.parse("2026-10-26T12:00:00Z"));
    expect(actions).toHaveLength(0);
  });

  test("walled pool yields a walled action carrying the lift date", () => {
    const actions = buildQuotaActions(
      quota([pool({ state: "walled", wallLiftsAt: "2026-11-05T00:00:00Z" })]),
      {},
      Date.parse("2026-10-26T12:00:00Z"),
    );
    expect(actions).toHaveLength(1);
    expect(actions[0].kind).toBe("walled");
    expect(actions[0].deadline).toBe("2026-11-05T00:00:00Z");
    expect(actions[0].detail).toContain("2026-11-05");
  });

  test("healthy pool with no facts yields no entry (clearing)", () => {
    const actions = buildQuotaActions(
      quota([pool({ state: "healthy", windows: [] })]),
      PLANS,
      Date.parse("2026-11-10T12:00:00Z"),
    );
    expect(actions).toHaveLength(0);
  });

  test("high usage with a distant reset yields a high-usage action", () => {
    const now = Date.parse("2026-10-26T12:00:00Z");
    const actions = buildQuotaActions(
      quota([
        pool({
          id: "openai:codex",
          provider: "OpenAI",
          plan: "Codex",
          windows: [
            { name: "weekly", unit: "other", limit: 100, used: 92, resetsAt: "2026-10-30T12:00:00Z", percent: 92 },
          ],
        }),
      ]),
      {},
      now,
    );
    expect(actions).toHaveLength(1);
    expect(actions[0].kind).toBe("high-usage");
    expect(actions[0].detail).toContain("92%");
  });

  test("high usage with a near reset yields no entry", () => {
    const now = Date.parse("2026-10-26T12:00:00Z");
    const actions = buildQuotaActions(
      quota([
        pool({
          id: "openai:codex",
          provider: "OpenAI",
          plan: "Codex",
          windows: [
            { name: "weekly", unit: "other", limit: 100, used: 92, resetsAt: "2026-10-26T20:00:00Z", percent: 92 },
          ],
        }),
      ]),
      {},
      now,
    );
    expect(actions).toHaveLength(0);
  });

  test("a hotter window with a near reset does not mask a qualifying cooler window (Codex P2, PR #4695)", () => {
    const now = Date.parse("2026-10-26T12:00:00Z");
    const actions = buildQuotaActions(
      quota([
        pool({
          id: "openai:codex",
          provider: "OpenAI",
          plan: "Codex",
          windows: [
            { name: "5-hour", unit: "other", limit: 100, used: 99, resetsAt: "2026-10-26T14:00:00Z", percent: 99 },
            { name: "weekly", unit: "other", limit: 100, used: 92, resetsAt: "2026-10-30T12:00:00Z", percent: 92 },
          ],
        }),
      ]),
      {},
      now,
    );
    expect(actions).toHaveLength(1);
    expect(actions[0].kind).toBe("high-usage");
    expect(actions[0].detail).toContain("weekly");
    expect(actions[0].detail).toContain("92%");
  });

  test("a hotter window with no reset recorded does not mask a qualifying cooler window (Codex P2, PR #4695)", () => {
    const now = Date.parse("2026-10-26T12:00:00Z");
    const actions = buildQuotaActions(
      quota([
        pool({
          id: "openai:codex",
          provider: "OpenAI",
          plan: "Codex",
          windows: [
            { name: "5-hour", unit: "other", limit: 100, used: 99, percent: 99 },
            { name: "weekly", unit: "other", limit: 100, used: 92, resetsAt: "2026-10-30T12:00:00Z", percent: 92 },
          ],
        }),
      ]),
      {},
      now,
    );
    expect(actions).toHaveLength(1);
    expect(actions[0].kind).toBe("high-usage");
    expect(actions[0].detail).toContain("weekly");
  });

  test("85% exactly is not high usage; entries order expiry, walled, high-usage", () => {
    const now = Date.parse("2026-10-26T12:00:00Z");
    const actions = buildQuotaActions(
      quota([
        pool({
          id: "openai:codex",
          provider: "OpenAI",
          plan: "Codex",
          windows: [
            { name: "weekly", unit: "other", limit: 100, used: 85, resetsAt: "2026-10-30T12:00:00Z", percent: 85 },
          ],
        }),
        pool(),
        pool({ id: "meta:high-usage", provider: "Meta", plan: "High usage", state: "walled" }),
      ]),
      PLANS,
      now,
    );
    expect(actions.map((a) => a.kind)).toEqual(["expiry", "walled"]);
  });
});

describe("quota action surfaces (#1174)", () => {
  test("flags carry the strip contract", () => {
    const actions = buildQuotaActions(quota([pool()]), PLANS, Date.parse("2026-10-26T12:00:00Z"));
    const flags = quotaActionFlags(actions);
    expect(flags).toHaveLength(1);
    expect(flags[0].kind).toBe("quota-action");
    expect(flags[0].href).toBe("/quota");
    expect(flags[0].label).toBe(actions[0].title);
  });

  test("tier items are stable task items", () => {
    const actions = buildQuotaActions(quota([pool()]), PLANS, Date.parse("2026-10-26T12:00:00Z"));
    const items = quotaActionTierItems(actions, Date.parse("2026-10-26T12:00:00Z"));
    expect(items).toHaveLength(1);
    expect(items[0].id).toBe("quota-action:opencode-go:personal:expiry");
    expect(items[0].kind).toBe("task");
    expect(items[0].reason).toBe(actions[0].detail);
    expect(items[0].horizon).toBe("longer");
  });

  test("/today renders the action as alert and objective", () => {
    const html = renderTodayBody(
      EMPTY_TODAY,
      EMPTY_ATTENTION,
      quota([pool({ state: "walled", wallLiftsAt: "2026-11-05T00:00:00Z" })]),
      [],
    );
    expect(html).toContain("Quota action:");
    expect(html).toContain("is walled");
    expect(html).toContain("quota · action");
  });

  test("/today with no eligible actions renders no quota alerts", () => {
    const html = renderTodayBody(
      EMPTY_TODAY,
      EMPTY_ATTENTION,
      quota([pool({ id: "example:pool", provider: "Example", plan: "Demo", state: "healthy", windows: [] })]),
      [],
    );
    expect(html).not.toContain("Quota action:");
    expect(html).toContain("No objectives lined up for today.");
  });
});
