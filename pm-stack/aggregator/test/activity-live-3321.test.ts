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
    onActivity?: (d: ActivityDelivery[]) => Promise<boolean> | boolean;
    onSourceGap?: (source: string, from: string, to: string | null) => void;
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
  test("unacknowledged deliveries are retried with the same id; the undated cycle advances seq", async () => {
    const state: HarnessState = { cardStatus: { p1: "in-progress", p2: "in-progress" }, failProjects: new Set(), failAll: false };
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
    const state: HarnessState = { cardStatus: { p1: "in-progress", p2: "in-progress" }, failProjects: new Set(), failAll: false };
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
    const state: HarnessState = { cardStatus: { p1: "in-progress", p2: "in-progress" }, failProjects: new Set(), failAll: false };
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

  test("polling failure opens a gap; the next complete poll reports recovery", async () => {
    const state: HarnessState = { cardStatus: { p1: "in-progress", p2: "in-progress" }, failProjects: new Set(), failAll: false };
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
});
