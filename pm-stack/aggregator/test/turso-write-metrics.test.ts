import { expect, test } from "bun:test";
import {
  createTursoActionsRemote,
  createTursoActivityRemote,
  createTursoCommentsRemote,
} from "../src/remote-stores.ts";
import { createTursoStoreClient } from "../src/turso-stores.ts";
import { TursoWriteMetrics } from "../src/turso-write-metrics.ts";

const context = { operation: "actions", candidates: 3 } as const;
const input = "insert into public.agg_actions(id) values ('fixture-marker')";
function success(metrics: Record<string, unknown> = {}) {
  return Response.json({
    results: [
      {
        type: "ok",
        response: {
          type: "execute",
          result: {
            cols: [{ name: "value" }],
            rows: [[{ type: "text", value: '"fixture-marker"' }]],
            ...metrics,
          },
        },
      },
      { type: "ok", response: { type: "close" } },
    ],
  });
}

test("model: only present safe counts contribute and zero is a known count", async () => {
  const metrics = new TursoWriteMetrics();
  const requests: unknown[] = [];
  const responses = [
    () => success({ affected_row_count: 0, rows_read: 7, rows_written: 0 }),
    () => success({ affected_row_count: 2, rows_written: 9 }),
    () => success({ affected_row_count: 4, rows_read: "5", rows_written: -1 }),
    () => success({ affected_row_count: 1.5, rows_read: 1e20, rows_written: null }),
    () => success(),
  ];
  const fetchFn = (async (_url, init) => {
    requests.push(JSON.parse(init!.body as string));
    return responses.shift()!();
  }) as typeof fetch;
  const client = createTursoStoreClient("https://example.test", "fixture-token", fetchFn, metrics);
  for (const _ of Array.from({ length: 5 })) {
    expect(await client.query(input, context)).toEqual([{ value: "fixture-marker" }]);
  }
  expect(requests).toHaveLength(5);
  expect(requests.every((req) => JSON.stringify(req) === JSON.stringify(requests[0]))).toBe(true);
  expect(metrics.snapshot().actions).toEqual({
    candidates: 15,
    affected: { knownBatches: 2, missingBatches: 3, rows: 2 },
    skipped: 4,
    read: { knownBatches: 1, missingBatches: 4, rows: 7 },
    written: { knownBatches: 2, missingBatches: 3, rows: 9 },
  });
  const copy = metrics.snapshot();
  copy.actions.read.rows = 999;
  expect(metrics.snapshot().actions.read.rows).toBe(7);
  expect(JSON.stringify(metrics.snapshot())).not.toContain("fixture");
});

test("transport oracle: instrumentation is inert for unlabelled, failed or disabled calls", async () => {
  const baseline = new TursoWriteMetrics().snapshot();
  const metrics = new TursoWriteMetrics();
  const requests: unknown[] = [];
  let response = () => success({ affected_row_count: 2, rows_written: 3 });
  const fetchFn = (async (_url, init) => {
    requests.push(JSON.parse(init!.body as string));
    return response();
  }) as typeof fetch;
  const a = createTursoStoreClient("https://example.test", "fixture-token", fetchFn);
  const b = createTursoStoreClient("https://example.test", "fixture-token", fetchFn, metrics);
  expect(await a.query(input, context)).toEqual(await b.query(input));
  expect(metrics.snapshot()).toEqual(baseline);
  response = () => new Response("", { status: 503 });
  await expect(b.query(input, context)).rejects.toThrow("HTTP 503");
  response = () =>
    Response.json({
      results: [{ type: "error", error: { message: "fixture-private-error" } }],
    });
  await expect(b.query(input, context)).rejects.toThrow("malformed result");
  response = () => success({ rows: [[]] });
  await expect(b.query(input, context)).rejects.toThrow("malformed Turso store row");
  expect(metrics.snapshot()).toEqual(baseline);
  expect(requests).toHaveLength(5);
  expect(requests.every((req) => JSON.stringify(req) === JSON.stringify(requests[0]))).toBe(true);

  class FailedDiagnostic extends TursoWriteMetrics {
    observe(): void {
      throw new Error("diagnostic failed");
    }
  }
  response = () => success({ affected_row_count: 2 });
  const c = createTursoStoreClient("https://example.test", "fixture-token", fetchFn, new FailedDiagnostic());
  expect(await c.query(input, context)).toEqual([{ value: "fixture-marker" }]);
  expect(requests).toHaveLength(6);
});

test("adapter oracle: only the four real UPSERTs label their exact chunk sizes", async () => {
  const metrics = new TursoWriteMetrics();
  let calls = 0;
  const client = createTursoStoreClient(
    "https://example.test",
    "fixture-token",
    (async (_url, _init) => {
      calls++;
      return success({ cols: [], rows: [], affected_row_count: 0 });
    }) as typeof fetch,
    metrics,
  );
  const get = async () => client;
  const t = "2026-11-23T00:00:00.000Z";
  const author = { kind: "drew", id: "fixture", label: "Fixture" } as const;
  await createTursoActionsRemote(get).saveAll(
    Array.from({ length: 201 }, (_, n) => ({
      id: String(n),
      clientToken: `fixture-${n}`,
      kind: "triage",
      verb: "acknowledge",
      objectRef: "repo#1",
      payload: {},
      author,
      createdAt: t,
      state: "new",
    })),
  );
  await createTursoActivityRemote(get).save({
    version: 1,
    checkpoints: {},
    seen: { github: ["fixture"], watchdog: [] },
    gaps: [
      {
        id: "fixture",
        source: "watchdog",
        from: t,
        to: t,
        status: "open",
        detectedAt: t,
      },
    ],
  });
  await createTursoCommentsRemote(get).saveAll([
    {
      id: "fixture",
      objectRef: "repo#1",
      kind: "message",
      author,
      authorVerified: true,
      body: "fixture",
      createdAt: t,
    },
  ]);
  expect(calls).toBe(5);
  const out = metrics.snapshot();
  expect(Object.keys(out)).toEqual(["activity.ingestion", "activity.gaps", "actions", "comments"]);
  expect(Object.values(out).map((v) => [v.candidates, v.affected.knownBatches, v.skipped])).toEqual([
    [2, 1, 2],
    [1, 1, 1],
    [201, 2, 201],
    [1, 1, 1],
  ]);
});
