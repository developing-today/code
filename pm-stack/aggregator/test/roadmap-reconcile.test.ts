import { describe, expect, test } from "bun:test";
import type { ProjectPoolItem, RoadmapRow } from "../src/model.ts";
import { renderRoadmap } from "../src/render.ts";
import { normalizeTitle, reconcileRoadmap, roadmapDriftReport, unavailableRoadmapReconciliation } from "../src/roadmap-reconcile.ts";

// #1144: Grist roadmap rows ↔ Project Kind=Epic reconciliation.

const row = (id: number, title: string, refs: string[] = []): RoadmapRow => ({
  id,
  subject: `grist:doc/roadmap/${id}`,
  title,
  area: "id",
  status: "In Progress",
  horizon: "Quarter",
  source: "grist",
  notes: "",
  links: refs.map((ref) => ({ ref: ref as RoadmapRow["links"][number]["ref"], relation: "tracks" as const })),
});

const epic = (ref: string, title: string, kind: string | null = "Epic"): ProjectPoolItem => ({
  id: `PVTI_${ref}`,
  url: "https://github.com/orgs/developing-today/projects/4",
  projectUrl: "https://github.com/orgs/developing-today/projects/4",
  ref: ref as ProjectPoolItem["ref"],
  repo: ref.split("#")[0],
  number: Number(ref.split("#")[1]),
  issueUrl: `https://github.com/${ref.replace("#", "/issues/")}`,
  title,
  state: "open",
  contentKind: "issue",
  status: "Todo",
  area: "id",
  horizon: "Quarter",
  kind,
});

describe("reconcileRoadmap (#1144)", () => {
  test("matches by linked ref first, then by normalized title", () => {
    const rows = [row(1, "Renamed beyond recognition", ["dezren39/nix#508"]), row(2, "Basement Cluster!!")];
    const pool = [epic("dezren39/nix#508", "EPIC: Basement cluster"), epic("developing-today/code#900", "basement cluster")];
    const rec = reconcileRoadmap(rows, pool);
    expect(rec.matched.length).toBe(2);
    expect(rec.matched[0].via).toBe("ref");
    expect(rec.matched[1].via).toBe("title");
    expect(rec.drift).toBe(0);
  });

  test("reports unmatched rows and unmatched epics in both directions", () => {
    const rows = [row(1, "Only in Grist"), row(2, "Shared", ["a/b#1"])];
    const pool = [epic("a/b#1", "Shared"), epic("a/b#2", "Only in Project")];
    const rec = reconcileRoadmap(rows, pool);
    expect(rec.unmatchedRows.map((r) => r.title)).toEqual(["Only in Grist"]);
    expect(rec.unmatchedEpics.map((e) => e.ref)).toEqual(["a/b#2"]);
    expect(rec.drift).toBe(2);
    const report = roadmapDriftReport(rec);
    expect(report.matched).toBe(1);
    expect(report.drift).toBe(2);
    expect(report.unmatchedEpics[0].ref).toBe("a/b#2");
  });

  test("zero drift when both sides agree; non-Epic pool items are ignored", () => {
    const rows = [row(1, "Epic A", ["a/b#1"])];
    const pool = [epic("a/b#1", "Epic A"), epic("a/b#2", "A mere task", "Task"), epic("a/b#3", "No kind", null)];
    const rec = reconcileRoadmap(rows, pool);
    expect(rec.drift).toBe(0);
    expect(rec.unmatchedEpics.length).toBe(0);
  });

  test("one epic can never match two rows", () => {
    const rows = [row(1, "Same Title"), row(2, "Same Title")];
    const pool = [epic("a/b#1", "Same Title")];
    const rec = reconcileRoadmap(rows, pool);
    expect(rec.matched.length).toBe(1);
    expect(rec.unmatchedRows.length).toBe(1);
  });

  test("normalizeTitle folds case, punctuation, and whitespace", () => {
    expect(normalizeTitle("  EPIC:  Basement—Cluster! ")).toBe("epic basement cluster");
  });
});

describe("renderRoadmap drift indicator (#1144)", () => {
  const PROV = {
    board: "x", roadmap: "x", items: "x", comments: "x", quota: "x", usage: "x",
    attention: "x", orchestration: "x", dns: "x", registry: "x", aa: "x",
  } as Parameters<typeof renderRoadmap>[2];

  test("drift > 0 flags the page; zero drift stays quiet", () => {
    const rows = [row(1, "Only in Grist")];
    const drifted = reconcileRoadmap(rows, []);
    const html = renderRoadmap(rows, new Map(), PROV, "quarter", undefined, undefined, drifted);
    expect(html).toContain("Roadmap drift");
    expect(html).toContain("1 Grist row");
    const clean = reconcileRoadmap([], []);
    const html2 = renderRoadmap([], new Map(), PROV, "quarter", undefined, undefined, clean);
    expect(html2).not.toContain("Roadmap drift");
  });

  test("unavailable reconciliation shows a notice, never a drift flag (Codex P2 on PR 2428)", () => {
    const rec = unavailableRoadmapReconciliation("Grist roadmap read unavailable (fixtures)");
    const html = renderRoadmap([row(1, "Only in Grist")], new Map(), PROV, "quarter", undefined, undefined, rec);
    expect(html).toContain("Roadmap reconciliation unavailable");
    expect(html).not.toContain("Roadmap drift:");
  });
});

describe("unavailable reconciliation report (Codex P2 on PR 2428)", () => {
  test("report asserts no drift and names the reason when a source read failed", () => {
    const rec = unavailableRoadmapReconciliation("fleet feed unreadable");
    expect(rec.availability).toBe("unavailable");
    const report = roadmapDriftReport(rec);
    expect(report.availability).toBe("unavailable");
    expect(report.drift).toBeNull();
    expect(report.unmatchedRows).toEqual([]);
    expect(report.unmatchedEpics).toEqual([]);
    const available = roadmapDriftReport(reconcileRoadmap([], []));
    expect(available.availability).toBe("available");
    expect(available.drift).toBe(0);
  });
});
