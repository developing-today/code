import { describe, expect, test } from "bun:test";
import { loadConfig } from "../src/config.ts";
import type { ProjectPoolItem, SaasService } from "../src/model.ts";
import { CONFLICT_CLASSES, conflictInbox, detectConflicts } from "../src/conflicts.ts";
import { buildHealth } from "../src/health.ts";
import { PAGES } from "../src/pages.ts";
import { renderConflicts, renderHealth } from "../src/render.ts";

// #1282 slice 1: the read-only source-conflict inbox. Seeded disagreement
// fixtures on purpose — a projection over agreeing sources proves nothing,
// so each fixture below states BOTH sides and the timestamp the age is
// measured from. Nothing here asserts a ruling: slice 1 cannot resolve.

const NOW = Date.parse("2026-11-03T12:00:00Z");

/** The per-item builder from test/brief.test.ts, trimmed to this file. */
function item(partial: Partial<ProjectPoolItem> & { number: number }): ProjectPoolItem {
  return {
    id: `PVTI_${partial.number}`,
    url: `https://github.com/orgs/developing-today/projects/4?pane=issue&itemId=${partial.number}`,
    projectUrl: "https://github.com/orgs/developing-today/projects/4",
    ref: `developing-today/code#${partial.number}`,
    repo: "developing-today/code",
    issueUrl: `https://github.com/developing-today/code/issues/${partial.number}`,
    title: `item ${partial.number}`,
    state: "open",
    contentKind: "issue",
    status: null,
    area: null,
    horizon: null,
    kind: "Task",
    ...partial,
  };
}

/** A SaaS service carrying one recorded conflict line and one dated read. */
function service(partial: Partial<SaasService> = {}): SaasService {
  return {
    id: "turso",
    name: "Turso",
    group: "database",
    plan: "Developer $0",
    status: "active",
    metrics: [],
    paygEstimate: null,
    measuredAt: "2026-11-01T12:00:00Z",
    sourceClass: "console",
    sourceNote: "quota-view.md",
    cadenceDays: 7,
    conflicts: [],
    reads: [{ at: "2026-11-01T12:00:00Z", text: "Dashboard says 3 of 5 databases" }],
    sources: { quotaViewSection: "### Turso", trackerRow: "Turso", baselineDoc: "baseline" },
    ...partial,
  };
}

describe("detectConflicts (#1282 slice 1)", () => {
  test("collects a Project-vs-derived disagreement with both values and a dated floor", () => {
    const pool = [item({ number: 1282, status: "In Progress", updatedAt: "2026-10-27T12:00:00Z" })];
    const entries = detectConflicts({ pool, now: NOW });
    expect(entries).toHaveLength(1);
    const [entry] = entries;
    expect(entry.id).toBe("project-vs-derived:developing-today/code#1282");
    expect(entry.classId).toBe("project-vs-derived");
    expect(entry.sources).toEqual({ a: "GitHub Project Status", b: "dashboard-derived state" });
    // Both sides, never a preferred one.
    expect(entry.values).toEqual({ a: "In Progress", b: "Todo" });
    expect(entry.ageDays).toBe(7);
    expect(entry.ageBasis).toBe("project-item-updated-at");
    expect(entry.item.url).toContain("/issues/1282");
  });

  test("sources that agree produce no entry", () => {
    const pool = [item({ number: 43, status: "Todo", updatedAt: "2026-11-02T12:00:00Z" })];
    expect(detectConflicts({ pool, now: NOW })).toEqual([]);
  });

  test("an item with no Project Status cannot disagree", () => {
    const pool = [item({ number: 7, status: null })];
    expect(detectConflicts({ pool, now: NOW })).toEqual([]);
  });

  test("emits one entry per recorded quota conflict line, undated conflicts stay honest", () => {
    const svc = service({
      conflicts: ["Summary row says $25/mo, card says $20/mo", "API read failed at 401"],
    });
    const entries = detectConflicts({ pool: [], services: [svc], now: NOW });
    expect(entries).toHaveLength(2);
    expect(entries.map((e) => e.id)).toEqual(["quota-records:turso:0", "quota-records:turso:1"]);
    expect(entries[0].values).toEqual({
      a: "Summary row says $25/mo, card says $20/mo",
      b: "Dashboard says 3 of 5 databases",
    });
    expect(entries[0].sources.a).toBe("Turso");
    expect(entries[0].ageDays).toBe(2);
    expect(entries[0].ageBasis).toBe("service-measured-at");
    expect(entries[0].href).toBe("/quota/service/turso");

    const undated = service({ measuredAt: null, reads: [], conflicts: ["no dates anywhere"] });
    const [entry] = detectConflicts({ pool: [], services: [undated], now: NOW });
    expect(entry.ageDays).toBeNull();
    expect(entry.ageBasis).toBe("undated");
    expect(entry.values.b).toBe("no dated read on record");
  });

  test("the inbox counts per class and reports the oldest disagreement", () => {
    const inbox = conflictInbox({
      pool: [
        item({ number: 1282, status: "In Progress", updatedAt: "2026-10-27T12:00:00Z" }),
        item({ number: 43, status: "Todo" }),
      ],
      services: [service({ conflicts: ["one line"] })],
      now: NOW,
    });
    expect(inbox.total).toBe(2);
    expect(inbox.oldestAgeDays).toBe(7);
    expect(inbox.byClass).toEqual([
      { classId: "project-vs-derived", label: CONFLICT_CLASSES[0].label, count: 1 },
      { classId: "quota-records", label: CONFLICT_CLASSES[1].label, count: 1 },
    ]);
    // Machine twin: the same projection survives JSON unchanged.
    expect(JSON.parse(JSON.stringify(inbox)).total).toBe(2);
  });

  test("an empty projection is a real empty inbox", () => {
    const inbox = conflictInbox({ pool: [], now: NOW });
    expect(inbox.total).toBe(0);
    expect(inbox.entries).toEqual([]);
    expect(inbox.oldestAgeDays).toBeNull();
  });
});

