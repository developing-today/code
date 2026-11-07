import { describe, expect, test } from "bun:test";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadConfig } from "../src/config.ts";
import { validateStreamEvent } from "../src/events.ts";
import { buildAttention, todayFromSection } from "../src/derive.ts";
import {
  ageMsText,
  diagnoseOrchestrationSection,
  feedAgeText,
  isFleetFeed,
  isOrchestrationSection,
  normalizeOrchestrationSection,
  orchestrationSectionProblem,
} from "../src/feed.ts";
import type { LinedUpTask, OrchestrationSection, QuotaView } from "../src/model.ts";
import { renderTodayBody } from "../src/render.ts";
import { liveSource } from "../src/sources/live.ts";

// #899: the orchestration section rides the fleet feed. The collector
// embeds the orchestrator's output file only when it passes the
// structural check; the aggregator serves it at /api/orchestration.json.

const SECTION: OrchestrationSection = {
  version: 1,
  generatedAt: "2026-10-25T17:20:00.000Z",
  runId: "orch-2026-10-25T17:20:00.000Z",
  provenance: {
    sources: ["fleet-feed.json, keeper state, repo backlogs (orchestration-pass run)"],
    runId: "orch-2026-10-25T17:20:00.000Z",
  },
  tiers: {
    needsDrew: [
      {
        id: "developing-today/code#812",
        kind: "decision",
        title: "Decide the public directory feed design",
        source: "backlog",
        reason: "Scope requires an owner design decision only Drew can make.",
        since: "2026-10-25T17:20:00.000Z",
        lastSeenAt: "2026-10-25T17:20:00.000Z",
        ref: "developing-today/code#812",
      },
    ],
    needsOrchestrator: [],
  },
  dependencies: [],
  goals: [
    {
      goalId: null,
      goal: "Keep the four repos healthy",
      momentum: "moving",
      summary: "Sweeps merged 3 PRs since the last run.",
      linedUp: [],
      progressedSinceLastRun: ["PR #888 merged"],
      stalled: [],
      gaps: [],
    },
  ],
  actions: [],
};

