import { describe, expect, test } from "bun:test";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadConfig } from "../src/config.ts";
import type { ProjectPoolItem, SaasService } from "../src/model.ts";
import { ConflictRulingsStore, RULING_CLASS_ID, validateRulingInput } from "../src/conflict-rulings.ts";
import {
  conflictInbox,
  effectiveProjectDisagreement,
  resolveTrustedProjectValue,
  rulingForClass,
} from "../src/conflicts.ts";
import { buildBriefs, buildItemBrief } from "../src/brief.ts";
import { buildHealth } from "../src/health.ts";
import { buildQueueView, reconciledInProgress } from "../src/queue-view.ts";
import { projectsBodyHtml, renderConflicts } from "../src/render.ts";

// #1282 slice 2: per-class trust rulings for Project-vs-derived. Seeded
// fixtures on purpose — a ruling over an agreeing projection proves
// nothing, so the pool below disagrees (Project "In Progress" vs derived
// "Todo") and the ruling's effect on the inbox, renders, briefs, queue,
// and health count is asserted from the same fixture.

const NOW = Date.parse("2026-11-03T12:00:00Z");

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
    conflicts: [],
    reads: [{ at: "2026-11-01T12:00:00Z", text: "Dashboard says 3 of 5 databases" }],
    sources: { quotaViewSection: "### Turso", trackerRow: "Turso", baselineDoc: "baseline" },
    ...partial,
  };
}

const clashPool = () => [item({ number: 1282, status: "In Progress", updatedAt: "2026-10-27T12:00:00Z" })];

