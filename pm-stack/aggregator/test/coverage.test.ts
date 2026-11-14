import { describe, expect, test } from "bun:test";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  computeCoverage,
  CoverageSnapshotStore,
  diffCoverage,
  isRepoCensus,
  isValidAnchorAt,
  nextCoverageSnapshot,
  staleIssueSearchQuery,
  staleIssueState,
  STALE_ISSUE_CENSUS_STALE_MS,
  type CoverageReport,
  type CoverageSnapshotFile,
  type RepoCensus,
  type RepoCoverage,
} from "../src/coverage.ts";
import { coverageFromFeed } from "../src/feed.ts";
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

  test("rejects negative counts and gate counts above the open total", () => {
    const valid = { repo: "a/b", openIssues: 10, gateNow: 3, gateNext: 2 };
    for (const counts of [
      { openIssues: -1, gateNow: 0, gateNext: 0 },
      { openIssues: 10, gateNow: -1, gateNext: 0 },
      { openIssues: 10, gateNow: 0, gateNext: -1 },
      { openIssues: 10, gateNow: 11, gateNext: 0 },
      { openIssues: 10, gateNow: 0, gateNext: 11 },
    ]) {
      expect(isRepoCensus({ ...valid, ...counts })).toBe(false);
    }
  });

  test("accepts null counts: the partial-failure row #3992 emits (#8193)", () => {
    // Oracle: the RepoCensus type itself - each count is a non-negative
    // integer or null ("unavailable, not zero").
    expect(isRepoCensus({ repo: "a/b", openIssues: null, gateNow: null, gateNext: null })).toBe(true);
    expect(isRepoCensus({ repo: "a/b", openIssues: null, gateNow: 3, gateNext: 2 })).toBe(true);
    expect(isRepoCensus({ repo: "a/b", openIssues: 10, gateNow: null, gateNext: null })).toBe(true);
    expect(isRepoCensus({ repo: "a/b", openIssues: 10, gateNow: 3, gateNext: null })).toBe(true);
    // The stale-issue fields already treat a failed query this way.
    expect(
      isRepoCensus({
        repo: "a/b",
        openIssues: null,
        gateNow: null,
        gateNext: null,
        staleOpenIssues: 4,
        staleIssuesCollectedAt: "2026-11-01T11:59:00.000Z",
      }),
    ).toBe(true);
  });

  test("still rejects impossible rows when nulls are in play (#8193)", () => {
    for (const counts of [
      { openIssues: null, gateNow: -1, gateNext: 0 },
      { openIssues: null, gateNow: 0, gateNext: 1.5 },
      { openIssues: -1, gateNow: null, gateNext: null },
      { openIssues: "10", gateNow: null, gateNext: null },
      { openIssues: null, gateNow: true, gateNext: null },
      { openIssues: 5, gateNow: 6, gateNext: null },
      { openIssues: 5, gateNow: null, gateNext: 6 },
    ]) {
      expect(isRepoCensus({ repo: "a/b", ...counts })).toBe(false);
    }
  });

  test("agrees with an independent acceptance oracle on generated rows (#8193 Codex P1)", () => {
    // Oracle: the RepoCensus contract restated here independently of
    // the implementation - each count is a non-negative integer or
    // null; the gate-vs-total ordering binds only when both sides are
    // numbers; the stale fields follow their pre-existing rules.
    const countOk = (v: unknown) => v === null || (typeof v === "number" && Number.isInteger(v) && v >= 0);
    const oracle = (row: Record<string, unknown>): boolean => {
      if (row.repo !== "a/b") return false;
      if (!countOk(row.openIssues) || !countOk(row.gateNow) || !countOk(row.gateNext)) return false;
      const le = (gate: unknown, total: unknown) =>
        gate === null || total === null || (gate as number) <= (total as number);
      if (!le(row.gateNow, row.openIssues) || !le(row.gateNext, row.openIssues)) return false;
      const stale = row.staleOpenIssues;
      if (!(stale === undefined || countOk(stale))) return false;
      const at = row.staleIssuesCollectedAt;
      if (!(at === undefined || at === null || (typeof at === "string" && Number.isFinite(Date.parse(at)))))
        return false;
      const err = row.staleIssuesError;
      if (!(err === undefined || err === null || typeof err === "string")) return false;
      return true;
    };
    // Deterministic LCG so the generated cases are reproducible.
    let seed = 8193;
    const next = () => (seed = (seed * 1103515245 + 12345) % 2147483648);
    const pick = <T>(xs: readonly T[]): T => xs[next() % xs.length];
    const counts: unknown[] = [
      null,
      0,
      1,
      2,
      5,
      10,
      999,
      -1,
      -100,
      1.5,
      0.5,
      "3",
      "0",
      "",
      true,
      false,
      undefined,
      NaN,
      Infinity,
      [],
      {},
    ];
    const staleCounts: unknown[] = [undefined, null, 0, 4, 90, -1, 1.5, "4", true];
    const stamps: unknown[] = [
      undefined,
      null,
      "2026-11-01T11:59:00.000Z",
      "2025-02-28T00:00:00Z",
      "not-a-date",
      7,
      {},
    ];
    const errors: unknown[] = [undefined, null, "search failed", "", 7, false, []];
    const repos: unknown[] = ["a/b", "a/b", "a/b", "ab", "", "a/", "/b", "a/b/c", 7, null];
    for (let i = 0; i < 900; i++) {
      const row: Record<string, unknown> = {
        repo: pick(repos),
        openIssues: pick(counts),
        gateNow: pick(counts),
        gateNext: pick(counts),
      };
      if (next() % 2) row.staleOpenIssues = pick(staleCounts);
      if (next() % 2) row.staleIssuesCollectedAt = pick(stamps);
      if (next() % 2) row.staleIssuesError = pick(errors);
      expect(isRepoCensus(row)).toBe(oracle(row));
    }
  });

  test("requires exactly one valid owner/name repository ref", () => {
    for (const repo of [
      "",
      "/repo",
      "owner/",
      "owner/name/extra",
      "owner name/repo",
      "owner/repo name",
      "owner//repo",
    ]) {
      expect(isRepoCensus({ repo, openIssues: 1, gateNow: 0, gateNext: 0 })).toBe(false);
    }
  });
});