describe("orchestration section validation", () => {
  test("accepts a well-formed section", () => {
    expect(isOrchestrationSection(SECTION)).toBe(true);
  });

  test("rejects non-objects and wrong versions", () => {
    expect(isOrchestrationSection(null)).toBe(false);
    expect(isOrchestrationSection("orchestration")).toBe(false);
    expect(isOrchestrationSection({ ...SECTION, version: 2 })).toBe(false);
  });

  test("rejects missing tier arrays or goal list", () => {
    const { tiers: _t, ...noTiers } = SECTION;
    expect(isOrchestrationSection(noTiers)).toBe(false);
    expect(isOrchestrationSection({ ...SECTION, tiers: { needsDrew: [] } })).toBe(false);
    expect(isOrchestrationSection({ ...SECTION, goals: undefined })).toBe(false);
  });

  // #949: the validator is the only gate before todayFromSection
  // dereferences per-goal fields; a goal missing its nested arrays used
  // to pass and crash /today with a TypeError instead of falling back.
  test("rejects a goal missing each nested array", () => {
    const goal = SECTION.goals[0]!;
    for (const field of ["linedUp", "stalled", "gaps"] as const) {
      const malformed = {
        ...SECTION,
        goals: [{ ...goal, [field]: undefined }],
      };
      expect(isOrchestrationSection(malformed)).toBe(false);
    }
  });

  test("rejects a section missing provenance", () => {
    const { provenance: _p, ...noProvenance } = SECTION;
    expect(isOrchestrationSection(noProvenance)).toBe(false);
  });

  test("accepts the single-string form of progressedSinceLastRun", () => {
    const stringForm = {
      ...SECTION,
      goals: [{ ...SECTION.goals[0]!, progressedSinceLastRun: "PR #888 merged" }],
    };
    expect(isOrchestrationSection(stringForm)).toBe(true);
  });

  test("todayFromSection does not throw on a validator-passing minimal section", () => {
    const minimal: OrchestrationSection = {
      ...SECTION,
      goals: [
        {
          goalId: null,
          goal: "Minimal goal",
          momentum: "moving",
          summary: "",
          linedUp: [],
          progressedSinceLastRun: ["one line of progress"],
          stalled: [],
          gaps: [],
        },
      ],
    };
    expect(isOrchestrationSection(minimal)).toBe(true);
    expect(() => todayFromSection(minimal)).not.toThrow();
  });

  // #2417: the gate checked array presence only, so poisoned elements
  // (null / primitives / shapeless objects) passed into consumers and
  // threw TypeErrors. Every array's elements are now validated at this
  // boundary; a poisoned section must reject, normalize to null, and
  // diagnose as invalid with a stable failed-check name.
  test("rejects poisoned tier elements in both tier arrays", () => {
    for (const bad of [null, 42, {}]) {
      const drew = { ...SECTION, tiers: { ...SECTION.tiers, needsDrew: [bad] } };
      expect(isOrchestrationSection(drew)).toBe(false);
      expect(orchestrationSectionProblem(drew)).toBe("tiers.needsDrew (per-item shape)");
      expect(normalizeOrchestrationSection(drew)).toBeNull();
      const orch = { ...SECTION, tiers: { ...SECTION.tiers, needsOrchestrator: [bad] } };
      expect(isOrchestrationSection(orch)).toBe(false);
      expect(orchestrationSectionProblem(orch)).toBe("tiers.needsOrchestrator (per-item shape)");
    }
  });

  test("rejects non-record elements in goal linedUp and stalled arrays", () => {
    const goal = SECTION.goals[0]!;
    for (const field of ["linedUp", "stalled"] as const) {
      const malformed = { ...SECTION, goals: [{ ...goal, [field]: [null] }] };
      expect(isOrchestrationSection(malformed)).toBe(false);
      expect(orchestrationSectionProblem(malformed)).toBe("goals (per-goal shape)");
      expect(normalizeOrchestrationSection(malformed)).toBeNull();
    }
    const nullGoal = { ...SECTION, goals: [null] };
    expect(isOrchestrationSection(nullGoal)).toBe(false);
  });

  // #3897: the per-goal gate checked array presence (and, after #2417,
  // record-ness) only — goal/momentum/summary/goalId and the element
  // fields todayFromSection dereferences were never validated, so
  // drifted goals rendered falsified state or threw downstream.
  test("rejects goals with malformed scalar fields and elements", () => {
    const goal = SECTION.goals[0]!;
    const badGoals: unknown[] = [
      { ...goal, goal: undefined },
      { ...goal, goal: "" },
      { ...goal, momentum: "flying" },
      { ...goal, momentum: undefined },
      { ...goal, summary: undefined },
      { ...goal, goalId: 42 },
      { ...goal, linedUp: [{ title: "Task A" }] },
      { ...goal, linedUp: [{ title: "Task A", horizon: "someday" }] },
      { ...goal, linedUp: [{ title: "Task A", horizon: "today", status: "bogus" }] },
      { ...goal, linedUp: ["Task A"] },
      { ...goal, stalled: [{ title: "Stuck" }] },
      { ...goal, stalled: [{ title: "Stuck", since: "2026-10-25", blocker: 42 }] },
      { ...goal, gaps: [{ finding: "x" }] },
      { ...goal, progressedSinceLastRun: ["ok", 42] },
    ];
    for (const bad of badGoals) {
      const malformed = { ...SECTION, goals: [bad] };
      expect(isOrchestrationSection(malformed)).toBe(false);
      expect(orchestrationSectionProblem(malformed)).toBe("goals (per-goal shape)");
      expect(normalizeOrchestrationSection(malformed)).toBeNull();
    }
  });

  test("accepts a goal with fully shaped elements", () => {
    const rich = {
      ...SECTION,
      goals: [
        {
          ...SECTION.goals[0]!,
          linedUp: [{ title: "Task A", status: "in-progress", horizon: "today", horizonWhy: "due" }],
          stalled: [{ title: "Stuck", since: "2026-10-25", blocker: "waiting" }],
          gaps: ["goal has no tasks lined up"],
        },
      ],
    };
    expect(isOrchestrationSection(rich)).toBe(true);
    expect(() => todayFromSection(rich as OrchestrationSection)).not.toThrow();
  });

  test("rejects poisoned dependency elements and bad dependency shapes", () => {
    const nullDep = { ...SECTION, dependencies: [null] };
    expect(isOrchestrationSection(nullDep)).toBe(false);
    expect(orchestrationSectionProblem(nullDep)).toBe("dependencies (per-dependency shape)");
    const noStatus = { ...SECTION, dependencies: [{ task: "Write the thing" }] };
    expect(isOrchestrationSection(noStatus)).toBe(false);
    const badStatus = { ...SECTION, dependencies: [{ task: "Write the thing", status: "bogus" }] };
    expect(isOrchestrationSection(badStatus)).toBe(false);
    const badRef = {
      ...SECTION,
      dependencies: [{ task: "Write the thing", status: "queued", ref: 42 }],
    };
    expect(isOrchestrationSection(badRef)).toBe(false);
    const good = {
      ...SECTION,
      dependencies: [{ task: "Write the thing", status: "queued", ref: "developing-today/code#1" }],
    };
    expect(isOrchestrationSection(good)).toBe(true);
  });

  test("rejects poisoned action elements and actions missing rendered fields", () => {
    const nullAction = { ...SECTION, actions: [null] };
    expect(isOrchestrationSection(nullAction)).toBe(false);
    expect(orchestrationSectionProblem(nullAction)).toBe("actions (per-action shape)");
    const noSummary = {
      ...SECTION,
      actions: [{ at: "2026-10-25T17:20:00.000Z", kind: "nudge", target: "task 1" }],
    };
    expect(isOrchestrationSection(noSummary)).toBe(false);
    const good = {
      ...SECTION,
      actions: [{ at: "2026-10-25T17:20:00.000Z", kind: "nudge", target: "task 1", summary: "Nudged it." }],
    };
    expect(isOrchestrationSection(good)).toBe(true);
  });

  test("a poisoned section diagnoses as invalid, never absent or live", () => {
    const poisoned = { ...SECTION, tiers: { ...SECTION.tiers, needsDrew: [null] } };
    const diagnosis = diagnoseOrchestrationSection(poisoned);
    expect(diagnosis.state).toBe("invalid");
    expect(diagnosis.failedChecks).toEqual(["tiers.needsDrew (per-item shape)"]);
  });

  test("a fleet feed carrying the section still validates as a feed", () => {
    const feed = {
      version: 1,
      generatedAt: "2026-10-25T17:25:00.000Z",
      quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
      attention: [],
      orchestration: SECTION,
    };
    expect(isFleetFeed(feed)).toBe(true);
  });
});

