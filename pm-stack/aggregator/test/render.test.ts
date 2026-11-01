import { describe, expect, test } from "bun:test";
import {
  attentionSorted,
  buildAttention,
  classifyAttention,
  deriveToday,
  executionStrip,
  isCalendarDate,
  laneFor,
  monthLabel,
  monthTimeline,
  roadmapDayGrain,
  rowDate,
  timeline,
  ymdLabel,
  allRefs,
} from "../src/derive.ts";
import {
  parseRef,
  refKey,
  refsInText,
  refUrl,
  type BoardItem,
  type GhItem,
  type ProvenanceDetail,
  type RefKey,
  type RoadmapRow,
  type SessionFeedEntry,
} from "../src/model.ts";
import {
  FIXTURE_ATTENTION,
  FIXTURE_BOARD,
  FIXTURE_ITEMS,
  FIXTURE_ROADMAP,
  fixtureSource,
} from "../src/sources/fixtures.ts";
import {
  renderAttention,
  renderBoard,
  renderItem,
  renderQuota,
  renderRoadmap,
  renderSessions,
  renderToday,
} from "../src/render.ts";

const items = new Map(FIXTURE_ITEMS.map((i) => [i.ref, i]));

describe("ref parsing", () => {
  test("parses repo-qualified refs", () => {
    expect(parseRef("developing-today/code#752")).toEqual({ repo: "developing-today/code", number: 752 });
    expect(refKey({ repo: "dezren39/nix", number: 501 })).toBe("dezren39/nix#501");
  });
  test("extracts refs from GitHub URLs in text", () => {
    const refs = refsInText(
      "see https://github.com/developing-today/code/pull/752 and https://github.com/dezren39/nix/issues/501",
    );
    expect(refs.sort()).toEqual(["developing-today/code#752", "dezren39/nix#501"].sort());
  });
  test("refUrl builds issue vs pr paths", () => {
    expect(refUrl("developing-today/code#752", "pr")).toBe("https://github.com/developing-today/code/pull/752");
    expect(refUrl("developing-today/code#752")).toBe("https://github.com/developing-today/code/issues/752");
  });
});

describe("execution strip", () => {
  test("classifies a mixed set into implementing / review / merged / queued", () => {
    const strip = executionStrip(
      [
        { ref: "developing-today/code#752", relation: "tracks" },
        { ref: "developing-today/code#763", relation: "implements" },
        { ref: "developing-today/code#762", relation: "tracks" },
        { ref: "developing-today/code#900", relation: "implements" },
      ],
      items,
    );
    expect(strip.merged.map((i) => i.ref)).toContain("developing-today/code#752");
    expect(strip.review.map((i) => i.ref)).toContain("developing-today/code#763");
    expect(strip.implementing.map((i) => i.ref)).toContain("developing-today/code#900");
    // #762 is labelled ready-for-agent, so it counts as implementing.
    expect(strip.implementing.map((i) => i.ref)).toContain("developing-today/code#762");
  });
  test("records unresolved refs", () => {
    const strip = executionStrip([{ ref: "nowhere/repo#9999", relation: "tracks" }], items);
    expect(strip.unknown).toContain("nowhere/repo#9999");
  });
});

describe("roadmap timeline", () => {
  test("folds dates into quarters and keeps unscheduled last", () => {
    const rows: RoadmapRow[] = [
      {
        id: 1,
        subject: "grist:d/1",
        title: "a",
        area: "T",
        status: "Done",
        horizon: "Q4 2026",
        source: "",
        notes: "",
        links: [],
      },
      {
        id: 2,
        subject: "grist:d/2",
        title: "b",
        area: "T",
        status: "Idea",
        horizon: "2026-10-24",
        source: "",
        notes: "",
        links: [],
      },
      {
        id: 3,
        subject: "grist:d/3",
        title: "c",
        area: "T",
        status: "Idea",
        horizon: "Unscheduled",
        source: "",
        notes: "",
        links: [],
      },
    ];
    const tl = timeline(rows);
    const keys = tl.lanes.map((l) => l.key);
    expect(keys[0]).toBe("2026-Q4");
    expect(keys[keys.length - 1]).toBe("unscheduled");
    expect(tl.themes).toContain("T");
  });
  test("lanes for dates and quarters", () => {
    expect(laneFor("Q1 2027").key).toBe("2027-Q1");
    expect(laneFor("2026-10-24").key).toBe("2026-Q4");
    expect(laneFor("Ongoing").key).toBe("ongoing");
  });
});

