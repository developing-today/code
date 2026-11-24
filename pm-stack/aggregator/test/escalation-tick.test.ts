import { describe, expect, test } from "bun:test";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { EscalationJournalEntry, PagePayload } from "../src/escalation.ts";
import {
  createEscalationTicker,
  escalationDecisionKey,
  escalationDelivery,
  escalationJournalLine,
} from "../src/escalation-tick.ts";
import { emptyState, type RemediationState } from "../src/remediation.ts";
import type { SloFacetSummary } from "../src/slo.ts";
import { SloJournal, groupSamplesByFacet } from "../src/slo-journal.ts";

const NOW = Date.parse("2026-11-22T12:00:00.000Z");

function summary(over: Partial<SloFacetSummary> = {}): SloFacetSummary {
  return {
    id: "projector-up",
    label: "Projector up",
    currentValue: 100,
    target: "99.5% of minutes",
    budgetRemainingPercent: 80,
    burnRate1h: 1,
    state: "burning",
    badMinutes: 10,
    budgetMinutes: 216,
    unknownSamples: 0,
    sourced: true,
    latestBad: true,
    ...over,
  };
}

interface Captured {
  journal: EscalationJournalEntry[];
  feed: EscalationJournalEntry[];
  pages: PagePayload[];
}

function fakeSink(captured: Captured, pushOutcome: unknown = { ok: true, status: 201 }) {
  return {
    journal: async (entry: EscalationJournalEntry) => void captured.journal.push(entry),
    feedEvent: async (entry: EscalationJournalEntry) => void captured.feed.push(entry),
    push: async (page: PagePayload) => {
      captured.pages.push(page);
      return pushOutcome;
    },
  };
}

function tickerFor(summaries: () => SloFacetSummary[], captured: Captured, pushOutcome?: unknown) {
  return createEscalationTicker({
    summaries: async () => summaries(),
    remediationState: (now: number): RemediationState => emptyState(now),
    sink: fakeSink(captured, pushOutcome),
    now: () => NOW,
  });
}

describe("escalation tick (#9991, design §4 wiring)", () => {
  test("a bad stateful facet pages on the first tick: journal + push, no feed event", async () => {
    const captured: Captured = { journal: [], feed: [], pages: [] };
    const ticker = tickerFor(() => [summary({ id: "pad-grist-up" })], captured);
    const result = await ticker.tick();
    // The joint pad/grist row fans out to both stateful facets.
    expect(result.decisions.map((d) => d.facet)).toEqual(["pad", "grist"]);
    expect(result.delivered).toHaveLength(2);
    expect(captured.journal.map((e) => e.facet)).toEqual(["pad", "grist"]);
    expect(captured.pages.map((p) => p.title)).toEqual(["PM stack: pad needs you", "PM stack: grist needs you"]);
    expect(captured.feed).toHaveLength(0);
  });

  test("a lone non-user-facing facet at detection routes to the orchestrator: journal + feed, no push", async () => {
    const captured: Captured = { journal: [], feed: [], pages: [] };
    const ticker = tickerFor(() => [summary()], captured);
    const result = await ticker.tick();
    expect(result.decisions).toHaveLength(1);
    expect(result.decisions[0]?.route).toBe("needs-orchestrator");
    expect(captured.journal).toHaveLength(1);
    expect(captured.feed).toHaveLength(1);
    expect(captured.feed[0]?.facet).toBe("projector");
    expect(captured.pages).toHaveLength(0);
  });

  test("an unchanged decision is delivered once, not once per tick", async () => {
    const captured: Captured = { journal: [], feed: [], pages: [] };
    const ticker = tickerFor(() => [summary({ id: "pad-grist-up" })], captured);
    await ticker.tick();
    const second = await ticker.tick();
    expect(second.decisions).toHaveLength(2);
    expect(second.delivered).toHaveLength(0);
    expect(captured.pages).toHaveLength(2);
    expect(captured.journal).toHaveLength(2);
  });

  test("a changed decision re-delivers: a second bad facet tightens the first to a page", async () => {
    const captured: Captured = { journal: [], feed: [], pages: [] };
    let current = [summary()];
    const ticker = tickerFor(() => current, captured);
    await ticker.tick();
    expect(captured.pages).toHaveLength(0);
    current = [summary(), summary({ id: "quota-age" })];
    const second = await ticker.tick();
    // Correlated failure (§4): both facets now page.
    expect(second.delivered.map((d) => d.entry.facet).sort()).toEqual(["projector", "quota"]);
    expect(captured.pages).toHaveLength(2);
  });

  test("recovery clears the delivered mark, so a recurrence escalates fresh", async () => {
    const captured: Captured = { journal: [], feed: [], pages: [] };
    let current = [summary({ id: "pad-grist-up" })];
    const ticker = tickerFor(() => current, captured);
    await ticker.tick();
    current = [summary({ id: "pad-grist-up", latestBad: false })];
    const recovered = await ticker.tick();
    expect(recovered.decisions).toHaveLength(0);
    current = [summary({ id: "pad-grist-up" })];
    const recurred = await ticker.tick();
    expect(recurred.delivered).toHaveLength(2);
    expect(captured.pages).toHaveLength(4);
  });

  test("a failed delivery is retried on the next tick, not marked delivered", async () => {
    const captured: Captured = { journal: [], feed: [], pages: [] };
    const ticker = tickerFor(() => [summary({ id: "pad-grist-up" })], captured, {
      ok: false,
      error: "push endpoint down",
    });
    const first = await ticker.tick();
    expect(first.delivered.map((d) => d.error)).toEqual(["push: push endpoint down", "push: push endpoint down"]);
    const second = await ticker.tick();
    expect(second.delivered).toHaveLength(2);
    expect(captured.pages).toHaveLength(4);
  });

  test("no bad facets: no decisions, no deliveries", async () => {
    const captured: Captured = { journal: [], feed: [], pages: [] };
    const ticker = tickerFor(() => [summary({ latestBad: false })], captured);
    const result = await ticker.tick();
    expect(result.decisions).toHaveLength(0);
    expect(result.delivered).toHaveLength(0);
    expect(captured.journal).toHaveLength(0);
  });
});