// #1110 regression: the publisher's actual shape. This fixture mirrors
// the orchestration.json the orchestration-pass cron really published
// (run orch-2026-10-27T03:14:56Z, trimmed to one entry per list) —
// most importantly `provenance` as an OBJECT { sources, runId }, the
// form the string-only validator rejected fleet-wide for a day. If the
// publisher's shape drifts again, this test must fail loudly.
const LIVE_SHAPE = {
  version: 1,
  generatedAt: "2026-10-27T03:14:56Z",
  runId: "orch-2026-10-27T03:14:56Z",
  provenance: {
    sources: [
      "fleet-feed.json 2026-10-27T03:08:15Z (attention: 1 Jules IN_PROGRESS session for code #724, no awaitingQuestion; quota pools healthy)",
      "laptop attention via laptop-ssh SUCCEEDED this run: 6 entries, all needs-poke empty sessions",
      "jules-review-state.json keeper last_run 2026-10-26T23:03:16Z",
    ],
    runId: "orch-2026-10-27T03:14:56Z",
  },
  tiers: {
    needsDrew: [
      {
        id: "developing-today/code#812",
        kind: "decision",
        title: "Decide the public directory feed design",
        source: "backlog",
        reason: "Scope requires an owner design decision only Drew can make.",
        since: "2026-10-27T03:14:56Z",
        lastSeenAt: "2026-10-27T03:14:56Z",
        ref: "developing-today/code#812",
      },
    ],
    needsOrchestrator: [],
  },
  dependencies: [
    {
      task: "code #807 wire exposure (F12.1 apply wiring)",
      ref: "developing-today/code#807",
      status: "queued",
      taskRef: "developing-today/code#818",
    },
  ],
  goals: [
    {
      goalId: "goal_a243772844c3",
      goal: "Keep the four repos healthy",
      momentum: "moving",
      summary: "Strong movement: code #996, #997 and #688 closed via merged PRs.",
      progressedSinceLastRun: ["code #996 closed via merged PR"],
      linedUp: [],
      stalled: [],
      gaps: [],
    },
  ],
  actions: [
    {
      at: "2026-10-27T03:14:56Z",
      kind: "verify",
      target: "fleet queues",
      summary: "Reviewed fleet feed 03:08Z and laptop attention.",
    },
  ],
};

