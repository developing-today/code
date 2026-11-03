import { describe, expect, test } from "bun:test";
import {
  buildAttention,
  chicagoYmd,
  deriveToday,
  monthTimeline,
  partitionFresh,
  roadmapDayGrain,
  sessionChildren,
  sessionRoots,
} from "../src/derive.ts";
import { type OrchestrationSection, type RoadmapRow, type SessionFeedEntry, type TierItem } from "../src/model.ts";
import { renderAttention, renderRoadmap, renderToday } from "../src/render.ts";
import {
  FIXTURE_ATTENTION,
  FIXTURE_BOARD,
  FIXTURE_ITEMS,
  FIXTURE_ROADMAP,
  fixtureSource,
} from "../src/sources/fixtures.ts";
import { julesSummary, mapJulesItems, type JulesReviewState } from "../src/sources/jules.ts";

const NOW = Date.parse("2026-10-25T15:00:00Z");
const iso = (ms: number) => new Date(ms).toISOString();

const tierItem = (over: Partial<TierItem>): TierItem => ({
  id: "x",
  kind: "task",
  title: "Item",
  source: "github",
  reason: "Because.",
  since: iso(NOW),
  lastSeenAt: iso(NOW),
  ...over,
});

const section = (over: Partial<OrchestrationSection>): OrchestrationSection => ({
  version: 1,
  generatedAt: iso(NOW),
  runId: "orch-test",
  provenance: { sources: ["test run"] },
  tiers: { needsDrew: [], needsOrchestrator: [] },
  dependencies: [],
  goals: [],
  actions: [],
  ...over,
});

/** The fixture-feed section: ses_h_connect escalated to Drew by #899. */
const SECTION = section({
  tiers: {
    needsDrew: [
      tierItem({
        id: "ses_h_connect",
        kind: "session",
        sessionId: "ses_h_connect",
        title: "Connect the app",
        source: "opencode-laptop",
        reason: "Only Drew can pick which repo installs the developing.today app first.",
      }),
      tierItem({
        id: "goal_plaid",
        kind: "task",
        title: "Link the Citi accounts in Plaid",
        source: "tracked",
        reason: "Only Drew can complete the bank sign-in in his own browser.",
      }),
    ],
    needsOrchestrator: [],
  },
});

describe("freshness expiry (#898 §2/§5)", () => {
  test("finished runs expire after 2h; anything after 24h; fresh stays", () => {
    const feed: SessionFeedEntry[] = [
      { sessionId: "fresh-err", lastActivityAt: iso(NOW - 10 * 60_000), lastTurnError: "boom", activeToolCalls: 0 },
      { sessionId: "done-3h", lastActivityAt: iso(NOW - 3 * 3_600_000), outcome: "succeeded", activeToolCalls: 0 },
      {
        sessionId: "done-1h",
        lastActivityAt: iso(NOW - 1 * 3_600_000),
        outcome: "failed",
        lastTurnError: "x",
        activeToolCalls: 0,
      },
      { sessionId: "ancient", lastActivityAt: iso(NOW - 30 * 3_600_000), activeToolCalls: 1 },
      { sessionId: "no-time", lastActivityAt: null, lastTurnError: "boom", activeToolCalls: 0 },
    ];
    const { current, expired } = partitionFresh(feed, NOW);
    expect(current.map((e) => e.sessionId).sort()).toEqual(["done-1h", "fresh-err", "no-time"]);
    expect(expired.map((e) => e.sessionId).sort()).toEqual(["ancient", "done-3h"]);
  });
});

