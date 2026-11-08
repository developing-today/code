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
  type CoverageReport,
  type CoverageSnapshotFile,
  type RepoCensus,
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
  const row = {
    repo: "developing-today/code",
    openIssues: 10,
    gateNow: 1,
    gateNext: 2,
    poolItems: 3,
    poolUnsetStatus: 1,
    poolUnsetArea: 0,
  };
  const report = { generatedAt: "2026-11-01T00:00:00Z", repos: [row] };
  const goodFile = { baseline: report, current: report, weekly: report, anchorAt: "2026-11-01T00:00:00Z" };

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
