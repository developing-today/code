import { afterEach, describe, expect, it, vi } from "vitest";
import { ageSweep, setDrainDue, signalOnAppend } from "../src/drain";
import { Turso } from "../src/turso";
import type { SinkEnv } from "../src/sink";

const env: SinkEnv = {
  TURSO_QUEUE_URL: "https://fleet-queue.example.invalid",
  TURSO_QUEUE_TOKEN: "q-token",
};

const NOW = new Date("2026-11-26T12:00:00Z");

interface FakeState {
  watermarks: Map<string, string>;
  depth: Map<string, number>;
  oldest: Map<string, string | null>;
  inserts: { key: string; value: string }[];
}

/**
 * A fake fleet-queue store behind the Turso pipeline fetch shape:
 * watermark reads/writes, per-kind depth counts and oldest reads
 * are served from the maps; every watermark INSERT is captured.
 */
function fakeStore(state: FakeState) {
  return async (_url: string, init: RequestInit) => {
    const body = JSON.parse(String(init.body)) as {
      requests: { stmt: { sql: string; args: { type: string; value?: string }[] } }[];
    };
    const stmt = body.requests[0].stmt;
    const arg = (i: number) => stmt.args?.[i]?.value;
    let rows: unknown[] = [];
    if (stmt.sql.includes("SELECT value FROM agent_watermarks")) {
      const v = state.watermarks.get(String(arg(0)));
      rows = v === undefined ? [] : [[{ type: "text", value: v }]];
    } else if (stmt.sql.includes("SELECT COUNT(*)")) {
      rows = [[{ type: "integer", value: String(state.depth.get(String(arg(0))) ?? 0) }]];
    } else if (stmt.sql.includes("SELECT MIN(created_at)")) {
      const v = state.oldest.get(String(arg(0)));
      rows = [[v ? { type: "text", value: v } : { type: "null" }]];
    } else if (stmt.sql.startsWith("INSERT INTO agent_watermarks")) {
      state.inserts.push({ key: String(arg(0)), value: String(arg(1)) });
      if (!state.watermarks.has(String(arg(0)))) state.watermarks.set(String(arg(0)), String(arg(1)));
    }
    return new Response(JSON.stringify({ results: [{ type: "ok", response: { result: { rows } } }, { type: "ok" }] }), {
      status: 200,
      headers: { "Content-Type": "application/json" },
    });
  };
}

function freshState(): FakeState {
  return { watermarks: new Map(), depth: new Map(), oldest: new Map(), inserts: [] };
}

afterEach(() => vi.unstubAllGlobals());

// Oracles are design D4: the count leg sets drain-due at the count
// threshold; the age leg sets it at the age threshold; a standing
// signal is never moved; a log-only kind has no threshold and never
// signals.
describe("signalOnAppend (count leg, D4.1)", () => {
  it("sets drain-due when depth reaches the count threshold", async () => {
    const state = freshState();
    state.depth.set("issue_new", 10);
    vi.stubGlobal("fetch", fakeStore(state));
    const out = await signalOnAppend(env, "issue_new", NOW);
    expect(out).toEqual({ kind: "issue_new", depth: 10, signaled: true });
    expect(state.watermarks.get("drain-due:issue_new")).toBe(NOW.toISOString());
  });

  it("does not signal below the count threshold", async () => {
    const state = freshState();
    state.depth.set("issue_new", 9);
    vi.stubGlobal("fetch", fakeStore(state));
    const out = await signalOnAppend(env, "issue_new", NOW);
    expect(out.signaled).toBe(false);
    expect(state.inserts).toHaveLength(0);
  });

  it("ci_failure signals on the first item (count threshold 1)", async () => {
    const state = freshState();
    state.depth.set("ci_failure", 1);
    vi.stubGlobal("fetch", fakeStore(state));
    const out = await signalOnAppend(env, "ci_failure", NOW);
    expect(out.signaled).toBe(true);
  });

  it("a standing signal is never moved", async () => {
    const state = freshState();
    state.depth.set("issue_new", 25);
    state.watermarks.set("drain-due:issue_new", "2026-11-26T11:00:00.000Z");
    vi.stubGlobal("fetch", fakeStore(state));
    const out = await signalOnAppend(env, "issue_new", NOW);
    expect(out.signaled).toBe(false);
    expect(state.watermarks.get("drain-due:issue_new")).toBe("2026-11-26T11:00:00.000Z");
    expect(state.inserts).toHaveLength(0);
  });

  it("a log-only kind never signals", async () => {
    const state = freshState();
    state.depth.set("push", 1000);
    vi.stubGlobal("fetch", fakeStore(state));
    const out = await signalOnAppend(env, "push", NOW);
    expect(out).toEqual({ kind: "push", depth: 0, signaled: false });
    expect(state.inserts).toHaveLength(0);
  });
});

describe("ageSweep (age leg, D4.2)", () => {
  it("sets drain-due for kinds past their age threshold, and only those", async () => {
    const state = freshState();
    state.oldest.set("issue_new", "2026-11-26T11:40:00Z"); // 20 min old, threshold 15
    state.oldest.set("pr_open", "2026-11-26T11:57:00Z"); // 3 min old, threshold 15
    state.oldest.set("ci_failure", "2026-11-26T11:54:00Z"); // 6 min old, threshold 5
    state.oldest.set("mention", null); // empty queue
    state.oldest.set("pr_review", "2026-11-26T11:44:00Z"); // 16 min old, threshold 15
    vi.stubGlobal("fetch", fakeStore(state));
    const out = await ageSweep(env, NOW);
    const byKind = Object.fromEntries(out.map((r) => [r.kind, r.signaled]));
    expect(byKind).toEqual({
      issue_new: true,
      pr_open: false,
      ci_failure: true,
      mention: false,
      pr_review: true,
    });
    expect(state.watermarks.get("drain-due:issue_new")).toBe(NOW.toISOString());
    expect(state.watermarks.has("drain-due:pr_open")).toBe(false);
  });
});

describe("setDrainDue", () => {
  it("sets once and reports the second call as already set", async () => {
    const state = freshState();
    vi.stubGlobal("fetch", fakeStore(state));
    const q = new Turso(env.TURSO_QUEUE_URL, env.TURSO_QUEUE_TOKEN);
    await expect(setDrainDue(q, "mention", NOW.toISOString())).resolves.toBe(true);
    await expect(setDrainDue(q, "mention", "2026-11-26T12:05:00.000Z")).resolves.toBe(false);
    expect(state.watermarks.get("drain-due:mention")).toBe(NOW.toISOString());
  });
});
