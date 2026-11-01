import { describe, expect, test } from "bun:test";
import {
  computeCoverage,
  diffCoverage,
  isRepoCensus,
  nextCoverageSnapshot,
  type CoverageReport,
  type CoverageSnapshotFile,
  type RepoCensus,
} from "../src/coverage.ts";
import type { ProjectPoolItem } from "../src/model.ts";

function poolItem(partial: Partial<ProjectPoolItem> & { repo: string }): ProjectPoolItem {
  return {
    id: "PVTI_x",
    url: "https://example.invalid/p",
    projectUrl: "https://example.invalid/proj",
    ref: `${partial.repo}#1`,
    number: 1,
    issueUrl: "https://example.invalid/i",
    title: "t",
    state: "open",
    contentKind: "issue",
    status: "Todo",
    area: "Fleet",
    horizon: null,
    kind: null,
    ...partial,
  } as ProjectPoolItem;
}

const NOW = new Date("2026-11-01T12:00:00Z");

const census: RepoCensus[] = [
  { repo: "developing-today/code", openIssues: 1122, gateNow: 22, gateNext: 22 },
  { repo: "dezren39/nix", openIssues: 24, gateNow: 0, gateNext: 0 },
];

const pool: ProjectPoolItem[] = [
  poolItem({ repo: "developing-today/code" }),
  poolItem({ repo: "developing-today/code", status: null }),
  poolItem({ repo: "developing-today/code", status: null, area: null }),
  poolItem({ repo: "dezren39/nix", area: null }),
  poolItem({ repo: "developing-today/opencode" }),
];

describe("isRepoCensus", () => {
  test("accepts a well-formed row, rejects malformed ones", () => {
    expect(isRepoCensus({ repo: "a/b", openIssues: 1, gateNow: 0, gateNext: 0 })).toBe(true);
    expect(isRepoCensus({ repo: "a/b", openIssues: 1.5, gateNow: 0, gateNext: 0 })).toBe(false);
    expect(isRepoCensus({ repo: "ab", openIssues: 1, gateNow: 0, gateNext: 0 })).toBe(false);
    expect(isRepoCensus(null)).toBe(false);
  });
});

describe("computeCoverage", () => {
  const report = computeCoverage(census, pool, NOW);

  test("carries the census backlog counts per repo", () => {
    const code = report.repos.find((r) => r.repo === "developing-today/code");
    expect(code?.openIssues).toBe(1122);
    expect(code?.gateNow).toBe(22);
    expect(code?.gateNext).toBe(22);
  });

  test("counts pool items and unset Status/Area per repo", () => {
    const code = report.repos.find((r) => r.repo === "developing-today/code");
    const nix = report.repos.find((r) => r.repo === "dezren39/nix");
    expect(code?.poolItems).toBe(3);
    expect(code?.poolUnsetStatus).toBe(2);
    expect(code?.poolUnsetArea).toBe(1);
    expect(nix?.poolItems).toBe(1);
    expect(nix?.poolUnsetArea).toBe(1);
  });

  test("a pool repo missing from the census keeps null backlog fields, never fake zeros", () => {
    const oc = report.repos.find((r) => r.repo === "developing-today/opencode");
    expect(oc?.poolItems).toBe(1);
    expect(oc?.openIssues).toBeNull();
    expect(oc?.gateNow).toBeNull();
  });

  test("no census at all: pool counts still compute, backlog stays null", () => {
    const bare = computeCoverage(null, pool, NOW);
    expect(bare.repos.every((r) => r.openIssues === null)).toBe(true);
    expect(bare.repos.find((r) => r.repo === "developing-today/code")?.poolItems).toBe(3);
  });

  test("repos are sorted and stamped", () => {
    expect(report.repos.map((r) => r.repo)).toEqual(["developing-today/code", "developing-today/opencode", "dezren39/nix"]);
    expect(report.generatedAt).toBe("2026-11-01T12:00:00.000Z");
  });
});

describe("snapshot rotation + delta", () => {
  const mk = (at: string, codeOpen: number | null): CoverageReport => ({
    generatedAt: at,
    repos: [
      { repo: "developing-today/code", openIssues: codeOpen, gateNow: 1, gateNext: 2, poolItems: 10, poolUnsetStatus: 3, poolUnsetArea: 4 },
    ],
  });

  test("first observation stores with no baseline and yields no delta", () => {
    const snap = nextCoverageSnapshot(null, mk("2026-11-01T00:00:00Z", 100), NOW);
    expect(snap.baseline).toBeNull();
    expect(diffCoverage(snap.current, snap.baseline).size).toBe(0);
  });

  test("within the week the baseline is kept and current refreshes", () => {
    const stored: CoverageSnapshotFile = { baseline: null, current: mk("2026-10-30T00:00:00Z", 90) };
    const snap = nextCoverageSnapshot(stored, mk("2026-11-01T00:00:00Z", 100), NOW);
    expect(snap.baseline).toBeNull();
    expect(snap.current.repos[0]?.openIssues).toBe(100);
  });

  test("at a week old the stored current becomes the baseline", () => {
    const stored: CoverageSnapshotFile = { baseline: null, current: mk("2026-10-24T00:00:00Z", 90) };
    const snap = nextCoverageSnapshot(stored, mk("2026-11-01T00:00:00Z", 100), NOW);
    expect(snap.baseline?.generatedAt).toBe("2026-10-24T00:00:00Z");
    const delta = diffCoverage(snap.current, snap.baseline);
    expect(delta.get("developing-today/code")?.openIssues).toBe(10);
  });

  test("delta is null where either side lacks a census", () => {
    const delta = diffCoverage(mk("2026-11-01T00:00:00Z", null), mk("2026-10-24T00:00:00Z", 90));
    expect(delta.get("developing-today/code")?.openIssues).toBeNull();
    expect(delta.get("developing-today/code")?.poolItems).toBe(0);
  });
});