describe("tiering (#898 §2)", () => {
  const entry = (extra: Partial<SessionFeedEntry>): SessionFeedEntry => ({
    sessionId: "s",
    lastActivityAt: iso(NOW - 60_000),
    activeToolCalls: 0,
    ...extra,
  });

  test("decision-shaped questions need Drew, with the reason stated", () => {
    const view = buildAttention([entry({ awaitingQuestion: "Should I merge this now, or wait?" })], null, NOW);
    expect(view.drew).toHaveLength(1);
    expect(view.drew[0].drewReason).toContain("Waiting on your decision");
  });

  test("routine questions and errors are the orchestrator's", () => {
    const view = buildAttention(
      [
        entry({ sessionId: "q", awaitingQuestion: "What is the capital of France?" }),
        entry({ sessionId: "e", lastTurnError: "429 subscription quota exhausted" }),
        entry({ sessionId: "p", awaitingQuestion: "Permission pending: bash: bun test" }),
      ],
      null,
      NOW,
    );
    expect(view.drew).toHaveLength(0);
    expect(view.orchestrator).toHaveLength(3);
    expect(view.orchestrator.every((e) => (e.orchestratorNote ?? "").length > 0)).toBe(true);
  });

  test("destructive permission requests need Drew", () => {
    const view = buildAttention([entry({ awaitingQuestion: "Permission pending: bash: rm -rf build" })], null, NOW);
    expect(view.drew).toHaveLength(1);
    expect(view.drew[0].drewReason).toContain("destructive");
  });

  test("credential walls need Drew", () => {
    const view = buildAttention(
      [entry({ lastTurnError: "sign-in required: verification code sent to your email" })],
      null,
      NOW,
    );
    expect(view.drew).toHaveLength(1);
    expect(view.drew[0].drewReason).toContain("sign-in only you can complete");
  });

  test("the orchestration section's classification overrides local rules, reason verbatim", () => {
    const sec = section({
      tiers: {
        needsDrew: [tierItem({ id: "s", kind: "session", sessionId: "s", reason: "Only Drew can do this thing." })],
        needsOrchestrator: [],
      },
    });
    const view = buildAttention([entry({ lastTurnError: "plain error" })], sec, NOW);
    expect(view.drew).toHaveLength(1);
    expect(view.drew[0].drewReason).toBe("Only Drew can do this thing.");
    // Section items not backed by a live session in this feed (a task,
    // or a session on a server outside the feed) render as tier cards.
    const withExtra = buildAttention([entry({ lastTurnError: "plain error" })], SECTION, NOW);
    expect(withExtra.drewExtra.map((i) => i.id)).toEqual(["ses_h_connect", "goal_plaid"]);
  });

  test("Jules questions belong to the keeper, not Drew", () => {
    const view = buildAttention(
      [
        entry({
          sessionId: "4259424002214698449",
          source: "jules",
          awaitingQuestion: "Does this look good, and should I proceed?",
        }),
      ],
      null,
      NOW,
    );
    expect(view.drew).toHaveLength(0);
    expect(view.orchestrator).toHaveLength(1);
  });

  test("in-motion sessions are in neither queue", () => {
    const view = buildAttention([entry({ activeToolCalls: 2 })], null, NOW);
    expect(view.drew).toHaveLength(0);
    expect(view.orchestrator).toHaveLength(0);
    expect(view.all[0].tier).toBe("in-motion");
  });
});

describe("session hierarchy (#898 §5)", () => {
  test("children group under parents; roots exclude them", () => {
    const view = buildAttention(FIXTURE_ATTENTION, SECTION);
    const children = sessionChildren(view.all);
    const roots = sessionRoots(view.all);
    expect(children.get("ses_k_wip")?.map((e) => e.sessionId)).toEqual(["ses_k_wip_explore"]);
    expect(roots.some((e) => e.sessionId === "ses_k_wip_explore")).toBe(false);
    expect(roots.some((e) => e.sessionId === "ses_k_wip")).toBe(true);
    // The finished fixture probe expired out entirely.
    expect(view.all.some((e) => e.sessionId === "ses_old_probe")).toBe(false);
    expect(view.expiredCount).toBeGreaterThanOrEqual(1);
    // The section's classification puts ses_h_connect in NEEDS DREW.
    expect(view.drew.some((e) => e.sessionId === "ses_h_connect")).toBe(true);
  });
});

