// #1116 item 3: checkout-drift classification + health surfacing.
// The git runner is faked — these tests pin the honesty contract:
// in-sync / behind-by-N / unknown-with-reason, never an invented state.
import { describe, expect, test } from "bun:test";
import { computeCheckoutDrift, type GitRun } from "../src/checkout-drift.ts";
import type { FleetFeed } from "../src/feed.ts";
import { buildHealth, createProbeCache, type HealthInputs, type ProbeFetch } from "../src/health.ts";
import { loadConfig } from "../src/config.ts";

const HEAD = "a".repeat(40);
const MAIN = "b".repeat(40);

function fakeGit(table: Record<string, string>): GitRun {
  return async (_cwd, args) => {
    const key = args.join(" ");
    if (key in table) return table[key];
    throw new Error(`unexpected git call: ${key}`);
  };
}

const refs = {
  "rev-parse --verify HEAD^{commit}": HEAD,
  "rev-parse --verify origin/main^{commit}": MAIN,
};

describe("computeCheckoutDrift", () => {
  test("in-sync: HEAD == origin/main, zero counts", async () => {
    const run = fakeGit({
      ...refs,
      "rev-parse --verify origin/main^{commit}": HEAD,
      "rev-list --count HEAD..origin/main": "0",
      "rev-list --count origin/main..HEAD": "0",
    });
    const drift = await computeCheckoutDrift("/x", run);
    expect(drift.state).toBe("in-sync");
    expect(drift.behindBy).toBe(0);
    expect(drift.head).toBe(HEAD);
  });

  test("behind: exact count is reported", async () => {
    const run = fakeGit({
      ...refs,
      "rev-list --count HEAD..origin/main": "7",
      "rev-list --count origin/main..HEAD": "0",
    });
    const drift = await computeCheckoutDrift("/x", run);
    expect(drift).toEqual({ state: "behind", head: HEAD, originMain: MAIN, behindBy: 7 });
  });

  test("unreadable checkout is unknown with a reason, not a state", async () => {
    const run: GitRun = async () => {
      throw new Error("fatal: not a git repository");
    };
    const drift = await computeCheckoutDrift("/x", run);
    expect(drift.state).toBe("unknown");
    expect(drift.reason).toContain("not a git repository");
    expect(drift.behindBy).toBeUndefined();
  });

  test("local commits are unknown with a reason (not a clean pull of main)", async () => {
    const run = fakeGit({
      ...refs,
      "rev-list --count HEAD..origin/main": "2",
      "rev-list --count origin/main..HEAD": "1",
    });
    const drift = await computeCheckoutDrift("/x", run);
    expect(drift.state).toBe("unknown");
    expect(drift.reason).toContain("1 local commit");
    expect(drift.behindBy).toBe(2);
  });
});

describe("buildHealth checkout surfacing", () => {
  const NOW = Date.parse("2026-10-31T00:00:00Z");
  const fetcher: ProbeFetch = async () => ({ status: 200 });

  function inputs(feed: FleetFeed | null): HealthInputs {
    return {
      mode: "live",
      cfg: loadConfig({}),
      feed,
      feedFileMtimeMs: null,
    };
  }

  function feedWith(checkout?: FleetFeed["checkout"]): FleetFeed {
    return {
      version: 1,
      generatedAt: new Date(NOW).toISOString(),
      ...(checkout ? { checkout } : {}),
    } as unknown as FleetFeed;
  }

  test("a feed without a checkout report surfaces unknown-with-reason", async () => {
    const view = await buildHealth(inputs(feedWith()), { fetcher, cache: createProbeCache(), now: NOW });
    expect(view.feed.checkout.state).toBe("unknown");
    expect(view.feed.checkout.reason).toContain("did not report");
  });

  test("behind drift is surfaced verbatim and marks the view degraded", async () => {
    const view = await buildHealth(
      inputs(feedWith({ state: "behind", head: HEAD, originMain: MAIN, behindBy: 3 })),
      { fetcher, cache: createProbeCache(), now: NOW },
    );
    expect(view.feed.checkout).toEqual({ state: "behind", head: HEAD, originMain: MAIN, behindBy: 3 });
    expect(view.degraded).toBe(true);
  });

  test("in-sync drift does not degrade on its own", async () => {
    const view = await buildHealth(
      inputs(feedWith({ state: "in-sync", head: HEAD, originMain: HEAD, behindBy: 0 })),
      { fetcher, cache: createProbeCache(), now: NOW },
    );
    expect(view.feed.checkout.state).toBe("in-sync");
    expect(view.degraded).toBe(false);
  });
});