describe("horizon calendar-date validation (#1628)", () => {
  const row = (horizon: string, id = 1): RoadmapRow => ({
    id,
    subject: `grist:d/${id}`,
    title: `row ${id}`,
    area: "T",
    status: "Idea",
    horizon,
    source: "",
    notes: "",
    links: [],
  });

  test("isCalendarDate rejects impossible dates, keeps real ones", () => {
    expect(isCalendarDate(2026, 13, 15)).toBe(false);
    expect(isCalendarDate(2026, 0, 15)).toBe(false);
    expect(isCalendarDate(2026, 1, 0)).toBe(false);
    expect(isCalendarDate(2026, 2, 30)).toBe(false);
    expect(isCalendarDate(2026, 4, 31)).toBe(false);
    expect(isCalendarDate(2026, 2, 29)).toBe(false);
    expect(isCalendarDate(2100, 2, 29)).toBe(false);
    expect(isCalendarDate(2028, 2, 29)).toBe(true);
    expect(isCalendarDate(2000, 2, 29)).toBe(true);
    expect(isCalendarDate(2026, 10, 24)).toBe(true);
  });

  test("laneFor sends impossible dates to the other lane", () => {
    expect(laneFor("2026-13-15").key).toStartWith("other:");
    expect(laneFor("2026-00-15").key).toStartWith("other:");
    expect(laneFor("2026-02-30").key).toStartWith("other:");
    expect(laneFor("2026-10-24").key).toBe("2026-Q4");
    expect(laneFor("2028-02-29").key).toBe("2028-Q1");
  });

  test("rowDate returns null for impossible dates", () => {
    expect(rowDate(row("2026-13-15"))).toBeNull();
    expect(rowDate(row("2026-00-15"))).toBeNull();
    expect(rowDate(row("2026-02-30"))).toBeNull();
    expect(rowDate(row("2026-04-31"))).toBeNull();
    expect(rowDate(row("2026-10-24"))).toBe("2026-10-24");
    expect(rowDate(row("2028-02-29"))).toBe("2028-02-29");
  });

  test("monthTimeline never renders an 'undefined' lane label", () => {
    const tl = monthTimeline([row("2026-13-15", 1), row("2026-10-24", 2)]);
    const labels = tl.lanes.map((l) => l.label);
    expect(labels).toContain("October 2026");
    expect(labels.some((l) => l.includes("undefined"))).toBe(false);
    expect(tl.lanes.some((l) => l.key.startsWith("other:"))).toBe(true);
  });

  test("roadmapDayGrain counts impossible dates as coarse, not outside", () => {
    const now = new Date("2026-10-28T12:00:00Z");
    const g = roadmapDayGrain([row("2026-02-30", 1), row("2026-13-15", 2), row("2026-12-25", 3)], "week", now);
    expect(g.coarse.map((r) => r.horizon)).toEqual(["2026-02-30", "2026-13-15"]);
    expect(g.outside).toBe(1);
  });

  test("ymdLabel and monthLabel neither throw nor relabel impossible dates", () => {
    expect(ymdLabel("2026-13-15")).toBe("2026-13-15");
    expect(ymdLabel("2026-02-30")).toBe("2026-02-30");
    expect(ymdLabel("2026-10-24")).toBe("Sat Oct 24");
    expect(monthLabel("2026-13")).toBe("2026-13");
    expect(monthLabel("2026-00")).toBe("2026-00");
    expect(monthLabel("2026-10")).toBe("October 2026");
  });
});

describe("attention classification", () => {
  test("orders stopped-for-question and errored first", () => {
    const feed: SessionFeedEntry[] = [
      { sessionId: "s1", lastActivityAt: "2026-10-25T00:00:00Z", activeToolCalls: 1 },
      { sessionId: "s2", lastActivityAt: "2026-10-25T00:00:00Z", awaitingQuestion: "which repo first?" },
      { sessionId: "s3", lastActivityAt: "2026-10-25T00:00:00Z", lastTurnError: "429 quota" },
    ];
    const sorted = attentionSorted(feed);
    expect(sorted[0].state).toBe("stopped-for-question");
    expect(sorted[1].state).toBe("errored");
    expect(sorted[2].state).toBe("in-motion");
  });
  test("classifies needs-poke, stalled-on-cli, stuck-retrying", () => {
    const now = Date.parse("2026-10-25T02:00:00Z");
    const poke = classifyAttention({ sessionId: "p", lastActivityAt: "2026-10-24T00:00:00Z", activeToolCalls: 0 }, now);
    expect(poke.state).toBe("needs-poke");
    const stall = classifyAttention(
      {
        sessionId: "s",
        lastActivityAt: "2026-10-25T01:00:00Z",
        longRunningCall: { command: "cargo build", startedAt: "2026-10-25T00:00:00Z" },
      },
      now,
    );
    expect(stall.state).toBe("stalled-on-cli");
    const retry = classifyAttention(
      { sessionId: "r", lastActivityAt: "2026-10-25T01:00:00Z", repeatedErrorCount: 4 },
      now,
    );
    expect(retry.state).toBe("stuck-retrying");
  });
});