describe("coverageFromFeed duplicate census rows", () => {
  test("refuses an ambiguous census instead of choosing a duplicate row", () => {
    expect(
      coverageFromFeed({
        coverage: [
          { repo: "a/b", openIssues: 10, gateNow: 1, gateNext: 2 },
          { repo: "a/b", openIssues: 20, gateNow: 3, gateNext: 4 },
          { repo: "c/d", openIssues: 5, gateNow: 0, gateNext: 1 },
        ],
      }),
    ).toBeNull();
  });
});

describe("coverageFromFeed null census rows (#8193)", () => {
  test("a null-count row survives the feed gate into the coverage report", () => {
    const rows = coverageFromFeed({
      coverage: [
        { repo: "a/b", openIssues: null, gateNow: null, gateNext: null, staleOpenIssues: 4 },
        { repo: "c/d", openIssues: 5, gateNow: 1, gateNext: 1 },
      ],
    });
    // The null-count row passes the feed gate with its counts still
    // null - not silently dropped, not zero-filled.
    expect(rows?.map((r) => r.repo)).toEqual(["a/b", "c/d"]);
    expect(rows?.[0]).toMatchObject({ openIssues: null, gateNow: null, gateNext: null, staleOpenIssues: 4 });
    // And it reaches the coverage report as unavailable counts.
    const report = computeCoverage(rows ?? [], null, NOW);
    const ab = report.repos.find((r) => r.repo === "a/b");
    expect(ab?.openIssues).toBeNull();
    expect(ab?.gateNow).toBeNull();
    expect(ab?.gateNext).toBeNull();
    expect(ab?.staleOpenIssues).toBe(4);
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
    expect(report.repos.map((r) => r.repo)).toEqual([
      "developing-today/code",
      "developing-today/opencode",
      "dezren39/nix",
    ]);
    expect(report.generatedAt).toBe("2026-11-01T12:00:00.000Z");
  });
});

