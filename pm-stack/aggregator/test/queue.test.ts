import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import {
  fetchQueueSnapshot,
  parseDedupeIssueRef,
  projectQueueSnapshot,
  projectQueueTask,
  unavailableQueueSnapshot,
  QUEUE_STALE_MS,
  type BrokerHealthRow,
  type BrokerTaskRow,
} from "../src/queue.ts";

const NOW = Date.parse("2026-11-14T15:00:00Z");

const HEALTH: BrokerHealthRow[] = [
  {
    queue: "repo-health-code-pilot",
    queued: 3,
    claimed: 2,
    parked: 5,
    blocked: 1,
    done: 40,
    failed: 1,
    cancelled: 0,
    handed_off: 1,
    dep_blocked: 2,
    total: 52,
    terminal_events_24h: 7,
    terminal_events_7d: 42,
    oldest_queued_at: "2026-11-14T14:30:00Z",
  },
  { queue: "code", queued: 0, total: 8, oldest_queued_at: null },
];

const TASKS: BrokerTaskRow[] = [
  {
    id: 16834,
    queue: "repo-health-code-pilot",
    title: "Fleet unification slice 2 remainder",
    priority: 5,
    status: "claimed",
    dedupe_key: "repo-health:v1:fix:developing-today/code:issue:8580",
    created_at: "2026-11-14T13:00:00Z",
    worker_id: "repo-worker-code",
    lane_id: "any-d",
    lease_until: "2026-11-14T15:30:00Z",
    attempts: 1,
    max_attempts: 3,
    progress: { stage: "in_progress", next: "backfill proof", stage_at: "2026-11-14T14:31:00Z" },
    payload: { repo: "developing-today/code", issue: { number: 8580 } },
  },
  {
    id: 999,
    queue: "repo-health-code-pilot",
    title: "No payload issue; dedupe only",
    status: "queued",
    dedupe_key: "repo-health:v1:triage:developing-today/code:issue:1234",
    payload: null,
  },
  {
    id: 1000,
    queue: "scratch-6634",
    title: "Unrelated queue family",
    status: "done",
    dedupe_key: "scratch:6634:absolute-park:1",
    payload: null,
  },
];

describe("parseDedupeIssueRef", () => {
  test("decodes the repo-health join key", () => {
    expect(parseDedupeIssueRef("repo-health:v1:fix:developing-today/code:issue:948")).toEqual({
      repo: "developing-today/code",
      issue: 948,
    });
  });
  test("returns null for non-issue keys and empty input", () => {
    expect(parseDedupeIssueRef("scratch:6634:absolute-park:1")).toBeNull();
    expect(parseDedupeIssueRef(null)).toBeNull();
    expect(parseDedupeIssueRef(undefined)).toBeNull();
  });
});

describe("projectQueueTask join", () => {
  test("dedupe and payload agree", () => {
    const t = projectQueueTask(TASKS[0]);
    expect(t.repo).toBe("developing-today/code");
    expect(t.issue).toBe(8580);
    expect(t.join).toBe("dedupe");
    expect(t.worker).toBe("repo-worker-code");
    expect(t.lane).toBe("any-d");
    expect(t.progressStage).toBe("in_progress");
  });
  test("dedupe-only join without payload", () => {
    const t = projectQueueTask(TASKS[1]);
    expect(t.issue).toBe(1234);
    expect(t.join).toBe("dedupe");
  });
  test("payload wins on mismatch and the disagreement is flagged", () => {
    const t = projectQueueTask({
      ...TASKS[0],
      payload: { repo: "developing-today/code", issue: { number: 7777 } },
    });
    expect(t.issue).toBe(7777);
    expect(t.join).toBe("payload-mismatch");
  });
  test("non-repo-health tasks project with no join", () => {
    const t = projectQueueTask(TASKS[2]);
    expect(t.issue).toBeNull();
    expect(t.join).toBe("none");
  });
  test("lane falls back to the progress lane stamp", () => {
    const t = projectQueueTask({
      id: 5,
      queue: "q",
      status: "claimed",
      lane_id: null,
      progress: { lane: "code-b", stage: "x" },
    });
    expect(t.lane).toBe("code-b");
  });
});