describe("orchestration provenance contract (#1110)", () => {
  test("accepts the section exactly as the orchestrator publishes it", () => {
    expect(isOrchestrationSection(LIVE_SHAPE)).toBe(true);
    expect(orchestrationSectionProblem(LIVE_SHAPE)).toBeNull();
    const normalized = normalizeOrchestrationSection(LIVE_SHAPE);
    expect(normalized?.runId).toBe("orch-2026-10-27T03:14:56Z");
    expect(normalized?.provenance).toEqual(LIVE_SHAPE.provenance);
    expect(normalized?.tiers.needsDrew.length).toBe(1);
    expect(() => todayFromSection(normalized!)).not.toThrow();
  });

  test("normalizes the legacy plain-string provenance to the object form", () => {
    const legacy = { ...LIVE_SHAPE, provenance: "orchestration-pass run orch-2026-10-27T03:14:56Z" };
    expect(isOrchestrationSection(legacy)).toBe(true);
    const normalized = normalizeOrchestrationSection(legacy);
    expect(normalized?.provenance).toEqual({
      sources: ["orchestration-pass run orch-2026-10-27T03:14:56Z"],
    });
  });

  test("rejects malformed provenance forms, naming the field", () => {
    for (const provenance of [42, {}, { sources: "fleet feed" }, { sources: ["ok", 7] }, { sources: [], runId: 5 }]) {
      const bad = { ...LIVE_SHAPE, provenance };
      expect(isOrchestrationSection(bad)).toBe(false);
      expect(orchestrationSectionProblem(bad)).toBe("provenance");
      expect(normalizeOrchestrationSection(bad)).toBeNull();
    }
  });

  test("the problem reporter names the first failing check", () => {
    expect(orchestrationSectionProblem({ ...SECTION, version: 2 })).toBe("version");
    expect(orchestrationSectionProblem({ ...SECTION, tiers: { needsDrew: [] } })).toBe("tiers.needsOrchestrator");
    expect(orchestrationSectionProblem(null)).toBe("not an object");
  });
});

