// #3321 (Codex P2s on PR #5141): live-source Kaneo activity discipline —
// deliveries are retained until ingestion is acknowledged, partial
// board polls neither diff nor replace the baseline, and polling
// failures/recoveries are reported as activity gaps.
import { describe, expect, test } from "bun:test";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadConfig } from "../src/config.ts";
import { liveSource } from "../src/sources/live.ts";
import type { ActivityDelivery } from "../src/activity.ts";
import type { FreshnessInput } from "../src/freshness.ts";

const FEED = {
  version: 1,
  generatedAt: new Date().toISOString(),
  quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
  attention: [],
};

interface HarnessState {
  // projectId -> columnId for the single test card in that project
  cardStatus: Record<string, string>;
  failProjects: Set<string>; // task reads 500 for these projects
  failAll: boolean;
}

async function makeSource(
  state: HarnessState,
  opts: {
    onActivity?: (
      d: ActivityDelivery[],
    ) =>
      | Promise<boolean | { failed: number; failedIds?: string[] }>
      | boolean
      | { failed: number; failedIds?: string[] };
    onSourceGap?: (source: string, from: string, to: string | null) => void;
    onFreshness?: (input: FreshnessInput) => Promise<unknown> | unknown;
    seenActivityIds?: (source: string) => Promise<readonly string[]> | readonly string[];
  },
) {
  const dir = await mkdtemp(join(tmpdir(), "pm-activity-live-"));
  const feedFile = join(dir, "fleet-feed.json");
  await writeFile(feedFile, JSON.stringify(FEED));
  const projectsFile = join(dir, "kaneo-projects.json");
  await writeFile(projectsFile, JSON.stringify({ projects: [] }));
  const cfg = loadConfig({
    ...Bun.env,
    PM_AGG_SOURCE: "live",
    PM_AGG_FEED_FILE: feedFile,
    PM_PROJECTOR_URL: "off",
    PM_KANEO_PROJECTS_FILE: projectsFile,
    KANEO_URL: "http://kaneo.test",
    KANEO_ADMIN_EMAIL: "test@example.com",
    KANEO_ADMIN_PASSWORD: "test-password",
  });
  return liveSource(cfg, opts);
}

function installFetch(state: HarnessState): () => void {
  const realFetch = globalThis.fetch;
  globalThis.fetch = (async (input: unknown) => {
    const url = String(input);
    const json = (body: unknown, headers: Record<string, string> = {}) =>
      new Response(JSON.stringify(body), {
        status: 200,
        headers: { "Content-Type": "application/json", ...headers },
      });
    if (url.includes("/api/auth/sign-in/email")) return json({}, { "set-cookie": "session=test; Path=/" });
    if (url.includes("/api/workspace")) return json([{ id: "ws1" }]);
    if (url.includes("/api/project?")) {
      return json([
        { id: "p1", slug: "one", name: "One" },
        { id: "p2", slug: "two", name: "Two" },
      ]);
    }
    if (url.includes("/api/task/tasks/")) {
      const pid = url.split("/api/task/tasks/")[1]!.split("?")[0]!;
      if (state.failAll || state.failProjects.has(pid)) return new Response("boom", { status: 500 });
      const status = state.cardStatus[pid] ?? "in-progress";
      return json({
        data: {
          columns: [
            {
              id: status,
              name: status,
              // No updatedAt: the undated-card path is under test.
              tasks: [{ id: `t-${pid}`, number: 7, title: `Card ${pid}`, status, externalLinks: [] }],
            },
          ],
        },
        pagination: { totalPages: 1 },
      });
    }
    return new Response("not found", { status: 404 });
  }) as typeof fetch;
  return () => {
    globalThis.fetch = realFetch;
  };
}

