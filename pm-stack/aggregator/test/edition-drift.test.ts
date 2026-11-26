// CF-9 (#14949): edition-drift classification, the sentinel verdict,
// and health surfacing. The git runner is faked — these tests pin the
// honesty contract: in-sync / behind-by-N with a time-based stale
// rule / unknown-with-reason, never an invented state, and an alarm
// that fires only on a measured stale edition.
import { describe, expect, test } from "bun:test";
import {
  computeEditionDrift,
  editionDriftVerdict,
  EDITION_DRIFT_STALE_MS,
  type EditionDrift,
} from "../src/edition-drift.ts";
import type { GitRun } from "../src/checkout-drift.ts";
import { buildHealth, createProbeCache, type HealthInputs, type ProbeFetch } from "../src/health.ts";
import type { FleetFeed } from "../src/feed.ts";
import { loadConfig } from "../src/config.ts";
import type { RfaState } from "../src/sources/ready-for-agent.ts";

const DEPLOYED = "a".repeat(40);
const MAIN = "b".repeat(40);
const DEPLOYED_AT = "2026-11-25T00:00:00Z";

function fakeGit(table: Record<string, string>): GitRun {
  return async (_cwd, args) => {
    const key = args.join(" ");
    if (key in table) return table[key];
    throw new Error(`unexpected git call: ${key}`);
  };
}

function behindTable(mainAt: string, behindBy = 42): Record<string, string> {
  return {
    "rev-parse --verify origin/main^{commit}": MAIN,
    [`merge-base --is-ancestor ${DEPLOYED} origin/main`]: "",
    [`rev-list --count ${DEPLOYED}..origin/main`]: String(behindBy),
    [`log -1 --format=%cI ${DEPLOYED}`]: DEPLOYED_AT,
    "log -1 --format=%cI origin/main": mainAt,
  };
}

describe("computeEditionDrift", () => {
  test("in-sync: deployed == origin/main", async () => {
    const run = fakeGit({
      "rev-parse --verify origin/main^{commit}": DEPLOYED,
      [`log -1 --format=%cI ${DEPLOYED}`]: DEPLOYED_AT,
    });
    const drift = await computeEditionDrift(DEPLOYED, "/x", run);
    expect(drift.state).toBe("in-sync");
    expect(drift.behindBy).toBe(0);
    expect(drift.stale).toBe(false);
    expect(drift.driftMs).toBe(0);
  });

  test("behind but fresh: main moved 3h ago, under the threshold — not stale", async () => {
    const drift = await computeEditionDrift(DEPLOYED, "/x", fakeGit(behindTable("2026-11-25T03:00:00Z")));
    expect(drift.state).toBe("behind");
    expect(drift.behindBy).toBe(42);
    expect(drift.driftMs).toBe(3 * 3_600_000);
    expect(drift.stale).toBe(false);
  });

  test("behind and stale: main has carried newer code past the threshold", async () => {
    const drift = await computeEditionDrift(DEPLOYED, "/x", fakeGit(behindTable("2026-11-25T12:00:00Z")));
    expect(drift.state).toBe("behind");
    expect(drift.driftMs).toBe(12 * 3_600_000);
    expect(drift.stale).toBe(true);
  });

  test("the threshold is a parameter, not a hardcode", async () => {
    const drift = await computeEditionDrift(DEPLOYED, "/x", fakeGit(behindTable("2026-11-25T03:00:00Z")), 1_000);
    expect(drift.stale).toBe(true);
  });

  test("unknown: the build was never stamped with a commit", async () => {
    const drift = await computeEditionDrift("unknown", "/x", fakeGit({}));
    expect(drift.state).toBe("unknown");
    expect(drift.reason).toContain("not a commit sha");
    expect(drift.stale).toBe(false);
  });

  test("unknown: origin/main unreadable in the checkout", async () => {
    const drift = await computeEditionDrift(DEPLOYED, "/x", fakeGit({}));
    expect(drift.state).toBe("unknown");
    expect(drift.reason).toContain("origin/main is unreadable");
    expect(drift.deployedCommit).toBe(DEPLOYED);
  });

  test("unknown: deployed commit is not an ancestor of origin/main here", async () => {
    const run = fakeGit({ "rev-parse --verify origin/main^{commit}": MAIN });
    const drift = await computeEditionDrift(DEPLOYED, "/x", run);
    expect(drift.state).toBe("unknown");
    expect(drift.reason).toContain("not an ancestor");
    expect(drift.mainHead).toBe(MAIN);
  });
});