describe("fixture source", () => {
  test("board and roadmap are non-empty and shaped", async () => {
    const src = fixtureSource();
    const board = await src.board();
    const rows = await src.roadmap();
    expect(board.length).toBeGreaterThan(0);
    expect(rows.length).toBe(23); // the real Grist seed (grew 16 → 23 with the cluster/id rows)
    const crossRepo = board.find((b) => b.id.includes("aggregator"));
    expect(crossRepo?.links.some((l) => l.ref.startsWith("developing-today/openchamber"))).toBe(true);
  });
});

describe("render smoke (fixtures)", () => {
  test("board renders cards and execution strips", async () => {
    const src = fixtureSource();
    const board = await src.board();
    const refs = allRefs(board);
    const html = renderBoard(board, await src.items(refs), src.provenance());
    expect(html).toContain("Cross-repo WIP board");
    expect(html).toContain("implementing:");
    expect(html).toContain("merged:");
  });
  test("roadmap renders a timeline with lanes", async () => {
    const src = fixtureSource();
    const rows = await src.roadmap();
    const html = renderRoadmap(rows, await src.items(allRefs(rows)), src.provenance());
    expect(html).toContain("Roadmap");
    expect(html).toContain("Theme");
    expect(html).toContain("Q4 2026");
  });
  test("item drill-down renders linked refs and the comment stream", async () => {
    const src = fixtureSource();
    const item = FIXTURE_BOARD.find((b) => b.id.includes("aggregator")) as BoardItem;
    const refs = allRefs([item]);
    const html = renderItem(
      item.title,
      item.id,
      item.links,
      await src.items(refs),
      await src.comments(refs),
      src.provenance(),
    );
    expect(html).toContain("Linked issues / PRs");
    expect(html).toContain("Activity stream");
    expect(html).toContain("developing-today/openchamber#210");
    expect(html).not.toContain("Planning notes");
    expect(html).not.toContain("PM-stack design & phase plan");
  });
  test("quota view renders pools and model-choice guidance", async () => {
    const src = fixtureSource();
    const html = renderQuota(await src.quota(), src.provenance());
    expect(html).toContain("Quota + throughput");
    expect(html).toContain("Headroom for a new session");
    expect(html).toContain("Antigravity");
  });
  test("attention view renders the tiered needs-you page", async () => {
    const src = fixtureSource();
    const view = buildAttention(await src.attention(), await src.orchestration());
    const html = renderAttention(view, await src.coverage(), src.provenance());
    expect(html).toContain("Needs you");
    expect(html).toContain("Needs User");
    expect(html).toContain("Needs Orchestrator");
    // No orchestration section in fixtures: the honesty banner shows
    // and the tier heading reports "unknown", not a number (#1139).
    expect(html).toContain("Tier source missing");
    expect(html).toContain("Needs User (unknown)");
    expect(html).not.toContain("Needs User (0)");
    expect(html).toContain("waiting for an answer");
    expect(html).toContain("Waiting on:");
  });
});

describe("freshness chips (#1138)", () => {
  const detail: ProvenanceDetail = {
    board: { text: "fixtures", state: "fixtures" },
    roadmap: { text: "fixtures", state: "fixtures" },
    items: { text: "fixtures", state: "fixtures" },
    comments: { text: "fixtures", state: "fixtures" },
    quota: { text: "live (fleet feed)", state: "live", ageMs: 120_000 },
    usage: { text: "fixtures", state: "fixtures" },
    attention: { text: "live (laptop live)", state: "live" },
    orchestration: { text: "live (orchestration section orch-x)", state: "live", ageMs: 600_000 },
    dns: { text: "fixtures", state: "fixtures" },
    registry: {
      text: "live (221 session records · 61 from the latest feed)",
      state: "live",
      ageMs: 180_000,
      count: 221,
      countLabel: "session records",
    },
    aa: { text: "fixtures", state: "fixtures" },
  };

  test("sessions chip carries the merged count with its noun; footer detail is collapsed", async () => {
    const src = fixtureSource();
    const html = renderSessions(await src.registry(), src.provenance(), detail);
    expect(html).toContain('class="fchip live"');
    expect(html).toContain("registry · live · 221 session records");
    // The ten-string provenance line survives, inside <details>.
    expect(html).toContain("<details");
    expect(html).toContain("source detail");
    expect(html).toContain("board: fixtures");
  });

  test("today renders chips for its own sources only", async () => {
    const src = fixtureSource();
    const today = deriveToday(FIXTURE_BOARD, FIXTURE_ROADMAP, items, null);
    const attention = buildAttention(await src.attention(), null);
    const html = renderToday(today, attention, await src.quota(), await src.coverage(), src.provenance(), detail);
    expect(html).toContain("orchestration · live");
    expect(html).toContain("quota · live");
    expect(html).not.toContain("dns · fixtures");
    expect(html).not.toContain("registry · live");
  });

  test("no detail, no chips — older callers render as before", async () => {
    const src = fixtureSource();
    const html = renderSessions(await src.registry(), src.provenance());
    expect(html).not.toContain('class="fchip');
  });
});