describe("snapshot rotation + delta", () => {
  const mk = (at: string, codeOpen: number | null): CoverageReport => ({
    generatedAt: at,
    repos: [
      {
        repo: "developing-today/code",
        openIssues: codeOpen,
        gateNow: 1,
        gateNext: 2,
        poolItems: 10,
        poolUnsetStatus: 3,
        poolUnsetArea: 4,
        staleOpenIssues: 5,
        staleIssuesCollectedAt: "2026-11-01T00:00:00Z",
        staleIssuesError: null,
      },
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

describe("snapshot rotation — poisoned anchorAt (#5196)", () => {
  const mk = (at: string, codeOpen: number | null): CoverageReport => ({
    generatedAt: at,
    repos: [
      {
        repo: "developing-today/code",
        openIssues: codeOpen,
        gateNow: 1,
        gateNext: 2,
        poolItems: 10,
        poolUnsetStatus: 3,
        poolUnsetArea: 4,
      },
    ],
  });

  test("a garbage anchorAt falls back to current.generatedAt and rotation fires on schedule", () => {
    const stored: CoverageSnapshotFile = {
      baseline: null,
      current: mk("2026-10-01T00:00:00Z", 90),
      weekly: mk("2026-10-01T00:00:00Z", 90),
      anchorAt: "not-a-date",
    };
    const snap = nextCoverageSnapshot(stored, mk("2026-11-01T00:00:00Z", 100), NOW);
    expect(snap.baseline?.generatedAt).toBe("2026-10-01T00:00:00Z");
    expect(snap.anchorAt).toBe("2026-11-01T12:00:00.000Z");
  });

  test("an impossible anchorAt (2026-02-30) is treated like an unparseable one", () => {
    const stored: CoverageSnapshotFile = {
      baseline: null,
      current: mk("2026-10-01T00:00:00Z", 90),
      weekly: mk("2026-10-01T00:00:00Z", 90),
      anchorAt: "2026-02-30T00:00:00Z",
    };
    const snap = nextCoverageSnapshot(stored, mk("2026-11-01T00:00:00Z", 100), NOW);
    expect(snap.baseline?.generatedAt).toBe("2026-10-01T00:00:00Z");
    expect(snap.anchorAt).toBe("2026-11-01T12:00:00.000Z");
  });

  test("the store reads a poisoned-anchor file as absent and a well-formed one as itself", async () => {
    const dir = await mkdtemp(join(tmpdir(), "coverage-anchor-"));
    try {
      const file = join(dir, "coverage-snapshot.json");
      const store = new CoverageSnapshotStore(file);
      const good: CoverageSnapshotFile = {
        baseline: null,
        current: mk("2026-10-01T00:00:00Z", 90),
        anchorAt: "2026-10-01T00:00:00Z",
      };
      await store.write(good);
      expect(await store.read()).toEqual(good);
      await store.write({ ...good, anchorAt: "not-a-date" });
      expect(await store.read()).toBeNull();
      await store.write({ ...good, anchorAt: "2026-02-30T00:00:00Z" });
      expect(await store.read()).toBeNull();
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });

  test("the fallback anchor rotates a week later instead of freezing", () => {
    // The freeze bug was one week downstream: main kept returning the
    // poisoned anchor, so no later read could ever cross the week. The
    // fallback must be a real date that the NEXT read rotates from.
    const stored: CoverageSnapshotFile = {
      baseline: null,
      current: mk("2026-10-31T00:00:00Z", 90),
      weekly: mk("2026-10-24T00:00:00Z", 80),
      anchorAt: "2026-02-30T00:00:00Z",
    };
    const snap = nextCoverageSnapshot(stored, mk("2026-11-01T00:00:00Z", 100), NOW);
    expect(snap.anchorAt).toBe("2026-10-31T00:00:00Z");
    expect(snap.baseline).toBeNull();
    const aWeekLater = new Date("2026-11-08T00:00:00Z");
    const rotated = nextCoverageSnapshot(snap, mk("2026-11-08T00:00:00Z", 120), aWeekLater);
    expect(rotated.anchorAt).toBe("2026-11-08T00:00:00.000Z");
    expect(rotated.baseline?.generatedAt).toBe("2026-10-24T00:00:00Z");
  });

  test("isValidAnchorAt agrees with an independent calendar oracle on generated dates", () => {
    // Oracle: plain leap-year + days-in-month arithmetic, written here
    // independently of the implementation Date round-trip (#5196 Codex
    // P1: parser invariants get property tests, not picked examples).
    const isLeap = (y: number) => y % 4 === 0 && (y % 100 !== 0 || y % 400 === 0);
    const daysInMonth = (y: number, m: number) =>
      [31, isLeap(y) ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][m - 1];
    // Deterministic LCG so the generated cases are reproducible.
    let seed = 5196;
    const next = () => (seed = (seed * 1103515245 + 12345) % 2147483648);
    const pad = (n: number) => String(n).padStart(2, "0");
    for (let i = 0; i < 600; i++) {
      const year = 1900 + (next() % 400);
      const month = 1 + (next() % 14); // 13-14: invalid months
      const day = next() % 34; // 0 and 32-33: invalid days
      const stamp = year + "-" + pad(month) + "-" + pad(day) + "T00:00:00Z";
      const expected = month <= 12 && day >= 1 && day <= daysInMonth(year, month);
      expect(isValidAnchorAt(stamp)).toBe(expected);
    }
    // Non-string and unparseable shapes never validate.
    for (const junk of ["", "not-a-date", null, undefined, 0, {}]) expect(isValidAnchorAt(junk)).toBe(false);
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

describe("stale open-issue census", () => {
  test("query uses a strict 90-day cutoff and excludes pull requests", () => {
    expect(staleIssueSearchQuery("a/b", new Date("2026-11-01T00:00:00Z"))).toBe(
      "repo:a/b is:issue is:open updated:<2026-08-03T00:00:00.000Z",
    );
  });

  test("freshness turns stale only after seven full days", () => {
    const repo = {
      staleOpenIssues: 7,
      staleIssuesCollectedAt: "2026-10-25T12:00:00.000Z",
      staleIssuesError: null,
    };
    expect(staleIssueState(repo, new Date(Date.parse(repo.staleIssuesCollectedAt) + STALE_ISSUE_CENSUS_STALE_MS))).toBe(
      "current",
    );
    expect(
      staleIssueState(repo, new Date(Date.parse(repo.staleIssuesCollectedAt) + STALE_ISSUE_CENSUS_STALE_MS + 1)),
    ).toBe("stale");
    expect(staleIssueState({ ...repo, staleOpenIssues: null, staleIssuesError: "search failed" }, NOW)).toBe("error");
    expect(staleIssueState({ ...repo, staleOpenIssues: null, staleIssuesCollectedAt: null }, NOW)).toBe("unavailable");
  });

  test("census rows carry the count and actual collection timestamp without treating missing values as zero", () => {
    const row: RepoCensus = {
      repo: "developing-today/code",
      openIssues: 100,
      gateNow: 2,
      gateNext: 3,
      staleOpenIssues: 12,
      staleIssuesCollectedAt: "2026-11-01T11:59:00.000Z",
    };
    expect(isRepoCensus(row)).toBe(true);
    const report = computeCoverage([row], null, NOW);
    expect(report.repos.find((r) => r.repo === row.repo)).toMatchObject({
      staleOpenIssues: 12,
      staleIssuesCollectedAt: row.staleIssuesCollectedAt,
      staleIssuesError: null,
    });
    // Repos with no census row do not appear (union of census + pool, #1220).
    expect(report.repos.map((r) => r.repo)).toEqual(["developing-today/code"]);
    expect(isRepoCensus({ ...row, staleOpenIssues: 1.5 })).toBe(false);
  });
});

describe("snapshot rotation anchor (PR #3309 Codex)", () => {
  const mk = (at: string, codeOpen: number | null): CoverageReport => ({
    generatedAt: at,
    repos: [
      {
        repo: "developing-today/code",
        openIssues: codeOpen,
        gateNow: 1,
        gateNext: 2,
        poolItems: 10,
        poolUnsetStatus: 3,
        poolUnsetArea: 4,
        staleOpenIssues: 5,
        staleIssuesCollectedAt: "2026-11-01T00:00:00Z",
        staleIssuesError: null,
      },
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
            {
              repo: "developing-today/code",
              openIssues: open,
              gateNow: 1,
              gateNext: 2,
              poolItems: 10,
              poolUnsetStatus: 3,
              poolUnsetArea: 4,
              staleOpenIssues: 5,
              staleIssuesCollectedAt: "2026-11-01T00:00:00Z",
              staleIssuesError: null,
            },
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
describe("CoverageSnapshotStore reads (#4140)", () => {
  const row: RepoCoverage = {
    repo: "developing-today/code",
    openIssues: 10,
    gateNow: 1,
    gateNext: 2,
    staleOpenIssues: null,
    staleIssuesCollectedAt: null,
    staleIssuesError: null,
    poolItems: 3,
    poolUnsetStatus: 1,
    poolUnsetArea: 0,
  };
  const report: CoverageReport = { generatedAt: "2026-11-01T00:00:00Z", repos: [row] };
  const goodFile: CoverageSnapshotFile = {
    baseline: report,
    current: report,
    weekly: report,
    anchorAt: "2026-11-01T00:00:00Z",
  };

  async function readRaw(raw: unknown): Promise<CoverageSnapshotFile | null> {
    const dir = await mkdtemp(join(tmpdir(), "coverage-store-read-"));
    try {
      const file = join(dir, "coverage-snapshot.json");
      await writeFile(file, typeof raw === "string" ? raw : JSON.stringify(raw), "utf8");
      return await new CoverageSnapshotStore(file).read();
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  }

  test("a well-formed file reads back; pre-weekly/pre-anchor files keep their fallbacks", async () => {
    expect(await readRaw(goodFile)).toEqual(goodFile);
    const legacy = { baseline: null, current: report };
    expect(await readRaw(legacy)).toEqual(legacy);
  });

  test("a malformed baseline or weekly reads as absent", async () => {
    expect(await readRaw({ ...goodFile, baseline: "junk" })).toBeNull();
    expect(await readRaw({ ...goodFile, baseline: { generatedAt: "2026-11-01T00:00:00Z" } })).toBeNull();
    expect(await readRaw({ ...goodFile, baseline: { generatedAt: "2026-11-01T00:00:00Z", repos: "nope" } })).toBeNull();
    expect(await readRaw({ ...goodFile, weekly: "junk" })).toBeNull();
  });

  test("garbage row field types read as absent, in any report slot", async () => {
    const badRow = { ...row, openIssues: "ten" };
    const badReport = { generatedAt: "2026-11-01T00:00:00Z", repos: [badRow] };
    expect(await readRaw({ ...goodFile, current: badReport })).toBeNull();
    expect(await readRaw({ ...goodFile, baseline: badReport })).toBeNull();
    expect(await readRaw("{not json")).toBeNull();
  });

  test("a present-but-unusable anchorAt reads as absent under the whole-file gate; absent stays legacy-valid (#5196)", async () => {
    // Garbage and impossible dates are corruption, not a silent fallback:
    // the whole file reads as absent, so the poisoned anchor is never
    // re-persisted on a panel read and rotation cannot freeze on it.
    expect(await readRaw({ ...goodFile, anchorAt: "not-a-date" })).toBeNull();
    expect(await readRaw({ ...goodFile, anchorAt: "2026-02-30T00:00:00Z" })).toBeNull();
    expect(await readRaw({ ...goodFile, anchorAt: "" })).toBeNull();
    expect(await readRaw({ ...goodFile, anchorAt: 1234 })).toBeNull();
    // Files written before anchorAt existed keep their fallback.
    const preAnchor = { baseline: null, current: report, weekly: report };
    expect(await readRaw(preAnchor)).toEqual(preAnchor);
  });
});

describe("isValidAnchorAt (#5196)", () => {
  test("real timestamps pass; junk, impossible dates and non-strings fail", () => {
    expect(isValidAnchorAt("2026-11-01T00:00:00Z")).toBe(true);
    expect(isValidAnchorAt("2026-11-01")).toBe(true);
    expect(isValidAnchorAt("")).toBe(false);
    expect(isValidAnchorAt("not-a-date")).toBe(false);
    expect(isValidAnchorAt("2026-02-30T00:00:00Z")).toBe(false);
    expect(isValidAnchorAt(undefined)).toBe(false);
    expect(isValidAnchorAt(1234)).toBe(false);
  });
});