describe("jules mapping (#898 §5)", () => {
  const state: JulesReviewState = {
    last_run: "2026-10-25T14:58:00Z",
    items: {
      "4259424002214698449": {
        state: "AWAITING_USER_FEEDBACK",
        title: "Fix #811: a joiner's resize",
        source: "sources/github/developing-today/code",
        created: "2026-10-25T12:44:14Z",
        last_action: "replied",
        last_action_at: "2026-10-25T14:58:25Z",
        note: "Agent reported work done and asked 'Does this look good, and should I proceed?' Replied.",
      },
      "999": {
        state: "FAILED",
        title: "Broke",
        source: "sources/github/developing-today/opencode",
        last_action_at: "2026-10-25T10:00:00Z",
      },
    },
    jules_note_1458:
      "Jules list this run: 84 sessions - 77 COMPLETED, 6 FAILED, 1 AWAITING_USER_FEEDBACK, 0 AWAITING_PLAN_APPROVAL, 0 IN_PROGRESS. 1 Jules response sent.",
  };

  test("maps keeper items to Jules feed entries", () => {
    const entries = mapJulesItems(state);
    const awaiting = entries.find((e) => e.sessionId === "4259424002214698449")!;
    expect(awaiting.source).toBe("jules");
    expect(awaiting.directory).toBe("developing-today/code");
    expect(awaiting.awaitingQuestion).toBe("Does this look good, and should I proceed?");
    const failed = entries.find((e) => e.sessionId === "999")!;
    expect(failed.outcome).toBe("failed");
    expect(failed.lastTurnError).toContain("failed");
  });

  test("parses keeper totals", () => {
    const s = julesSummary(state);
    expect(s.total).toBe(84);
    expect(s.completed).toBe(77);
    expect(s.failed).toBe(6);
    expect(s.tracked).toBe(2);
    expect(s.at).toBe("2026-10-25T14:58:00Z");
  });

  test("ties totals to the last run across midnight, not the lexicographic max (#1587)", () => {
    // Yesterday's late note must not outrank today's note all day.
    const s = julesSummary({
      last_run: "2026-10-26T00:10:00Z",
      items: {},
      jules_note_2359:
        "Jules list this run: 84 sessions - 77 COMPLETED, 6 FAILED, 1 AWAITING_USER_FEEDBACK, 0 AWAITING_PLAN_APPROVAL, 0 IN_PROGRESS. 1 Jules response sent.",
      jules_note_0010:
        "Jules list this run: 12 sessions - 3 COMPLETED, 1 FAILED, 2 AWAITING_USER_FEEDBACK, 1 AWAITING_PLAN_APPROVAL, 5 IN_PROGRESS. 1 Jules response sent.",
    });
    expect(s.total).toBe(12);
    expect(s.completed).toBe(3);
    expect(s.failed).toBe(1);
    expect(s.at).toBe("2026-10-26T00:10:00Z");
  });

  test("a stale late-evening note does not stand in for the morning run (#1587)", () => {
    const s = julesSummary({
      last_run: "2026-10-26T09:30:00Z",
      items: {},
      jules_note_2359:
        "Jules list this run: 84 sessions - 77 COMPLETED, 6 FAILED, 1 AWAITING_USER_FEEDBACK, 0 AWAITING_PLAN_APPROVAL, 0 IN_PROGRESS. 1 Jules response sent.",
      jules_note_0930:
        "Jules list this run: 20 sessions - 9 COMPLETED, 2 FAILED, 3 AWAITING_USER_FEEDBACK, 1 AWAITING_PLAN_APPROVAL, 5 IN_PROGRESS. 1 Jules response sent.",
    });
    expect(s.total).toBe(20);
    expect(s.completed).toBe(9);
    expect(s.failed).toBe(2);
  });

  test("an unpadded note key never outranks a padded later one (#1587)", () => {
    const s = julesSummary({
      last_run: "2026-10-27T14:58:00Z",
      items: {},
      jules_note_905:
        "Jules list this run: 50 sessions - 40 COMPLETED, 5 FAILED, 1 AWAITING_USER_FEEDBACK, 1 AWAITING_PLAN_APPROVAL, 3 IN_PROGRESS. 1 Jules response sent.",
      jules_note_1458:
        "Jules list this run: 84 sessions - 77 COMPLETED, 6 FAILED, 1 AWAITING_USER_FEEDBACK, 0 AWAITING_PLAN_APPROVAL, 0 IN_PROGRESS. 1 Jules response sent.",
    });
    expect(s.total).toBe(84);
    expect(s.completed).toBe(77);
    expect(s.failed).toBe(6);
  });

  test("omits totals when no note can be tied to the last run (#1587)", () => {
    const s = julesSummary({
      last_run: "2026-10-26T10:00:00Z",
      items: {},
      jules_note_0930:
        "Jules list this run: 20 sessions - 9 COMPLETED, 2 FAILED, 3 AWAITING_USER_FEEDBACK, 1 AWAITING_PLAN_APPROVAL, 5 IN_PROGRESS. 1 Jules response sent.",
    });
    expect(s.total).toBeUndefined();
    expect(s.at).toBe("2026-10-26T10:00:00Z");
    expect(s.tracked).toBe(0);
  });
});