describe("live kaneo activity (#3321, PR #5141 codex)", () => {
  test("complete board snapshots publish a stable payload-free freshness invalidation", async () => {
    const state: HarnessState = { cardStatus: { p1: "in-progress", p2: "in-progress" }, failProjects: new Set(), failAll: false };
    const restore = installFetch(state);
    const notices: FreshnessInput[] = [];
    try {
      const src = await makeSource(state, { onFreshness: (input) => notices.push(input) });
      await src.board();
      state.cardStatus.p1 = "in-review";
      await src.board();
      await src.board(); // same accepted snapshot retries with the same key
      expect(notices).toHaveLength(3);
      expect(notices[0]!.eventId).not.toBe(notices[1]!.eventId);
      expect(notices[1]!.eventId).toBe(notices[2]!.eventId);
      expect(notices[0]).toMatchObject({ source: "kaneo", scope: ["board"], reason: "source-event" });
      expect(Object.keys(notices[0]!).sort()).toEqual(["eventId", "occurredAt", "reason", "scope", "source"]);

      state.failProjects.add("p2");
      await src.board(); // partial polls do not claim a complete snapshot
      expect(notices).toHaveLength(3);
    } finally {
      restore();
    }
  });

  test("unacknowledged deliveries are retried with the same id; the undated cycle advances seq", async () => {
    const state: HarnessState = {
      cardStatus: { p1: "in-progress", p2: "in-progress" },
      failProjects: new Set(),
      failAll: false,
    };
    const restore = installFetch(state);
    const attempts: ActivityDelivery[][] = [];
    let ack = false;
    try {
      const src = await makeSource(state, {
        onActivity: async (d) => {
          attempts.push(d);
          return ack;
        },
      });
      await src.board(); // baseline
      state.cardStatus.p1 = "in-review";
      await src.board(); // ingestion fails (ack=false): baseline retained
      expect(attempts).toHaveLength(1);
      await src.board(); // retry: same change, same stable delivery id
      expect(attempts).toHaveLength(2);
      expect(attempts[1]![0]!.sourceId).toBe(attempts[0]![0]!.sourceId);
      ack = true;
      await src.board(); // retry now acknowledged: baseline advances
      expect(attempts).toHaveLength(3);
      state.cardStatus.p1 = "in-progress";
      await src.board();
      state.cardStatus.p1 = "in-review";
      await src.board();
      // attempts[0..2] are the first transition and its retries (same
      // id by design); attempts[3] and attempts[4] are the next two
      // transitions of the undated cycle and must be distinct from
      // the first and from each other, with advancing seq stamps.
      expect(attempts).toHaveLength(5);
      const transitionIds = [attempts[2]![0]!.sourceId, attempts[3]![0]!.sourceId, attempts[4]![0]!.sourceId];
      expect(new Set(transitionIds).size).toBe(3);
      expect(transitionIds[0]).toContain("undated:1");
      expect(transitionIds[1]).toContain("undated:2");
      expect(transitionIds[2]).toContain("undated:3");
    } finally {
      restore();
    }
  });

  test("undated seq resumes above the persisted seen ids after a restart", async () => {
    const state: HarnessState = {
      cardStatus: { p1: "in-progress", p2: "in-progress" },
      failProjects: new Set(),
      failAll: false,
    };
    const restore = installFetch(state);
    const seen: ActivityDelivery[] = [];
    try {
      // A previous process already ingested two undated transitions
      // for card t-p1; the store's persisted seen set is the only
      // memory of them. This process must not reissue undated:1.
      const src = await makeSource(state, {
        onActivity: async (d) => {
          seen.push(...d);
          return true;
        },
        seenActivityIds: () => [
          "kaneo:one/t-p1:status:in-progress->in-review:undated:1",
          "kaneo:one/t-p1:status:in-review->in-progress:undated:2",
          "kaneo:two/t-p2:appeared:2026-11-03T00:00:00.000Z",
        ],
      });
      await src.board(); // baseline (seeds the counters)
      expect(seen).toHaveLength(0);
      state.cardStatus.p1 = "in-review";
      await src.board();
      expect(seen).toHaveLength(1);
      expect(seen[0]!.sourceId).toContain("undated:3");
    } finally {
      restore();
    }
  });

  test("a partial poll does not diff or replace the baseline", async () => {
    const state: HarnessState = {
      cardStatus: { p1: "in-progress", p2: "in-progress" },
      failProjects: new Set(),
      failAll: false,
    };
    const restore = installFetch(state);
    const seen: ActivityDelivery[] = [];
    try {
      const src = await makeSource(state, {
        onActivity: async (d) => {
          seen.push(...d);
          return true;
        },
      });
      await src.board(); // baseline
      // p2's read fails (partial poll) while p1's card moves.
      state.failProjects.add("p2");
      state.cardStatus.p1 = "in-review";
      await src.board();
      expect(seen).toHaveLength(0); // no diff from a partial snapshot
      // Reads recover: the preserved baseline still yields the change.
      state.failProjects.clear();
      await src.board();
      expect(seen).toHaveLength(1);
      expect(seen[0]!.summary).toContain("in-progress -> in-review");
    } finally {
      restore();
    }
  });

  test("overlapping board polls serialize diffing: no A->C phantom transition (PR #5141 codex)", async () => {
    const state: HarnessState = {
      cardStatus: { p1: "in-progress", p2: "in-progress" },
      failProjects: new Set(),
      failAll: false,
    };
    const restore = installFetch(state);
    const deliveries: ActivityDelivery[] = [];
    let releaseFirst: (() => void) | null = null;
    let firstCallSeen: (() => void) | null = null;
    const firstCall = new Promise<void>((r) => {
      firstCallSeen = r;
    });
    let calls = 0;
    try {
      const src = await makeSource(state, {
        onActivity: async (d) => {
          deliveries.push(...d);
          calls++;
          if (calls === 1) {
            firstCallSeen!();
            await new Promise<void>((r) => {
              releaseFirst = r;
            });
          }
          return true;
        },
      });
      await src.board(); // baseline: both cards in-progress
      state.cardStatus.p1 = "in-review";
      const b1 = src.board(); // diffs in-progress -> in-review, then blocks in onActivity
      await firstCall;
      state.cardStatus.p1 = "in-progress";
      const b2 = src.board(); // fetches the return to in-progress while the first poll is still ingesting
      releaseFirst!();
      await Promise.all([b1, b2]);
      const p1Moves = deliveries.filter((d) => d.ref.includes("t-p1")).map((d) => d.summary);
      // Serialized: the second poll diffed against the ADVANCED
      // baseline, so history is in-progress -> in-review -> in-progress.
      // Unserialized it would diff in-progress -> in-progress and
      // derive no delivery at all for the second move.
      expect(p1Moves.some((s) => s.includes("in-progress -> in-review"))).toBe(true);
      expect(p1Moves.some((s) => s.includes("in-review -> in-progress"))).toBe(true);
      expect(p1Moves).toHaveLength(2);
    } finally {
      restore();
    }
  });

  test("partial failure advances acknowledged cards only: the succeeded card's return move is not lost (PR #5141 codex)", async () => {
    const state: HarnessState = {
      cardStatus: { p1: "in-progress", p2: "in-progress" },
      failProjects: new Set(),
      failAll: false,
    };
    const restore = installFetch(state);
    const attempts: ActivityDelivery[][] = [];
    let failP2 = true;
    try {
      const src = await makeSource(state, {
        onActivity: async (d) => {
          attempts.push(d);
          if (failP2) {
            const p2 = d.filter((x) => x.ref.includes("t-p2"));
            return { failed: p2.length, failedIds: p2.map((x) => x.sourceId) };
          }
          return { failed: 0, failedIds: [] };
        },
      });
      await src.board(); // baseline: both cards in-progress
      state.cardStatus.p1 = "in-review";
      state.cardStatus.p2 = "in-review";
      await src.board(); // both move; p2's delivery fails, p1's lands
      expect(attempts).toHaveLength(1);
      const p2FailedId = attempts[0]!.find((x) => x.ref.includes("t-p2"))!.sourceId;
      // p1 returns before the next poll; p2 stays in-review. Because
      // p1's baseline advanced, its return move diffs and emits; p2's
      // retained baseline re-diffs its failed change with the same id.
      state.cardStatus.p1 = "in-progress";
      failP2 = false;
      await src.board();
      expect(attempts).toHaveLength(2);
      const second = attempts[1]!;
      const p1Return = second.find((x) => x.ref.includes("t-p1"));
      expect(p1Return?.summary).toContain("in-review -> in-progress");
      const p2Retry = second.find((x) => x.ref.includes("t-p2"));
      expect(p2Retry?.sourceId).toBe(p2FailedId);
      // A third identical poll emits nothing: both baselines advanced.
      await src.board();
      expect(attempts).toHaveLength(2);
    } finally {
      restore();
    }
  });

  test("polling failure opens a gap; the next complete poll reports recovery", async () => {
    const state: HarnessState = {
      cardStatus: { p1: "in-progress", p2: "in-progress" },
      failProjects: new Set(),
      failAll: false,
    };
    const restore = installFetch(state);
    const gaps: { source: string; from: string; to: string | null }[] = [];
    try {
      const src = await makeSource(state, {
        onSourceGap: (source, from, to) => {
          gaps.push({ source, from, to });
        },
      });
      await src.board(); // healthy baseline: exactly one startup gap
      // reconciliation (to=null), so a gap persisted by a previous
      // process that died mid-outage gets resolved by the server sink
      expect(gaps).toHaveLength(1);
      expect(gaps[0]!.source).toBe("kaneo");
      expect(gaps[0]!.to).toBeNull();
      state.failAll = true; // every task read fails -> fixtures fallback
      await src.board();
      expect(gaps.length).toBeGreaterThan(1);
      expect(gaps[1]!.source).toBe("kaneo");
      expect(gaps[1]!.to).not.toBeNull();
      state.failAll = false;
      const beforeRecovery = gaps.length;
      await src.board(); // recovery is reported with to=null
      expect(gaps.length).toBe(beforeRecovery + 1);
      expect(gaps[gaps.length - 1]!.to).toBeNull();
      await src.board(); // healthy again: the reconciliation does not repeat
      expect(gaps.length).toBe(beforeRecovery + 1);
    } finally {
      restore();
    }
  });

  test("a gap noted behind a queued complete poll is not resolved by that poll (PR #5141 codex round 4)", async () => {
    const state: HarnessState = {
      cardStatus: { p1: "in-progress", p2: "in-progress" },
      failProjects: new Set(),
      failAll: false,
    };
    const restore = installFetch(state);
    // Counting wrapper: the queued complete poll (b1) has finished
    // fetching once 6 task reads have been served (baseline 2 +
    // blocked poll 2 + b1 2), so the failure flag can flip without
    // failing b1's own reads.
    let taskFetches = 0;
    let resolveQueuedFetched: (() => void) | null = null;
    const queuedFetched = new Promise<void>((r) => {
      resolveQueuedFetched = r;
    });
    const mockFetch = globalThis.fetch;
    globalThis.fetch = (async (input: unknown, init?: unknown) => {
      const res = await (mockFetch as (i: unknown, i2?: unknown) => Promise<Response>)(input, init);
      if (String(input).includes("/api/task/tasks/")) {
        taskFetches++;
        if (taskFetches === 6) resolveQueuedFetched!();
      }
      return res;
    }) as typeof fetch;
    const gaps: { source: string; from: string; to: string | null }[] = [];
    let releaseFirst: (() => void) | null = null;
    let firstCallSeen: (() => void) | null = null;
    const firstCall = new Promise<void>((r) => {
      firstCallSeen = r;
    });
    let calls = 0;
    try {
      const src = await makeSource(state, {
        onActivity: async () => {
          calls++;
          if (calls === 1) {
            firstCallSeen!();
            await new Promise<void>((r) => {
              releaseFirst = r;
            });
          }
          return true;
        },
        onSourceGap: (source, from, to) => {
          gaps.push({ source, from, to });
        },
      });
      await src.board(); // baseline: startup reconciliation only
      expect(gaps).toHaveLength(1);
      state.cardStatus.p1 = "in-review";
      const b0 = src.board(); // diffs, then blocks in onActivity
      await firstCall;
      const b1 = src.board(); // complete poll: queues behind b0's ingestion
      await queuedFetched; // b1 has fetched; its ingestion is queued
      state.failAll = true;
      const b2 = src.board(); // failing poll: its gap must queue behind b1
      await new Promise((r) => setTimeout(r, 25)); // let b2 reach its gap transition
      releaseFirst!();
      await Promise.all([b0, b1, b2]);
      // Serialized: b1's recovery ran BEFORE b2's gap existed, so the
      // gap is still open — no recovery entry follows it. Unserialized
      // (gap noted immediately), b1 would emit a recovery (to=null)
      // after the gap and close an outage no poll has recovered from.
      expect(gaps).toHaveLength(2);
      expect(gaps[1]!.to).not.toBeNull();
      state.failAll = false;
      await src.board(); // a genuinely later complete poll recovers it
      expect(gaps).toHaveLength(3);
      expect(gaps[2]!.to).toBeNull();
    } finally {
      restore();
    }
  });
});