describe("live source orchestration states (#1140)", () => {
  async function sourceWithFeed(extra: Record<string, unknown>) {
    const dir = await mkdtemp(join(tmpdir(), "pm-orch-"));
    const feedFile = join(dir, "fleet-feed.json");
    await writeFile(
      feedFile,
      JSON.stringify({
        version: 1,
        generatedAt: new Date().toISOString(),
        quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
        attention: [],
        ...extra,
      }),
    );
    const cfg = loadConfig({ PM_AGG_SOURCE: "live", PM_AGG_FEED_FILE: feedFile, PM_PROJECTOR_URL: "off" });
    return liveSource(cfg);
  }

  test("absent: no section in the feed reads as not published", async () => {
    const src = await sourceWithFeed({});
    expect(await src.orchestration()).toBeNull();
    expect(src.provenance().orchestration).toContain("has not published");
  });

  test("collector errors are distinct from sections not yet published", async () => {
    const src = await sourceWithFeed({
      meta: {
        sections: {
          orchestration: { status: "error", at: new Date().toISOString(), error: "invalid JSON" },
          dns: { status: "error", at: new Date().toISOString(), error: "register unreadable" },
        },
      },
    });
    expect(await src.orchestration()).toBeNull();
    expect(src.provenance().orchestration).toContain("collector error");
    expect(src.provenanceDetail?.().orchestration.state).toBe("error");
    expect(await src.dns()).toBeNull();
    expect(src.provenance().dns).toContain("collector error");
    expect(src.provenanceDetail?.().dns.state).toBe("error");
  });

  test("invalid: a present-but-malformed section is named, not 'absent'", async () => {
    const src = await sourceWithFeed({ orchestration: { ...LIVE_SHAPE, provenance: 42 } });
    expect(await src.orchestration()).toBeNull();
    expect(src.provenance().orchestration).toContain("invalid");
    expect(src.provenance().orchestration).toContain("provenance");
    expect(src.provenance().orchestration).not.toContain("has not published");
  });

  test("live: the published shape is served with its run id", async () => {
    const src = await sourceWithFeed({ orchestration: LIVE_SHAPE });
    const section = await src.orchestration();
    expect(section?.runId).toBe("orch-2026-10-27T03:14:56Z");
    expect(section?.tiers.needsDrew.length).toBe(1);
    expect(section?.provenance.sources.length).toBe(3);
    expect(src.provenance().orchestration).toContain("live (orchestration section orch-2026-10-27T03:14:56Z");
  });

  test("departed orchestration items emit a tier-change event", async () => {
    const dir = await mkdtemp(join(tmpdir(), "pm-orch-events-"));
    const feedFile = join(dir, "fleet-feed.json");
    const cfg = loadConfig({
      PM_AGG_SOURCE: "live",
      PM_AGG_FEED_FILE: feedFile,
      PM_AGG_REGISTRY_FILE: join(dir, "registry.json"),
      PM_PROJECTOR_URL: "off",
    });
    const events: Array<{ kind: string; ref: string; summary: string }> = [];
    const src = liveSource(cfg, { onEvent: (event) => events.push(event) });
    const writeSection = async (section: any) =>
      writeFile(
        feedFile,
        JSON.stringify({
          version: 1,
          generatedAt: new Date().toISOString(),
          quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
          attention: [],
          orchestration: section,
        }),
      );
    await writeSection(LIVE_SHAPE);
    await src.orchestration();
    const next = {
      ...LIVE_SHAPE,
      runId: "orch-next",
      generatedAt: new Date().toISOString(),
      provenance: { ...LIVE_SHAPE.provenance, runId: "orch-next" },
      tiers: { needsDrew: [], needsOrchestrator: [] },
    };
    await writeSection(next);
    await src.orchestration();
    expect(
      events.find((event) => event.kind === "tier-change" && event.ref === "developing-today/code#812")?.summary,
    ).toContain("needs-user → —");
  });

  test("an orchestration action with a free-text target is preserved as a valid event (#4489 Codex P2)", async () => {
    const dir = await mkdtemp(join(tmpdir(), "pm-orch-action-ref-"));
    const feedFile = join(dir, "fleet-feed.json");
    const cfg = loadConfig({
      PM_AGG_SOURCE: "live",
      PM_AGG_FEED_FILE: feedFile,
      PM_AGG_REGISTRY_FILE: join(dir, "registry.json"),
      PM_PROJECTOR_URL: "off",
    });
    const events: Array<{ at?: string; actor: string; kind: string; ref: string; summary: string }> = [];
    const src = liveSource(cfg, { onEvent: (event) => events.push(event) });
    await writeFile(
      feedFile,
      JSON.stringify({
        version: 1,
        generatedAt: new Date().toISOString(),
        quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
        attention: [],
        orchestration: LIVE_SHAPE,
      }),
    );
    await src.orchestration();
    // LIVE_SHAPE's published action targets "fleet queues" (free text
    // with a space). Its event must survive validation with a
    // whitespace-free ref instead of being dropped by emitEvent.
    const action = events.find((event) => event.kind === "verify");
    expect(action).toBeDefined();
    expect(action?.ref).toBe("fleet-queues");
    expect(validateStreamEvent({ at: new Date().toISOString(), ...action }).ok).toBe(true);
  });

  test("a durable publication identity prevents replay after source restart", async () => {
    const dir = await mkdtemp(join(tmpdir(), "pm-orch-restart-"));
    const feedFile = join(dir, "fleet-feed.json");
    const cfg = loadConfig({
      PM_AGG_SOURCE: "live",
      PM_AGG_FEED_FILE: feedFile,
      PM_AGG_REGISTRY_FILE: join(dir, "registry.json"),
      PM_PROJECTOR_URL: "off",
    });
    await writeFile(
      feedFile,
      JSON.stringify({
        version: 1,
        generatedAt: new Date().toISOString(),
        quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
        attention: [],
        orchestration: LIVE_SHAPE,
      }),
    );
    const persisted = new Set<string>();
    const events: Array<{ kind: string; ref: string }> = [];
    const opts = {
      onEvent: (event: { kind: string; ref: string }) => {
        events.push(event);
        if (event.kind === "orchestration-published") persisted.add(`${event.kind}:${event.ref}`);
      },
      hasEvent: async (kind: string, ref: string) => persisted.has(`${kind}:${ref}`),
    };
    await liveSource(cfg, opts).orchestration();
    const firstRunCount = events.length;
    await liveSource(cfg, opts).orchestration();
    expect(events).toHaveLength(firstRunCount);
  });

  test("registry merge emits events for every newly merged session", async () => {
    const dir = await mkdtemp(join(tmpdir(), "pm-registry-events-"));
    const feedFile = join(dir, "fleet-feed.json");
    const registry = Array.from({ length: 60 }, (_, i) => ({
      id: `session-${i}`,
      source: "opencode-sandbox",
      label: `Session ${i}`,
      agentId: "opencode:test",
      agentLabel: "test",
      parentId: null,
      status: "complete",
      taskRefs: [],
    }));
    await writeFile(
      feedFile,
      JSON.stringify({
        version: 1,
        generatedAt: new Date().toISOString(),
        quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
        attention: [],
        registry,
      }),
    );
    const cfg = loadConfig({
      PM_AGG_SOURCE: "live",
      PM_AGG_FEED_FILE: feedFile,
      PM_AGG_REGISTRY_FILE: join(dir, "registry.json"),
      PM_PROJECTOR_URL: "off",
    });
    const events: unknown[] = [];
    await liveSource(cfg, { onEvent: (event) => events.push(event) }).registry();
    expect(events).toHaveLength(60);
  });
});