describe("projectQueueSnapshot", () => {
  test("projects counts, oldest-queued age, and task rows", () => {
    const snap = projectQueueSnapshot(HEALTH, TASKS, NOW, NOW);
    expect(snap.available).toBe(true);
    expect(snap.stale).toBe(false);
    expect(snap.fetchedAt).toBe(NOW);
    const pilot = snap.queues.find((q) => q.queue === "repo-health-code-pilot");
    expect(pilot?.queued).toBe(3);
    expect(pilot?.parked).toBe(5);
    expect(pilot?.oldestQueuedAgeMs).toBe(30 * 60_000);
    expect(pilot?.terminalEvents24h).toBe(7);
    expect(pilot?.terminalEvents7d).toBe(42);
    const code = snap.queues.find((q) => q.queue === "code");
    expect(code?.oldestQueuedAgeMs).toBeNull();
    expect(snap.tasks).toHaveLength(3);
  });
  test("a snapshot older than the stale threshold is stale", () => {
    const snap = projectQueueSnapshot(HEALTH, TASKS, NOW - QUEUE_STALE_MS - 1, NOW);
    expect(snap.stale).toBe(true);
    expect(snap.available).toBe(true);
  });
  test("errors make the snapshot unavailable and stale, never a throw", () => {
    const snap = projectQueueSnapshot([], [], NOW, NOW, ["broker down"]);
    expect(snap.available).toBe(false);
    expect(snap.stale).toBe(true);
    expect(snap.errors).toEqual(["broker down"]);
  });
  test("unavailable snapshot shape", () => {
    const snap = unavailableQueueSnapshot(NOW, "no token");
    expect(snap.available).toBe(false);
    expect(snap.stale).toBe(true);
    expect(snap.queues).toEqual([]);
    expect(snap.tasks).toEqual([]);
  });
});

describe("fetchQueueSnapshot (fixture broker)", () => {
  const calls: { url: string; init?: { headers?: Record<string, string>; body?: string } }[] = [];
  const stubFetch = async (url: string, init?: { headers?: Record<string, string>; body?: string }) => {
    calls.push({ url, init });
    const body = JSON.parse(init?.body ?? "{}") as { queue?: string };
    if (url.endsWith("/health")) {
      return { ok: true, status: 200, json: async () => ({ ok: true, rows: HEALTH }) };
    }
    if (url.endsWith("/list")) {
      const rows = TASKS.filter((t) => t.queue === body.queue);
      return { ok: true, status: 200, json: async () => ({ ok: true, rows }) };
    }
    throw new Error(`unexpected url ${url}`);
  };
  test("reads /health then /list per queue with the agent token, read verbs only", async () => {
    calls.length = 0;
    const snap = await fetchQueueSnapshot("http://broker.test:4801", "tok-123", stubFetch as never, () => NOW);
    expect(snap.available).toBe(true);
    // Only queues named by /health get a /list call: the two pilot
    // tasks come back; the scratch-queue task is never fetched.
    expect(snap.tasks).toHaveLength(2);
    expect(calls.map((c) => c.url)).toEqual([
      "http://broker.test:4801/health",
      "http://broker.test:4801/list",
      "http://broker.test:4801/list",
    ]);
    for (const c of calls) {
      expect(c.init?.headers?.["X-Agent-Token"]).toBe("tok-123");
    }
    const listBody = JSON.parse(calls[1].init?.body ?? "{}") as { rich?: boolean };
    expect(listBody.rich).toBe(true);
  });
  test("a broker error surfaces as a throw for the caller's fail-soft policy", async () => {
    const errFetch = async () => ({
      ok: false,
      status: 503,
      json: async () => ({ error: "backend down" }),
    });
    await expect(fetchQueueSnapshot("http://broker.test:4801", "tok", errFetch as never, () => NOW)).rejects.toThrow(
      "backend down",
    );
  });
});

describe("read-only by construction", () => {
  test("the collector module names no write verb", () => {
    const src = readFileSync(new URL("../src/queue.ts", import.meta.url), "utf8");
    for (const verb of ["/claim", "/complete", "/enqueue", "/park", "/block", "/fail", "/cancel", "/bind"]) {
      expect(src).not.toContain(`"${verb}"`);
      expect(src).not.toContain(`\`${verb}\``);
    }
    expect(src).toContain('"/health"');
    expect(src).toContain('"/list"');
  });
});

import { queuePanelHtml } from "../src/render.ts";

describe("queuePanelHtml (#9921)", () => {
  const fresh = projectQueueSnapshot(HEALTH, TASKS, NOW, NOW);
  test("fresh snapshot renders streams, counts, age, and claimed id items", () => {
    const html = queuePanelHtml(fresh);
    expect(html).toContain("Queue &amp; fleet");
    expect(html).toContain("repo-health-code-pilot");
    expect(html).toContain("30m"); // oldest queued age humanized
    expect(html).toContain("7 / 42"); // terminal 24h / 7d
    expect(html).toContain("#8580");
    expect(html).toContain("any-d");
    expect(html).toContain("2026-11-14T15:00:00.000Z"); // source timestamp
  });
  test("stale snapshot: the warning replaces the data", () => {
    const stale = projectQueueSnapshot(HEALTH, TASKS, NOW - QUEUE_STALE_MS - 1, NOW);
    const html = queuePanelHtml(stale);
    expect(html).toContain("Stale source");
    expect(html).not.toContain("#8580");
    expect(html).not.toContain("<table>");
  });
  test("unavailable snapshot: warning with the read error, no data", () => {
    const html = queuePanelHtml(unavailableQueueSnapshot(NOW, "broker down"));
    expect(html).toContain("Stale source");
    expect(html).toContain("broker down");
    expect(html).not.toContain("<table>");
  });
  test("null snapshot renders the unavailable banner", () => {
    const html = queuePanelHtml(null);
    expect(html).toContain("unavailable in the current source");
  });
});