const PROV = {
  board: "x",
  roadmap: "x",
  items: "x",
  comments: "x",
  quota: "x",
  usage: "x",
  attention: "x",
  orchestration: "x",
  dns: "x",
  registry: "x",
  aa: "x",
};

describe("/conflicts page (#1282 slice 1)", () => {
  test("renders the item, both sources, both values, and the age floor", () => {
    const inbox = conflictInbox({
      pool: [item({ number: 1282, status: "In Progress", updatedAt: "2026-10-27T12:00:00Z" })],
      services: [service({ conflicts: ["Summary row says $25/mo, card says $20/mo"] })],
      now: NOW,
    });
    const html = renderConflicts(inbox, PROV);
    expect(html).toContain("2026-10-30T12:00:00Z".slice(0, 0) + "item 1282");
    expect(html).toContain("In Progress");
    expect(html).toContain("GitHub Project Status");
    expect(html).toContain("dashboard-derived state");
    expect(html).toContain("at least <strong>7</strong> day");
    expect(html).toContain("Summary row says $25/mo, card says $20/mo");
    expect(html).toContain("/api/conflicts.json");
    expect(html).toContain("Project Status vs derived state");
  });

  test("an empty inbox says so rather than looking unread", () => {
    const html = renderConflicts(conflictInbox({ pool: [], now: NOW }), PROV);
    expect(html).toContain("No live source disagreements");
  });

  test("the manifest registers the page and its machine twin", () => {
    const page = PAGES.find((p) => p.id === "conflicts");
    expect(page).toEqual(
      expect.objectContaining({ route: "/conflicts", api: "/api/conflicts.json", group: "work", nav: "dropdown" }),
    );
  });
});

describe("health surface counts the inbox (#1282 slice 1)", () => {
  test("buildHealth carries the count and the page renders it", async () => {
    const inbox = conflictInbox({
      pool: [item({ number: 1282, status: "In Progress", updatedAt: "2026-10-27T12:00:00Z" })],
      services: [service({ conflicts: ["one line"] })],
      now: NOW,
    });
    const view = await buildHealth(
      { mode: "fixtures", cfg: loadConfig({ PM_AGG_DEV: "1" }), feed: null, feedFileMtimeMs: null, conflicts: inbox },
      { now: NOW },
    );
    expect(view.conflicts?.total).toBe(2);
    expect(view.conflicts?.oldestAgeDays).toBe(7);
    const html = renderHealth(view, PROV as Parameters<typeof renderHealth>[1]);
    expect(html).toContain('href="/conflicts"');
    expect(html).toContain("2 live source conflicts");
    expect(html).toContain("oldest at least <strong>7</strong>");
  });

  test("a health view assembled without the inbox says so instead of inventing zero", async () => {
    const view = await buildHealth(
      { mode: "fixtures", cfg: loadConfig({ PM_AGG_DEV: "1" }), feed: null, feedFileMtimeMs: null },
      { now: NOW },
    );
    expect(view.conflicts).toBeNull();
    const html = renderHealth(view, PROV as Parameters<typeof renderHealth>[1]);
    expect(html).toContain("not assembled for this health view");
  });
});