describe("orchestration diagnosis (#1139)", () => {
  test("absent: null and missing values diagnose as absent", () => {
    expect(diagnoseOrchestrationSection(null).state).toBe("absent");
    expect(diagnoseOrchestrationSection(undefined).state).toBe("absent");
  });

  test("invalid: a present-but-malformed section names the failing check", () => {
    const bad = { ...SECTION, tiers: { needsOrchestrator: [] } };
    const d = diagnoseOrchestrationSection(bad, Date.parse("2026-10-25T18:00:00.000Z"));
    expect(d.state).toBe("invalid");
    expect(d.failedChecks).toContain("tiers.needsDrew");
  });

  test("live: a fresh valid section diagnoses live with run id and age", () => {
    const d = diagnoseOrchestrationSection(SECTION, Date.parse("2026-10-25T17:50:00.000Z"));
    expect(d.state).toBe("live");
    expect(d.runId).toBe("orch-2026-10-25T17:20:00.000Z");
    expect(d.ageMs).toBe(30 * 60_000);
  });

  test("stale: a valid section older than the 90m threshold diagnoses stale", () => {
    const d = diagnoseOrchestrationSection(SECTION, Date.parse("2026-10-25T19:00:00.000Z"));
    expect(d.state).toBe("stale");
    expect(d.ageMs).toBe(100 * 60_000);
  });

  test("invalid: an unparseable generatedAt fails the generatedAt check (#1564)", () => {
    const bad = { ...SECTION, generatedAt: "not a date" };
    expect(orchestrationSectionProblem(bad)).toBe("generatedAt");
    const d = diagnoseOrchestrationSection(bad, Date.parse("2026-10-25T17:50:00.000Z"));
    expect(d.state).toBe("invalid");
    expect(d.failedChecks).toEqual(["generatedAt"]);
    const feed = {
      version: 1,
      generatedAt: "not a date",
      quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
      attention: [],
      orchestration: SECTION,
    };
    expect(isFleetFeed(feed)).toBe(false);
  });

  test("invalid: a generatedAt more than 60s in the future is never live (#1564)", () => {
    const now = Date.parse("2026-10-25T17:50:00.000Z");
    const future = { ...SECTION, generatedAt: new Date(now + 30 * 86_400_000).toISOString() };
    const d = diagnoseOrchestrationSection(future, now);
    expect(d.state).toBe("invalid");
    expect(d.failedChecks).toEqual(["generatedAt"]);
  });

  test("live: a generatedAt within the 60s future-skew tolerance stays live (#1564)", () => {
    const now = Date.parse("2026-10-25T17:50:00.000Z");
    const within = { ...SECTION, generatedAt: new Date(now + 30_000).toISOString() };
    expect(diagnoseOrchestrationSection(within, now).state).toBe("live");
  });

  test("age text: a non-finite age renders unknown, never NaNd old (#1564)", () => {
    expect(ageMsText(Number.NaN)).toBe("unknown");
    expect(feedAgeText("not a date")).toBe("unknown");
  });
});

