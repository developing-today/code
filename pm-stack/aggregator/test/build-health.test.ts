// #9924: tests for the Build health panel's pure core — the Actions
// runs parser, the lane projection, and the nightly trend projection
// (including a failing nightly, whose link must survive).
import { describe, expect, test } from "bun:test";
import {
  BUILD_HEALTH_LANES,
  BUILD_HEALTH_NIGHTLY,
  buildHealthIsStale,
  fetchBuildHealthSnapshot,
  parseWorkflowRuns,
  projectLane,
  projectNightlyTrend,
  type ActionsRun,
  type LaneRuns,
} from "../src/build-health.ts";

const run = (id: number, createdAt: string, status: string, conclusion: string | null): ActionsRun => ({
  id,
  status,
  conclusion,
  createdAt,
  url: `https://github.com/developing-today/code/actions/runs/${id}`,
  event: "push",
});

describe("parseWorkflowRuns", () => {
  test("keeps well-formed runs and drops malformed rows", () => {
    const parsed = parseWorkflowRuns({
      workflow_runs: [
        {
          id: 1,
          status: "completed",
          conclusion: "success",
          created_at: "2026-11-14T00:00:00Z",
          html_url: "https://x/1",
          event: "push",
        },
        {
          id: "nope",
          status: "completed",
          conclusion: "success",
          created_at: "2026-11-14T00:00:00Z",
          html_url: "https://x/2",
          event: "push",
        },
        {
          id: 3,
          status: "queued",
          conclusion: null,
          created_at: "2026-11-14T01:00:00Z",
          html_url: "https://x/3",
          event: "push",
        },
        null,
      ],
    });
    expect(parsed.map((r) => r.id)).toEqual([1, 3]);
    expect(parsed[1].conclusion).toBeNull();
  });

  test("a response without workflow_runs throws", () => {
    expect(() => parseWorkflowRuns({})).toThrow();
    expect(() => parseWorkflowRuns(null)).toThrow();
  });
});

describe("projectLane", () => {
  const laneRuns: LaneRuns = {
    lane: BUILD_HEALTH_LANES[0],
    error: null,
    runs: [
      run(2, "2026-11-13T00:00:00Z", "completed", "failure"),
      run(3, "2026-11-14T00:00:00Z", "queued", null),
      run(1, "2026-11-12T00:00:00Z", "completed", "success"),
    ],
  };
  test("latest is the newest run whatever its status; history is completed-only, newest first", () => {
    const proj = projectLane(laneRuns);
    expect(proj.latest?.id).toBe(3);
    expect(proj.history.map((r) => r.id)).toEqual([2, 1]);
  });
  test("an empty lane projects to null latest and empty history", () => {
    const proj = projectLane({ lane: BUILD_HEALTH_LANES[0], runs: [], error: "github Actions HTTP 403" });
    expect(proj.latest).toBeNull();
    expect(proj.history).toEqual([]);
  });
});

describe("projectNightlyTrend — the failing-nightly case", () => {
  const nightly: LaneRuns = {
    lane: BUILD_HEALTH_NIGHTLY,
    error: null,
    runs: [
      run(10, "2026-11-10T15:00:00Z", "completed", "cancelled"),
      run(11, "2026-11-11T15:00:00Z", "completed", "success"),
      run(12, "2026-11-12T15:00:00Z", "completed", "failure"),
      run(13, "2026-11-13T15:00:00Z", "queued", null),
    ],
  };
  test("trend is completed-only, oldest first, and the failure keeps its link", () => {
    const trend = projectNightlyTrend(nightly);
    expect(trend.map((r) => r.id)).toEqual([10, 11, 12]);
    const failing = trend.find((r) => r.conclusion === "failure");
    expect(failing?.url).toBe("https://github.com/developing-today/code/actions/runs/12");
  });
});

describe("fetchBuildHealthSnapshot", () => {
  test("per-lane failures are recorded on the lane, not thrown", async () => {
    const fetchFn = (async (url: string) => {
      if (url.includes("fleet-rust-tests")) return new Response("no", { status: 500 });
      return new Response(
        JSON.stringify({
          workflow_runs: [
            {
              id: 7,
              status: "completed",
              conclusion: "success",
              created_at: "2026-11-14T00:00:00Z",
              html_url: "https://x/7",
              event: "schedule",
            },
          ],
        }),
        { status: 200 },
      );
    }) as unknown as typeof fetch;
    const snap = await fetchBuildHealthSnapshot("tok", fetchFn, () => new Date("2026-11-14T12:00:00Z"));
    expect(snap.fetchedAt).toBe("2026-11-14T12:00:00.000Z");
    const fleet = snap.lanes.find((l) => l.lane.file === "fleet-rust-tests.yml");
    expect(fleet?.error).toContain("HTTP 500");
    expect(fleet?.runs).toEqual([]);
    const id = snap.lanes.find((l) => l.lane.file === "id-check.yml");
    expect(id?.error).toBeNull();
    expect(id?.runs.map((r) => r.id)).toEqual([7]);
    expect(snap.nightly.runs.map((r) => r.id)).toEqual([7]);
  });
});

describe("buildHealthIsStale", () => {
  test("fresh within the window, stale past it, unparseable is stale", () => {
    const now = new Date("2026-11-14T12:10:00Z");
    const snap = (fetchedAt: string) => ({
      fetchedAt,
      lanes: [],
      nightly: { lane: BUILD_HEALTH_NIGHTLY, runs: [], error: null },
    });
    expect(buildHealthIsStale(snap("2026-11-14T12:00:00Z"), now)).toBe(false);
    expect(buildHealthIsStale(snap("2026-11-14T11:00:00Z"), now)).toBe(true);
    expect(buildHealthIsStale(snap("junk"), now)).toBe(true);
  });
});
