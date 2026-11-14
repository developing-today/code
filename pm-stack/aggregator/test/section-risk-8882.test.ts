// lane-scratch: any-d /tmp/lane-any-d-20261112-0202 task 15095
import { describe, expect, test } from "bun:test";
import {
  buildNotificationCandidates,
  isNotificationTestClass,
  NOTIFICATION_CLASS_DEFAULTS,
  NOTIFICATION_CLASSES,
  notificationTestLanding,
  type NotificationSnapshot,
} from "../src/notifications.ts";
import { poolServesDoor, sectionPoolIdsFromConfig } from "../src/aa.ts";

// Thu 2026-11-12 08:00Z. Fri 2026-11-13 / Sat 2026-11-14 / Mon 2026-11-16 below.
const NOW = Date.parse("2026-11-12T08:00:00Z");
const FRI = "2026-11-13T14:28:00Z";
const SAT = "2026-11-14T09:00:00Z";
const MON = "2026-11-16T11:04:00Z";

function pool(id: string, state: string, windows: Array<{ percent: number; resetsAt: string | null }>) {
  return {
    id,
    provider: id,
    plan: "",
    state,
    windows: windows.map((w, i) => ({
      name: `w${i}`,
      unit: "tokens" as const,
      limit: 100,
      used: w.percent,
      percent: w.percent,
      resetsAt: w.resetsAt,
    })),
  };
}

function snapshot(pools: unknown[], sectionPools: Record<string, string[]>): NotificationSnapshot {
  return {
    attention: { drew: [], drewExtra: [] } as unknown as NotificationSnapshot["attention"],
    quota: { pools } as unknown as NotificationSnapshot["quota"],
    health: { services: [] },
    sectionPools,
  };
}

function sectionCandidates(pools: unknown[], sectionPools: Record<string, string[]>) {
  return buildNotificationCandidates(snapshot(pools, sectionPools), NOW).filter((c) => c.class === "section-risk");
}

describe("section-risk class registration (#8882)", () => {
  test("the class is a default-on preference class landing on /quota", () => {
    expect(NOTIFICATION_CLASS_DEFAULTS["section-risk"]).toBe(true);
    const meta = NOTIFICATION_CLASSES.find((c) => c.id === "section-risk");
    expect(meta?.label).toBe("Section failure / severe risk");
    expect(isNotificationTestClass("section-risk")).toBe(true);
    expect(notificationTestLanding("section-risk", "https://board.example")).toBe("https://board.example/quota");
  });
});

describe("section-risk derivation (#8882)", () => {
  test("every workhorse pool walled -> exactly one FAILED candidate, compact body, soonest reset first", () => {
    const candidates = sectionCandidates(
      [
        pool("pool-a", "healthy", [{ percent: 100, resetsAt: SAT }]),
        pool("pool-b", "walled", [{ percent: 100, resetsAt: FRI }]),
      ],
      { workhorse: ["pool-a", "pool-b"] },
    );
    expect(candidates).toHaveLength(1);
    expect(candidates[0].objectRef).toBe("section:workhorse");
    expect(candidates[0].title).toBe("Section failed: workhorse");
    expect(candidates[0].breakthrough).toBe(true);
    expect(candidates[0].url).toBe("/quota");
    expect(candidates[0].body).toBe("walled: pool-b (resets Fri 14:28Z), pool-a (resets Sat 09:00Z)");
  });

  test("only non-walled pool >=85% -> one SEVERE RISK candidate; stale pools never count as serving", () => {
    const candidates = sectionCandidates(
      [
        pool("pool-a", "healthy", [{ percent: 100, resetsAt: FRI }]),
        pool("pool-b", "healthy", [{ percent: 97, resetsAt: MON }]),
        pool("pool-c", "stale", [{ percent: 10, resetsAt: MON }]),
      ],
      { workhorse: ["pool-a", "pool-b", "pool-c"] },
    );
    expect(candidates).toHaveLength(1);
    expect(candidates[0].title).toBe("Section at risk: workhorse");
    expect(candidates[0].breakthrough).toBe(false);
    expect(candidates[0].body).toBe("walled: pool-a (resets Fri 14:28Z) · pending: pool-b 97% (resets Mon 11:04Z)");
  });

  test("one healthy pool under 85% -> NO section candidate even with other pools walled/near", () => {
    const candidates = sectionCandidates(
      [
        pool("pool-a", "healthy", [{ percent: 100, resetsAt: FRI }]),
        pool("pool-b", "healthy", [{ percent: 97, resetsAt: MON }]),
        pool("pool-c", "healthy", [{ percent: 40, resetsAt: MON }]),
      ],
      { workhorse: ["pool-a", "pool-b", "pool-c"] },
    );
    expect(candidates).toHaveLength(0);
  });

  test("absent membership -> no section candidates (never guessed)", () => {
    const candidates = sectionCandidates([pool("pool-a", "walled", [{ percent: 100, resetsAt: FRI }])], {});
    expect(candidates).toHaveLength(0);
  });
});

