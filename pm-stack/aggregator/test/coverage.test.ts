import { describe, expect, test } from "bun:test";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  computeCoverage,
  CoverageSnapshotStore,
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

describe("computeCoverage — pool unavailable (PR #3309 Codex)", () => {
  test("a null pool renders pool fields as null, never fake zeros, while census counts stay real", () => {
    const report = computeCoverage(census, null, NOW);
    const code = report.repos.find((r) => r.repo === "developing-today/code");
    expect(code?.openIssues).toBe(1122);
    expect(code?.poolItems).toBeNull();
    expect(code?.poolUnsetStatus).toBeNull();
    expect(code?.poolUnsetArea).toBeNull();
    // No pool -> no pool-only repos appear.
    expect(report.repos.map((r) => r.repo)).toEqual(["developing-today/code", "dezren39/nix"]);
  });

  test("delta against a baseline is null for pool fields while the pool is unavailable", () => {
    const baseline = computeCoverage(census, pool, NOW);
    const current = computeCoverage(census, null, NOW);
    const delta = diffCoverage(current, baseline);
    expect(delta.get("developing-today/code")?.poolItems).toBeNull();
    expect(delta.get("developing-today/code")?.openIssues).toBe(0);
  });
});

describe("snapshot rotation anchor (PR #3309 Codex)", () => {
  const mk = (at: string, codeOpen: number | null): CoverageReport => ({
    generatedAt: at,
    repos: [
      { repo: "developing-today/code", openIssues: codeOpen, gateNow: 1, gateNext: 2, poolItems: 10, poolUnsetStatus: 3, poolUnsetArea: 4 },
    ],
  });

  test("refreshing current within the week does not move the anchor", () => {
    const stored: CoverageSnapshotFile = {
      baseline: null,
      current: mk("2026-10-30T00:00:00Z", 90),
      anchorAt: "2026-10-26T00:00:00Z",
    };
    const snap = nextCoverageSnapshot(stored, mk("2026-11-01T00:00:00Z", 100), NOW);
    expect(snap.anchorAt).toBe("2026-10-26T00:00:00Z");
    expect(snap.baseline).toBeNull();
  });

  test("rotation is measured from the anchor, not the refreshed current timestamp", () => {
    // Current was refreshed yesterday (so its generatedAt is young), but
    // the week anchored 2026-10-25 has fully elapsed by 2026-11-01.
    const stored: CoverageSnapshotFile = {
      baseline: null,
      current: mk("2026-10-31T00:00:00Z", 90),
      anchorAt: "2026-10-25T00:00:00Z",
    };
    const snap = nextCoverageSnapshot(stored, mk("2026-11-01T12:00:00Z", 100), NOW);
    expect(snap.baseline?.generatedAt).toBe("2026-10-31T00:00:00Z");
    expect(snap.anchorAt).toBe("2026-11-01T12:00:00.000Z");
  });

  test("rotation promotes the frozen weekly report, not the refreshed current (PR #3309 Codex)", () => {
    // Codex's reproduction: 100 issues on Nov 1, 160 on Nov 7 at 23:59,
    // 170 at the Nov 8 rotation. The week-over-week delta must be +70
    // (against the frozen Nov 1 report), not +10 (against the Nov 7
    // refresh that `current` happened to hold at rotation time).
    const stored: CoverageSnapshotFile = {
      baseline: null,
      weekly: mk("2026-11-01T00:00:00Z", 100),
      current: mk("2026-11-07T23:59:00Z", 160),
      anchorAt: "2026-11-01T00:00:00Z",
    };
    const snap = nextCoverageSnapshot(stored, mk("2026-11-08T00:00:00Z", 170), new Date("2026-11-08T00:00:00Z"));
    expect(snap.baseline?.generatedAt).toBe("2026-11-01T00:00:00Z");
    expect(diffCoverage(snap.current, snap.baseline).get("developing-today/code")?.openIssues).toBe(70);
    // The fresh report becomes the new week's frozen report.
    expect(snap.weekly?.generatedAt).toBe("2026-11-08T00:00:00Z");
    expect(snap.anchorAt).toBe("2026-11-08T00:00:00.000Z");
  });

  test("within the week the frozen weekly report does not move when current refreshes", () => {
    const stored: CoverageSnapshotFile = {
      baseline: null,
      weekly: mk("2026-11-01T00:00:00Z", 100),
      current: mk("2026-11-03T00:00:00Z", 130),
      anchorAt: "2026-11-01T00:00:00Z",
    };
    const snap = nextCoverageSnapshot(stored, mk("2026-11-05T00:00:00Z", 150), new Date("2026-11-05T00:00:00Z"));
    expect(snap.weekly?.generatedAt).toBe("2026-11-01T00:00:00Z");
    expect(snap.current.repos[0]?.openIssues).toBe(150);
  });

  test("a legacy snapshot without an anchor falls back to current.generatedAt", () => {
    const stored: CoverageSnapshotFile = { baseline: null, current: mk("2026-10-24T00:00:00Z", 90) };
    const snap = nextCoverageSnapshot(stored, mk("2026-11-01T00:00:00Z", 100), NOW);
    expect(snap.baseline?.generatedAt).toBe("2026-10-24T00:00:00Z");
  });
});

describe("CoverageSnapshotStore writes (PR #3309 Codex)", () => {
  test("concurrent writes serialize and leave one valid JSON snapshot — the last one queued", async () => {
    const dir = await mkdtemp(join(tmpdir(), "coverage-store-"));
    try {
      const file = join(dir, "coverage-snapshot.json");
      const store = new CoverageSnapshotStore(file);
      const mkSnap = (open: number): CoverageSnapshotFile => ({
        baseline: null,
        current: {
          generatedAt: "2026-11-01T00:00:00Z",
          repos: [
            { repo: "developing-today/code", openIssues: open, gateNow: 1, gateNext: 2, poolItems: 10, poolUnsetStatus: 3, poolUnsetArea: 4 },
          ],
        },
      });
      const big = mkSnap(1000);
      const small = mkSnap(999);
      await Promise.all([store.write(big), store.write(small), store.write(big)]);
      const parsed = JSON.parse(await readFile(file, "utf8")) as CoverageSnapshotFile;
      expect(parsed.current.repos[0]?.openIssues).toBe(1000);
      expect(await store.read()).toEqual(big);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
});