const ruling = (trusted: "project" | "derived" = "derived") => ({
  classId: RULING_CLASS_ID,
  trusted,
  createdAt: "2026-11-02T12:00:00Z",
  createdBy: "drew:test",
  reason: "Project Status is stale; the derived state is the truth",
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

describe("validateRulingInput (#1282 slice 2)", () => {
  test("accepts the project-vs-derived ruling in both directions", () => {
    for (const trusted of ["project", "derived"] as const) {
      const parsed = validateRulingInput({ classId: "project-vs-derived", trusted, reason: "why" });
      expect(parsed.ok).toBe(true);
    }
  });

  test("rejects anything outside the slice-2 class", () => {
    expect(validateRulingInput({ classId: "kaneo-vs-github", trusted: "derived" }).ok).toBe(false);
    expect(validateRulingInput({ classId: "project-vs-derived", trusted: "kaneo" }).ok).toBe(false);
    expect(validateRulingInput({ classId: "project-vs-derived", trusted: "derived", reason: "x".repeat(501) }).ok).toBe(
      false,
    );
    expect(validateRulingInput(null).ok).toBe(false);
  });
});

describe("ConflictRulingsStore (#1282 slice 2)", () => {
  test("set / getForClass / revoke round-trips as data, one ruling per class", async () => {
    const file = join(mkdtempSync(join(tmpdir(), "rulings-")), "conflict-rulings.json");
    const store = new ConflictRulingsStore(file);
    expect(await store.getForClass("project-vs-derived")).toBeNull();
    const first = await store.set({ classId: RULING_CLASS_ID, trusted: "project", reason: null }, "drew:test");
    expect(first.trusted).toBe("project");
    // A second ruling for the same class replaces, never duplicates.
    await store.set({ classId: RULING_CLASS_ID, trusted: "derived", reason: "new evidence" }, "drew:test");
    expect((await store.all()).filter((r) => r.classId === "project-vs-derived")).toHaveLength(1);
    expect((await store.getForClass("project-vs-derived"))?.trusted).toBe("derived");
    expect(await store.revoke("project-vs-derived")).toBe(true);
    expect(await store.revoke("project-vs-derived")).toBe(false);
    expect(await store.getForClass("project-vs-derived")).toBeNull();
    // A fresh store over the same file starts empty after revocation.
    expect(await new ConflictRulingsStore(file).getForClass("project-vs-derived")).toBeNull();
  });
});

describe("inbox suppression (#1282 slice 2)", () => {
  test("a standing ruling suppresses the class but keeps it auditable", () => {
    const live = conflictInbox({
      pool: clashPool(),
      services: [service({ conflicts: ["Summary row says $25/mo, card says $20/mo"] })],
      now: NOW,
    });
    expect(live.total).toBe(2);
    const ruled = conflictInbox({
      pool: clashPool(),
      services: [service({ conflicts: ["Summary row says $25/mo, card says $20/mo"] })],
      now: NOW,
      rulings: [ruling()],
    });
    expect(ruled.total).toBe(1);
    expect(ruled.entries.map((e) => e.classId)).toEqual(["quota-records"]);
    expect(ruled.suppressed).toHaveLength(1);
    expect(ruled.suppressed[0].classId).toBe("project-vs-derived");
    // The suppressed entry keeps both values — suppression is not deletion.
    expect(ruled.suppressed[0].values).toEqual({ a: "In Progress", b: "Todo" });
    expect(ruled.byClass.find((c) => c.classId === "project-vs-derived")?.count).toBe(0);
    expect(ruled.suppressedByClass.find((c) => c.classId === "project-vs-derived")?.count).toBe(1);
    expect(ruled.rulings).toHaveLength(1);
    expect(rulingForClass(ruled.rulings, "project-vs-derived")?.trusted).toBe("derived");
  });

  test("revoking re-surfaces the class's current disagreements", () => {
    const pool = clashPool();
    const suppressed = conflictInbox({ pool, now: NOW, rulings: [ruling()] });
    expect(suppressed.total).toBe(0);
    // Revocation is just the absence of the ruling: detection re-reports.
    expect(conflictInbox({ pool, now: NOW }).total).toBe(1);
  });

  test("a ruling for another class never suppresses project-vs-derived", () => {
    const inbox = conflictInbox({
      pool: clashPool(),
      now: NOW,
      rulings: [{ ...ruling(), classId: "quota-records" as never }],
    });
    expect(inbox.total).toBe(1);
    expect(inbox.suppressed).toEqual([]);
  });
});

describe("trusted-value helpers (#1282 slice 2)", () => {
  test("effectiveProjectDisagreement is null while a ruling stands", () => {
    const [p] = clashPool();
    expect(effectiveProjectDisagreement(p, null, undefined, null)).not.toBeNull();
    expect(effectiveProjectDisagreement(p, null, undefined, ruling())).toBeNull();
  });

  test("resolveTrustedProjectValue applies the trusted side", () => {
    const [p] = clashPool();
    expect(resolveTrustedProjectValue(p, null, undefined, ruling("derived"))).toBe("Todo");
    expect(resolveTrustedProjectValue(p, null, undefined, ruling("project"))).toBe("In Progress");
    expect(resolveTrustedProjectValue(p, null, undefined, null)).toBeNull();
  });
});

describe("renders apply the ruling (#1282 slice 2)", () => {
  test("briefs suppress the disagreement while a ruling stands", () => {
    const without = buildBriefs(clashPool(), { orchestration: null });
    expect(without.get("developing-today/code#1282")?.disagreement).not.toBeNull();
    const withRuling = buildBriefs(clashPool(), { orchestration: null, ruling: ruling() });
    expect(withRuling.get("developing-today/code#1282")?.disagreement).toBeNull();
    expect(withRuling.get("developing-today/code#1282")?.state.value).toBe("Todo");
    const projectTrusted = buildItemBrief(clashPool()[0], { orchestration: null, ruling: ruling("project") });
    expect(projectTrusted.state.value).toBe("In Progress");
    expect(projectTrusted.state.text).toContain("applied by a standing");
  });

  test("queue rows suppress the disagreement while a ruling stands", () => {
    const base = { board: [], pool: clashPool(), items: new Map(), orchestration: null };
    expect(reconciledInProgress(base).find((r) => r.ref === "developing-today/code#1282")?.disagreement).not.toBeNull();
    const ruledView = buildQueueView({ ...base, ruling: ruling() });
    const rec = reconciledInProgress({ ...base, ruling: ruling() });
    expect(rec.find((r) => r.ref === "developing-today/code#1282")?.disagreement).toBeNull();
    for (const row of [...ruledView.inProgress, ...ruledView.nextUp]) {
      if (row.ref === "developing-today/code#1282") expect(row.disagreement).toBeNull();
    }
  });

  test("pool cards show ⚠ without a ruling and the trusted value with one", () => {
    const clash = projectsBodyHtml(clashPool(), { orchestration: null });
    expect(clash).toContain("⚠ project: In Progress · derived: Todo");
    const trusted = projectsBodyHtml(clashPool(), { orchestration: null, ruling: ruling("derived") });
    expect(trusted).not.toContain("⚠ project:");
    expect(trusted).toContain("trusts derived");
    expect(trusted).toContain(">Todo</span>");
    expect(trusted).toContain("derived: Todo");
    expect(trusted).not.toContain(">In Progress</span>");
    const projectTrusted = projectsBodyHtml(clashPool(), { orchestration: null, ruling: ruling("project") });
    expect(projectTrusted).toContain("trusts project");
    expect(projectTrusted).toContain(">In Progress</span>");
    expect(projectTrusted).toContain("derived: In Progress");
  });

  test("/conflicts names the ruling, the suppressed count, and the revoke path", () => {
    const html = renderConflicts(
      conflictInbox({ pool: clashPool(), now: NOW, rulings: [ruling()] }),
      PROV,
    );
    expect(html).toContain("Trust rulings");
    expect(html).toContain("trusts <strong>derived</strong>");
    expect(html).toContain("suppressed from this queue");
    expect(html).toContain("/api/conflicts/rulings");
  });
});

describe("health counts the suppressed inbox (#1282 slice 2)", () => {
  test("buildHealth carries the post-ruling total, not the detected one", async () => {
    const inbox = conflictInbox({ pool: clashPool(), now: NOW, rulings: [ruling()] });
    const view = await buildHealth(
      { mode: "fixtures", cfg: loadConfig({ PM_AGG_DEV: "1" }), feed: null, feedFileMtimeMs: null, conflicts: inbox },
      { now: NOW },
    );
    expect(view.conflicts?.total).toBe(0);
  });
});