describe("editionDriftVerdict", () => {
  const stale: EditionDrift = {
    state: "behind",
    deployedCommit: DEPLOYED,
    mainHead: MAIN,
    behindBy: 6178,
    deployedCommitAt: "2026-11-15T10:47:19Z",
    mainHeadAt: "2026-11-25T10:00:00Z",
    driftMs: 240 * 3_600_000,
    stale: true,
  };

  test("stale is the incident, and the detail names the numbers", () => {
    const verdict = editionDriftVerdict(stale);
    expect(verdict.state).toBe("down");
    expect(verdict.detail).toContain("6178 commits behind");
    expect(verdict.detail).toContain("240h");
    expect(verdict.detail).toContain("overdue");
  });

  test("behind within threshold is up", () => {
    const verdict = editionDriftVerdict({ ...stale, stale: false, driftMs: 2 * 3_600_000 });
    expect(verdict.state).toBe("up");
    expect(verdict.detail).toContain("within the");
  });

  test("unknown never fires the alarm", () => {
    const verdict = editionDriftVerdict({ state: "unknown", stale: false, reason: "git failed" });
    expect(verdict.state).toBe("up");
    expect(verdict.detail).toContain("unknown");
    expect(EDITION_DRIFT_STALE_MS).toBe(6 * 3_600_000);
  });
});

describe("buildHealth edition surfacing", () => {
  const NOW = Date.parse("2026-11-25T12:00:00Z");
  const fetcher: ProbeFetch = async () => ({ status: 200 });
  const RFA_STATE: RfaState = {
    repositories: [],
    backends: [{ id: "opencode", kind: "READY", reason: null }],
    workItems: [],
  };
  const rfaUp = async (): Promise<RfaState> => RFA_STATE;

  // A minimal live feed, mirroring checkout-drift.test.ts: with no
  // feed at all the section health reads as error and degrades the
  // view for reasons unrelated to this test's subject.
  const FEED = { version: 1, generatedAt: new Date(NOW).toISOString() } as unknown as FleetFeed;

  function inputs(edition?: EditionDrift | null): HealthInputs {
    return {
      mode: "live",
      cfg: loadConfig({ PM_AGG_DEV: "1" }),
      feed: FEED,
      feedFileMtimeMs: null,
      ...(edition !== undefined ? { edition } : {}),
    };
  }

  test("a stale edition surfaces verbatim and marks the view degraded", async () => {
    const edition: EditionDrift = {
      state: "behind",
      deployedCommit: DEPLOYED,
      mainHead: MAIN,
      behindBy: 9,
      deployedCommitAt: DEPLOYED_AT,
      mainHeadAt: "2026-11-25T11:00:00Z",
      driftMs: 11 * 3_600_000,
      stale: true,
    };
    const view = await buildHealth(inputs(edition), {
      fetcher,
      cache: createProbeCache(),
      now: NOW,
      rfaState: rfaUp,
    });
    expect(view.edition).toEqual(edition);
    expect(view.degraded).toBe(true);
  });

  test("a fresh behind edition surfaces without degrading on its own", async () => {
    const edition: EditionDrift = {
      state: "behind",
      deployedCommit: DEPLOYED,
      mainHead: MAIN,
      behindBy: 9,
      deployedCommitAt: DEPLOYED_AT,
      mainHeadAt: "2026-11-25T01:00:00Z",
      driftMs: 1 * 3_600_000,
      stale: false,
    };
    const view = await buildHealth(inputs(edition), {
      fetcher,
      cache: createProbeCache(),
      now: NOW,
      rfaState: rfaUp,
    });
    expect(view.edition?.state).toBe("behind");
    expect(view.degraded).toBe(false);
  });

  test("no edition input assembles as null, never an invented in-sync", async () => {
    const view = await buildHealth(inputs(), {
      fetcher,
      cache: createProbeCache(),
      now: NOW,
      rfaState: rfaUp,
    });
    expect(view.edition).toBeNull();
  });
});
