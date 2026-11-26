// #15019 (C6 slice 2): one evidence-backed incident page.
// Oracles: the C6 design (doc/2026-11-13T15-12-00Z_design_reporting-
// architecture-event-driven/c6-report-pages-design.md) — §1.1 page-data
// envelope, §6.1 incident section order, §6.2 page threshold — driven
// by a fixture shaped from a REAL alerts-ledger incident (verbatim
// export, see the fixture's provenance field): the OpenCode working
// outage of 2026-11-23, healthHook incident dfb9f3e2. The Claude auth
// outage candidate has no ledger record, so the issue's fallback rule
// applies (documented on #15019).
import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import type { Incident, IncidentEvent } from "../src/alerts.ts";
import {
  INCIDENT_SECTION_IDS,
  buildIncidentEnvelope,
  incidentPageThreshold,
  renderIncidentPage,
  validatePageEnvelope,
  type C6PageEnvelope,
} from "../src/incident-page.ts";

const fixture = JSON.parse(
  readFileSync(join(import.meta.dir, "fixtures", "incident-15019-opencode-working.json"), "utf8"),
) as {
  incident: Incident;
  events: IncidentEvent[];
  related: Incident[];
  provenance: string;
};

const GENERATED_AT = "2026-11-25T12:00:00Z";
const envelope: C6PageEnvelope = buildIncidentEnvelope({
  incident: fixture.incident,
  events: fixture.events,
  related: fixture.related,
  generatedAt: GENERATED_AT,
});
const section = (id: string) => envelope.sections.find((s) => s.id === id)!;

describe("threshold (oracle: C6 design §6.2)", () => {
  test("the fixture incident qualifies under (a) routing tier >= 30 min and (d) recurrence", () => {
    const t = incidentPageThreshold(fixture.incident, fixture.events, fixture.related);
    expect(t.qualifies).toBe(true);
    expect(t.clauses).toContain("a");
    expect(t.clauses).toContain("d");
  });

  test("a short isolated incident does not earn a page", () => {
    const blip: Incident = {
      ...fixture.incident,
      id: "00000000-0000-4000-8000-000000000000",
      incident_key: "health-hook:some-other-target:00000000-0000-4000-8000-000000000001",
      first_seen_at: "2026-11-23T05:26:51.026Z",
      last_seen_at: "2026-11-23T05:27:17.480Z",
      closed_at: "2026-11-23T05:27:18.409Z",
      occurrences: 1,
    };
    const t = incidentPageThreshold(blip, [], []);
    expect(t.qualifies).toBe(false);
    expect(t.clauses).toEqual([]);
  });
});