describe("roadmap grains (#898 §3)", () => {
  test("month timeline places dated rows by month and keeps quarter grain", () => {
    const tl = monthTimeline(FIXTURE_ROADMAP);
    const keys = tl.lanes.map((l) => l.key);
    expect(keys).toContain("2026-10");
    expect(keys).toContain("2026-Q4");
  });

  test("day grains window the dated rows and count the rest honestly", () => {
    const now = new Date("2026-10-24T18:00:00Z"); // Sat Oct 24, Chicago
    expect(chicagoYmd(now)).toBe("2026-10-24");
    const today = roadmapDayGrain(FIXTURE_ROADMAP, "today", now);
    expect(today.lanes).toHaveLength(1);
    expect(today.lanes[0].rows.length).toBeGreaterThan(0); // the 2026-10-24 seed row
    expect(today.coarse.length).toBeGreaterThan(0); // quarter-grain rows
    const week = roadmapDayGrain(FIXTURE_ROADMAP, "week", now);
    expect(week.lanes).toHaveLength(7);
  });

  test("roadmap renders the horizon switcher at day grain", async () => {
    const src = fixtureSource();
    const row: RoadmapRow = {
      id: 99,
      subject: "grist:doc/Roadmap/99",
      title: "Dated today row",
      area: "PM stack",
      status: "Planned",
      horizon: chicagoYmd(),
      source: "",
      notes: "",
      links: [],
    };
    const html = renderRoadmap([...FIXTURE_ROADMAP, row], await src.items([]), src.provenance(), "today");
    expect(html).toContain("Next 3 days");
    expect(html).toContain("Dated today row");
    expect(html).toContain("no calendar date");
  });
});

describe("today view (#898 §4)", () => {
  const items = new Map(FIXTURE_ITEMS.map((i) => [i.ref, i]));

  test("renders the orchestration section when present", () => {
    const sec = section({
      goals: [
        {
          goalId: null,
          goal: "Keep the four repos healthy",
          momentum: "blocked",
          summary: "Waiting on the GitHub App route.",
          linedUp: [
            {
              title: "Ship aggregator slice 2",
              status: "in-progress",
              horizon: "today",
              horizonWhy: "Drew asked for it today",
              blockedBy: [{ task: "CI green", status: "in-progress" }],
            },
          ],
          progressedSinceLastRun: ["PR #892 merged"],
          stalled: [],
          gaps: [],
        },
      ],
    });
    const today = deriveToday(FIXTURE_BOARD, FIXTURE_ROADMAP, items, sec);
    expect(today.fromOrchestrator).toBe(true);
    expect(today.objectives[0].title).toBe("Ship aggregator slice 2");
    expect(today.objectives[0].chip).toContain("orchestrator");
    expect(today.objectives[0].detail).toContain("blocked by CI green");
    expect(today.momentum[0].state).toBe("blocked");
    expect(today.momentum[0].accomplished).toEqual(["PR #892 merged"]);
  });

  test("tolerates a string-shaped progressedSinceLastRun and renders it", async () => {
    // Regression for the /today 500: the orchestrator published each
    // goal's progressedSinceLastRun as a single string, and renderToday
    // called .map on it. The section type declares string[], so the
    // string shape arrives via a cast, exactly as it does at runtime.
    const sec = section({
      goals: [
        {
          goalId: null,
          goal: "Keep the four repos healthy",
          momentum: "moving",
          summary: "Sweeps are landing.",
          linedUp: [],
          progressedSinceLastRun: "PR #892 merged" as unknown as string[],
          stalled: [],
          gaps: [],
        },
      ],
    });
    const today = deriveToday(FIXTURE_BOARD, FIXTURE_ROADMAP, items, sec);
    expect(today.momentum[0].accomplished).toEqual(["PR #892 merged"]);
    const src = fixtureSource();
    const attention = buildAttention(await src.attention(), null);
    const html = renderToday(today, attention, await src.quota(), await src.coverage(), src.provenance());
    expect(html).toContain("Recently accomplished");
    expect(html).toContain("PR #892 merged");
  });

  test("renderToday coerces a non-array accomplished instead of crashing", async () => {
    // Belt-and-braces for the same regression: a TodayView that bypasses
    // derive.ts normalization must still render.
    const today = {
      fromOrchestrator: true,
      generatedAt: iso(NOW),
      objectives: [],
      momentum: [
        {
          project: "Keep the four repos healthy",
          state: "moving" as const,
          accomplished: "PR #892 merged" as unknown as string[],
        },
      ],
    };
    const src = fixtureSource();
    const attention = buildAttention(await src.attention(), null);
    const html = renderToday(today, attention, await src.quota(), await src.coverage(), src.provenance());
    expect(html).toContain("Recently accomplished");
    expect(html).toContain("PR #892 merged");
  });

  test("derives objectives + momentum without a summary, and says so on the page", async () => {
    const today = deriveToday(FIXTURE_BOARD, FIXTURE_ROADMAP, items, null);
    expect(today.fromOrchestrator).toBe(false);
    expect(today.objectives.length).toBeGreaterThan(0);
    expect(today.momentum.length).toBeGreaterThan(0);
    const src = fixtureSource();
    const attention = buildAttention(await src.attention(), null);
    const html = renderToday(today, attention, await src.quota(), await src.coverage(), src.provenance());
    expect(html).toContain("Top objectives");
    expect(html).toContain("Project momentum");
    expect(html).toContain("has not published a goal-measurement section yet");
  });
});