describe("escalation journal + feed shapes (#9991, design §3)", () => {
  const entry: EscalationJournalEntry = {
    ts: "2026-11-22T12:00:00.000Z",
    facet: "pad",
    route: "page",
    reason: "stateful service is down",
    maxRemediationAttempts: null,
  };

  test("the journal line rides the watchdog journal under an escalation facet", async () => {
    const line = escalationJournalLine(entry);
    expect(line.facet).toBe("escalation:pad");
    expect(line.atMs).toBe(NOW);
    expect(line.escalation).toEqual(entry);
    // Round-trip through the real journal: the line parses back and
    // groups under its own facet, never under a SLO definition id.
    const dir = await mkdtemp(join(tmpdir(), "escalation-journal-"));
    try {
      const journal = new SloJournal(join(dir, "slo-journal.jsonl"));
      await journal.append([line]);
      const lines = await journal.readAll();
      expect(lines).toHaveLength(1);
      expect(lines[0]?.escalation).toEqual(entry);
      const byFacet = groupSamplesByFacet(lines);
      expect([...byFacet.keys()]).toEqual(["escalation:pad"]);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });

  test("the feed delivery is a watchdog event with a stable per-decision id", () => {
    const delivery = escalationDelivery(entry);
    expect(delivery.source).toBe("watchdog");
    expect(delivery.kind).toBe("watchdog-event");
    expect(delivery.ref).toBe("escalation:pad");
    expect(delivery.at).toBe(entry.ts);
    expect(delivery.summary).toContain("page Drew");
    expect(escalationDelivery(entry).sourceId).toBe(delivery.sourceId);
  });

  test("the decision key changes when the route or the attempt cap changes", () => {
    const base = {
      facet: "tunnel" as const,
      facetClass: "user-facing" as const,
      route: "needs-orchestrator" as const,
      reason: "r",
      push: false,
      maxRemediationAttempts: null,
    };
    expect(escalationDecisionKey(base)).not.toBe(escalationDecisionKey({ ...base, route: "page", push: true }));
    expect(escalationDecisionKey(base)).not.toBe(escalationDecisionKey({ ...base, maxRemediationAttempts: 1 }));
    expect(escalationDecisionKey(base)).toBe(escalationDecisionKey({ ...base }));
  });
});