describe("envelope contract (oracle: C6 design §1.1)", () => {
  test("schema, identity, template and page class follow the envelope shape", () => {
    expect(envelope.schema).toBe("c6.page-data/v1");
    expect(envelope.page_class).toBe("incident");
    // Page identity joins on healthHook.incidentId (design §6.1), not
    // the alerts-ledger row id.
    expect(envelope.page_id).toBe("incident:opencode-working:dfb9f3e2-3af7-4bce-a790-7acd2271b9ca");
    expect(envelope.template).toEqual({ id: "c6-incident", version: 1 });
    expect(envelope.record_id).toMatch(/^c6page:[0-9a-f]{64}$/);
    expect(envelope.content_version).toMatch(/^sha256:[0-9a-f]{64}$/);
    expect(envelope.as_of).toBe("2026-11-23T04:27:43.698Z");
    expect(envelope.generated_at).toBe(GENERATED_AT);
  });

  test("sources carry the ledger cursor and the healthHook incident id", () => {
    const ledger = envelope.sources.find((s) => s.id === "alerts-ledger");
    expect(ledger?.cursor).toBe("480");
    expect(ledger?.observed_at).toBe("2026-11-23T04:27:43.698Z");
    const hook = envelope.sources.find((s) => s.id === "health-hook:opencode-working");
    expect(hook?.cursor).toBe("dfb9f3e2-3af7-4bce-a790-7acd2271b9ca");
  });

  test("freshness uses the closed-incident budget (24h) and ages honestly", () => {
    expect(envelope.freshness.budget_seconds).toBe(86_400);
    expect(envelope.freshness.last_source_event_at).toBe("2026-11-23T04:27:43.698Z");
    // Generated >24h after the last source event: stale, with the
    // closed-incident reason — never silently fresh.
    expect(envelope.freshness.state).toBe("stale");
    expect(envelope.freshness.reason).toContain("closed");
    const fresh = buildIncidentEnvelope({
      incident: fixture.incident,
      events: fixture.events,
      related: fixture.related,
      generatedAt: "2026-11-23T05:00:00Z",
    });
    expect(fresh.freshness.state).toBe("fresh");
    expect(fresh.freshness.reason).toBeNull();
  });

  test("content hash and record id are immutable for the same inputs; generated_at is transport-only", () => {
    const again = buildIncidentEnvelope({
      incident: fixture.incident,
      events: fixture.events,
      related: fixture.related,
      generatedAt: "2026-11-26T00:00:00Z",
    });
    expect(again.content_version).toBe(envelope.content_version);
    expect(again.record_id).toBe(envelope.record_id);
    const changed = buildIncidentEnvelope({
      incident: { ...fixture.incident, occurrences: 13 },
      events: fixture.events,
      related: fixture.related,
      generatedAt: GENERATED_AT,
    });
    expect(changed.content_version).not.toBe(envelope.content_version);
  });

  test("validator accepts the built envelope and rejects unknown major schema / missing sections", () => {
    expect(validatePageEnvelope(envelope)).toEqual({ ok: true });
    expect(validatePageEnvelope({ ...envelope, schema: "c6.page-data/v2" }).ok).toBe(false);
    expect(validatePageEnvelope({ ...envelope, sections: envelope.sections.slice(1) }).ok).toBe(false);
    expect(validatePageEnvelope({ ...envelope, sections: [...envelope.sections].reverse() }).ok).toBe(false);
  });
});