describe("attention render (slice 2)", () => {
  test("tiers, reasons, source labels, collapse, coverage, expiry all render", async () => {
    const src = fixtureSource();
    const view = buildAttention(await src.attention(), SECTION);
    const html = renderAttention(view, await src.coverage(), src.provenance());
    expect(html).toContain("Needs User");
    // Section present: counts render as numbers and the tier header
    // carries the section's run id — no honesty banner (#1139).
    expect(html).not.toContain("Needs User (unknown)");
    expect(html).not.toContain("Tier source missing");
    expect(html).toContain("orch-test");
    expect(html).toContain("Why you:");
    expect(html).toContain("Only Drew can pick which repo installs");
    expect(html).toContain("OpenCode · laptop");
    expect(html).toContain("Jules");
    expect(html).toContain("sub-session");
    expect(html).toContain("Session coverage");
    expect(html).toContain("not wired");
    expect(html).toContain("expired out of this view");
    expect(html).not.toContain("one-shot probe"); // the expired fixture entry
  });
});

describe("attention honesty (#1139)", () => {
  test("absent section: banner renders, counts are unknown, never a confident zero", async () => {
    const src = fixtureSource();
    const view = buildAttention(await src.attention(), null);
    expect(view.tierSource.present).toBe(false);
    expect(view.tierSource.state).toBe("absent");
    const html = renderAttention(view, await src.coverage(), src.provenance());
    expect(html).toContain("Tier source missing");
    expect(html).toContain("the orchestrator's published queues are NOT reflected");
    expect(html).toContain("Needs User (unknown)");
    expect(html).toContain("Needs Orchestrator (unknown)");
    expect(html).not.toContain("Needs User (0)");
    // The false-reassurance empty copy is suppressed in this state.
    expect(html).not.toContain("Nothing in the user queue right now.");
  });

  test("invalid section: the banner names the failed check", async () => {
    const src = fixtureSource();
    const view = buildAttention(await src.attention(), null, Date.now(), {
      state: "invalid",
      failedChecks: ["provenance"],
    });
    expect(view.tierSource.present).toBe(false);
    const html = renderAttention(view, await src.coverage(), src.provenance());
    expect(html).toContain("Tier source invalid");
    expect(html).toContain("provenance");
    expect(html).toContain("Needs User (unknown)");
  });

  test("stale section: counts render, badged with a stale banner and the run stamp", async () => {
    const src = fixtureSource();
    const view = buildAttention(await src.attention(), SECTION, Date.now(), {
      state: "stale",
      runId: SECTION.runId,
      generatedAt: SECTION.generatedAt,
    });
    expect(view.tierSource.present).toBe(true);
    const html = renderAttention(view, await src.coverage(), src.provenance());
    expect(html).toContain("Tier source stale");
    expect(html).not.toContain("Needs User (unknown)");
    expect(html).toContain("orch-test");
  });

  test("today's derived banner names an invalid section instead of 'not published'", async () => {
    const src = fixtureSource();
    const items = new Map(FIXTURE_ITEMS.map((i) => [i.ref, i]));
    const today = deriveToday(FIXTURE_BOARD, FIXTURE_ROADMAP, items, null, new Date(), {
      state: "invalid",
      failedChecks: ["tiers.needsDrew"],
    });
    const attention = buildAttention(await src.attention(), null);
    const html = renderToday(today, attention, await src.quota(), await src.coverage(), src.provenance());
    expect(html).toContain("failed validation");
    expect(html).toContain("tiers.needsDrew");
    expect(html).not.toContain("has not published a goal-measurement section yet");
  });
});