describe("live source diagnostics + provenance detail (#1139/#1138)", () => {
  async function sourceWithFeed(over: Record<string, unknown>): Promise<ReturnType<typeof liveSource>> {
    const dir = await mkdtemp(join(tmpdir(), "pm-diag-"));
    const feedFile = join(dir, "fleet-feed.json");
    const now = new Date().toISOString();
    await writeFile(
      feedFile,
      JSON.stringify({
        version: 1,
        generatedAt: now,
        quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
        attention: [],
        ...over,
      }),
    );
    const cfg = loadConfig({
      ...Bun.env,
      PM_AGG_SOURCE: "live",
      PM_AGG_FEED_FILE: feedFile,
      PM_AGG_REGISTRY_FILE: join(dir, "registry.json"),
      PM_AGG_COMMENTS_FILE: join(dir, "comments.json"),
      PM_AGG_SNAPSHOTS_DIR: join(dir, "snapshots"),
      PM_AGG_OPENCHAMBER_STATE: join(dir, "no-state.json"),
      PM_PROJECTOR_URL: "off",
      KANEO_ADMIN_EMAIL: "",
      KANEO_ADMIN_PASSWORD: "",
      GRIST_API_KEY: "",
    });
    return liveSource(cfg);
  }

  const liveSection = () => ({
    ...SECTION,
    generatedAt: new Date().toISOString(),
    runId: "orch-diag-live",
    provenance: { sources: ["test"], runId: "orch-diag-live" },
  });

  test("diagnostics reflects the section the source actually holds", async () => {
    const ok = await sourceWithFeed({ orchestration: liveSection() });
    await ok.orchestration();
    expect(ok.diagnostics?.().orchestration.state).toBe("live");
    expect(ok.diagnostics?.().orchestration.runId).toBe("orch-diag-live");
    expect(ok.provenanceDetail?.().orchestration.state).toBe("live");

    const missing = await sourceWithFeed({});
    await missing.orchestration();
    expect(missing.diagnostics?.().orchestration.state).toBe("absent");

    const invalidSrc = await sourceWithFeed({
      orchestration: {
        version: 1,
        generatedAt: new Date().toISOString(),
        runId: "x",
        tiers: { needsDrew: [], needsOrchestrator: [] },
      },
    });
    await invalidSrc.orchestration();
    const d = invalidSrc.diagnostics?.().orchestration;
    expect(d?.state).toBe("invalid");
    expect(d?.failedChecks).toContain("provenance");
    expect(invalidSrc.provenanceDetail?.().orchestration.state).toBe("invalid");
  });

  test("registry facet counts the MERGED total, with its noun (#1138)", async () => {
    const src = await sourceWithFeed({
      orchestration: liveSection(),
      registry: [
        {
          id: "ses-diag-1",
          source: "opencode-sandbox",
          label: "Diag session",
          status: "idle",
          agentId: "opencode:main",
          agentLabel: "OpenCode · main (sandbox)",
          taskRefs: [],
        },
      ],
    });
    const records = await src.registry();
    expect(records.length).toBe(1);
    expect(src.provenance().registry).toContain("1 session records");
    expect(src.provenance().registry).toContain("1 from the latest feed");
    const facet = src.provenanceDetail?.().registry;
    expect(facet?.count).toBe(1);
    expect(facet?.countLabel).toBe("session records");
    expect(facet?.state).toBe("live");
  });

  test("registry and coverage share sanitized optional feed rows (#2539)", async () => {
    for (const registry of ["bad", { registry: "bad" }, 42, [null]]) {
      const src = await sourceWithFeed({ registry });
      const records = await src.registry();
      expect(Array.isArray(records)).toBe(true);

      const coverage = await src.coverage();
      const muse = coverage.find((row) => row.source === "claude-code");
      expect(muse?.wired).toBe(false);
      expect(muse?.detail).toContain("no launch records");
    }

    const clean = await sourceWithFeed({
      registry: [
        {
          id: "ses-muse-1",
          source: "claude-code",
          label: "Muse launch",
          status: "launched",
          agentId: "claude-code",
          agentLabel: "Muse",
          taskRefs: [],
        },
        {
          id: "ses-opencode-1",
          source: "opencode-sandbox",
          label: "OpenCode session",
          status: "idle",
          agentId: "opencode:main",
          agentLabel: "OpenCode · main (sandbox)",
          taskRefs: [],
        },
      ],
    });
    expect(await clean.registry()).toHaveLength(2);
    const muse = (await clean.coverage()).find((row) => row.source === "claude-code");
    expect(muse?.wired).toBe(true);
    expect(muse?.detail).toContain("1 recorded");
  });
});