describe("section order and completeness (oracle: C6 design §6.1)", () => {
  test("exactly the eight template sections, in template order, with stable ids", () => {
    expect(envelope.sections.map((s) => s.id)).toEqual([...INCIDENT_SECTION_IDS]);
    for (const s of envelope.sections) {
      expect(s.title.length).toBeGreaterThan(0);
      expect(["facts", "narrative", "chart", "questions", "comments"]).toContain(s.kind);
    }
  });

  test("current state carries the real impact interval and delivery facts", () => {
    const data = section("current-state").data as Record<string, unknown>;
    expect(data.status).toBe("closed");
    expect(data.affected_service).toBe("OpenCode working");
    expect(data.impact_interval).toEqual({
      start: "2026-11-23T02:27:07.751Z",
      end: "2026-11-23T04:27:43.078Z",
      duration_seconds: 7235,
    });
    expect(data.occurrences).toBe(12);
    // delivered_at is null in the ledger: stated, not smoothed over.
    expect(data.push_delivered_at).toBeNull();
  });

  test("story is assembled only from recorded timestamps", () => {
    const data = section("story").data as Record<string, unknown>;
    expect(data.start).toBe("2026-11-23T02:26:43.496389Z");
    expect(data.detection).toBe("2026-11-23T02:27:09.899Z");
    expect(data.resolution).toBe("2026-11-23T04:27:43.698Z");
    // No ack/escalation was recorded for this incident.
    expect(data.acknowledged_at).toBeNull();
  });

  test("timeline lists the ledger events; attempts record the evidenced restart and no invented failures", () => {
    const data = section("timeline-attempts").data as {
      timeline: Array<{ at: string; kind: string }>;
      attempts: Array<{ outcome: string }>;
      no_failed_attempts_recorded: boolean;
    };
    expect(data.timeline.map((t) => t.kind)).toEqual(["opened", "closed"]);
    expect(data.timeline[0].at).toBe("2026-11-23T02:27:09.899Z");
    expect(data.attempts).toHaveLength(1);
    expect(data.attempts[0].outcome).toBe("succeeded");
    expect(data.no_failed_attempts_recorded).toBe(true);
  });

  test("findings carry source/time labels and the session-liveness series", () => {
    const data = section("findings").data as {
      series: Array<{ label: string; at: string; sessions_last_hour: number }>;
      evidence: Array<{ source: string; at: string }>;
    };
    expect(data.series).toEqual([
      { label: "At detection (down record)", at: "2026-11-23T02:26:43.496389Z", sessions_last_hour: 72 },
      { label: "At recovery (up record)", at: "2026-11-23T04:27:23.185572Z", sessions_last_hour: 75 },
    ]);
    expect(data.evidence.length).toBeGreaterThanOrEqual(2);
    for (const e of data.evidence) {
      expect(e.source).toContain("alerts ledger event");
      expect(e.at.length).toBeGreaterThan(0);
    }
  });

  test("related lists the real recurrence series for the same target", () => {
    const data = section("related").data as { incidents: Array<{ ledger_id: string }> };
    expect(data.incidents).toHaveLength(13);
    expect(data.incidents.map((i) => i.ledger_id)).toContain("dd65acad-c68e-4aba-96c7-03957993ec9d");
  });

  test("root cause is explicitly provisional — the records do not name what stopped the unit", () => {
    const data = section("root-cause").data as Record<string, unknown>;
    expect(data.status).toBe("provisional");
    expect(String(data.summary)).toContain("SIGTERM");
    expect(String(data.summary)).toContain("do not identify");
  });

  test("remediation horizons link real issues and mark unknowns explicitly", () => {
    const data = section("remediation").data as {
      horizons: Array<{ horizon: string; owner: string | null; issue: string | null }>;
    };
    expect(data.horizons.map((h) => h.horizon)).toEqual(["short", "medium", "long"]);
    const long = data.horizons.find((h) => h.horizon === "long")!;
    expect(long.owner).toBeNull();
    expect(long.issue).toBeNull();
  });

  test("links join the ledger card and the owning issues", () => {
    const hrefs = envelope.links.map((l) => `${l.rel}:${l.href}`);
    expect(hrefs).toContain("incident:/alerts#incident-1de30604-fb7d-493a-aa8c-d2ca15765e80");
    expect(hrefs).toContain("issue:https://github.com/developing-today/code/issues/15019");
    expect(hrefs).toContain("issue:https://github.com/developing-today/code/issues/9574");
  });
});

describe("rendering (oracle: C6 design §2 mobile-first)", () => {
  const html = renderIncidentPage(envelope);

  test("sections render in template order under their stable ids", () => {
    const positions = INCIDENT_SECTION_IDS.map((id) => html.indexOf(`id="${id}"`));
    for (const p of positions) expect(p).toBeGreaterThan(-1);
    expect([...positions].sort((a, b) => a - b)).toEqual(positions);
  });

  test("title and freshness come first in reading order", () => {
    expect(html.indexOf("<h1")).toBeLessThan(html.indexOf('id="current-state"'));
    expect(html).toContain("As of 2026-11-23T04:27:43.698Z");
    expect(html).toContain("stale");
  });

  test("the graph has a text summary and a tabular alternative with caption and scoped headers", () => {
    expect(html).toContain('role="img"');
    expect(html).toContain("Sessions in the last hour: 72 at detection");
    expect(html).toContain("<caption>");
    expect(html).toContain('scope="col"');
  });

  test("provisional root cause and unknown owner are visible, not hidden", () => {
    expect(html).toContain("Provisional");
    expect(html).toContain("owner unknown");
  });

  test("headings descend h1 -> h2 with no skips, and every link is a real anchor", () => {
    expect(html.match(/<h1/g)).toHaveLength(1);
    expect(html).not.toContain("<h3");
    for (const m of html.matchAll(/<a [^>]*>/g)) expect(m[0]).toContain("href=");
  });
});