describe("section pool join from config leaf sets (#8882)", () => {
  const CONFIG = `{
    "provider": {},
    "experimental": { "model_routes": {
      "workhorse": { "targets": ["opencode-route/wk-plan", "_antigravity/claude-x"] },
      "wk-plan": { "targets": ["opencode-go-plans/deepseek-flash", "cline-pass/deepseek-flash"] },
      "speed": { "targets": ["_muse/muse-spark"] },
      "specialist": { "targets": ["mistral-plan/mistral-x"] },
      "summit": { "targets": ["openai-plan/gpt-luna"] },
      "small": { "targets": ["opencode-route/small-100k", "_claude/haiku"] },
      "small-100k": { "targets": ["_claude/haiku"] }
    } }
  }`;
  const POOLS = [
    "opencode-go.drewrypope",
    "opencode-go.popesdrew",
    "cline-pass",
    "_antigravity.gemini",
    "_antigravity.claude-gpt",
    "meta",
    "anthropic:claude-pro",
    "_openai.main",
    "_openai.alt",
    "mistral",
    "deepseek",
  ];

  test("doors join to pools: service-id spellings, split pools, and plan doors", () => {
    const membership = sectionPoolIdsFromConfig(CONFIG, POOLS);
    // The workhorse fixture's only Antigravity leaf is a Claude
    // model (_antigravity/claude-x), so per-model attribution
    // (#9170) serves only _antigravity.claude-gpt — NOT the Gemini
    // pool the old door-level join added.
    expect(membership.workhorse).toEqual([
      "_antigravity.claude-gpt",
      "cline-pass",
      "opencode-go.drewrypope",
      "opencode-go.popesdrew",
    ]);
    expect(membership.speed).toEqual(["meta"]);
    expect(membership.specialist).toEqual(["mistral"]);
    expect(membership.summit).toEqual(["_openai.alt", "_openai.main"]);
    // small reaches small-100k's leaves through its nested route target.
    expect(membership.small).toEqual(["anthropic:claude-pro"]);
  });

  test("poolServesDoor pins the three join rules", () => {
    expect(poolServesDoor("anthropic:claude-pro", "_claude")).toBe(true);
    expect(poolServesDoor("_antigravity.gemini", "_antigravity")).toBe(true);
    expect(poolServesDoor("opencode-go.popesdrew", "opencode-go-plans")).toBe(true);
    expect(poolServesDoor("deepseek", "opencode-go-plans")).toBe(false);
    expect(poolServesDoor("meta", "_claude")).toBe(false);
  });

  test("split Antigravity pools join by leaf model, not door (#9170)", () => {
    // Mirrors the live config shape: speed reaches exactly one
    // Antigravity leaf, a Gemini model, through a nested route.
    const splitConfig = `{
      "provider": {},
      "experimental": { "model_routes": {
        "speed": { "targets": ["opencode-route/gemini-hydrogen-plan"] },
        "gemini-hydrogen-plan": { "targets": ["_antigravity/gemini-4-hydrogen#high"] },
        "workhorse": { "targets": ["_antigravity/claude-sonnet-5-5"] },
        "summit": { "targets": ["_antigravity/gemini-4-argon", "_antigravity/claude-opus-5-5"] }
      } }
    }`;
    const membership = sectionPoolIdsFromConfig(splitConfig, POOLS);
    // Regression (task 15348): a Gemini-only section must NOT gain
    // _antigravity.claude-gpt, whose health would suppress its alert.
    expect(membership.speed).toEqual(["_antigravity.gemini"]);
    // Mirror image: a Claude-only section gains only claude-gpt.
    expect(membership.workhorse).toEqual(["_antigravity.claude-gpt"]);
    // Both leaf kinds present -> both pools serve the section.
    expect(membership.summit).toEqual(["_antigravity.claude-gpt", "_antigravity.gemini"]);
  });

  test("unparseable config yields no membership", () => {
    expect(sectionPoolIdsFromConfig("{not json", POOLS)).toEqual({});
  });
});