// #6174: the Today-page sibling of the #6149 Needs-you fix. A lined-up
// task whose ref is not a GitHub ref (a tracked goal id is the live-data
// shape) passed the section gate unchecked and reached refUrl unguarded
// in the objectives render, 500ing /today and /frag/today.
describe("#6174: lined-up refs on the Today objectives path", () => {
  const QUOTA = {
    pools: [],
    burn: [],
    choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] },
  } satisfies QuotaView;

  function sectionWithRef(ref: unknown): OrchestrationSection {
    return {
      ...SECTION,
      goals: [
        {
          ...SECTION.goals[0]!,
          linedUp: [
            {
              title: "Link the Citi accounts in Plaid",
              ref: ref as LinedUpTask["ref"],
              status: "queued",
              horizon: "today",
              horizonWhy: "due today",
            },
          ],
        },
      ],
    };
  }

  function renderObjectives(section: OrchestrationSection): string {
    const today = todayFromSection(section);
    return renderTodayBody(today, buildAttention([], section), QUOTA, []);
  }

  test("a goal-id lined-up ref passes the gate and renders as plain code, never a link", () => {
    const section = sectionWithRef("goal_b647ff5c92f5");
    expect(isOrchestrationSection(section)).toBe(true);
    const html = renderObjectives(section);
    expect(html).toContain("Link the Citi accounts in Plaid");
    expect(html).toContain("<code>goal_b647ff5c92f5</code>");
    expect(html).not.toContain("github.com/goal_b647ff5c92f5");
  });

  test("a valid GitHub lined-up ref still renders as a link on the objective", () => {
    const section = sectionWithRef("developing-today/code#812");
    expect(isOrchestrationSection(section)).toBe(true);
    const html = renderObjectives(section);
    expect(html).toContain('href="https://github.com/developing-today/code/issues/812"');
    expect(html).toContain("developing-today/code#812");
  });

  test("a non-string lined-up ref fails the gate (designed fallback, no render throw)", () => {
    expect(isOrchestrationSection(sectionWithRef(12345))).toBe(false);
  });
});
