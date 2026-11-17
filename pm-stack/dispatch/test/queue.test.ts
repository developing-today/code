import { afterEach, describe, expect, test } from "bun:test";
import { ClaimLostError, QueueClient, QueueError } from "../src/queue.ts";

// #2902: ensureWorker memoized its registration promise with ??=,
// including when it rejected -- one transient failure wedged every
// later launch for the life of the process. The memo must reset on
// rejection (retry next call) and stay cached on success.

const realFetch = globalThis.fetch;

// Read-route lookups (issue #5555): a failed PostgREST lookup is an
// error, never a "not found" — getTask backs the task read route and
// findTaskBySessionId backs the session route's repo gate, so both
// fail closed with QueueError on a non-OK response and reserve null
// for a successful empty result.
describe("getTask / findTaskBySessionId failure mapping (#5555)", () => {
  test("getTask throws QueueError on a non-OK response", async () => {
    stubFetch(async () => new Response("unavailable", { status: 503 }));
    const client = new QueueClient("http://queue.test", "test-key");
    await expect(client.getTask(42)).rejects.toBeInstanceOf(QueueError);
  });

  test("getTask returns null only for a successful empty result", async () => {
    stubFetch(async () => Response.json([]));
    const client = new QueueClient("http://queue.test", "test-key");
    expect(await client.getTask(42)).toBeNull();
  });

  test("getTask returns the row on success", async () => {
    stubFetch(async () => Response.json([{ id: 42, queue: "q", status: "queued", dedupe_key: "k" }]));
    const client = new QueueClient("http://queue.test", "test-key");
    expect((await client.getTask(42))?.id).toBe(42);
  });

  test("findTaskBySessionId throws QueueError on a non-OK response (gate fails closed)", async () => {
    stubFetch(async () => new Response("boom", { status: 500 }));
    const client = new QueueClient("http://queue.test", "test-key");
    await expect(client.findTaskBySessionId("ses_1")).rejects.toBeInstanceOf(QueueError);
  });

  test("findTaskBySessionId returns the launching task on success", async () => {
    stubFetch(async () => Response.json([{ id: 7, queue: "q", status: "claimed", dedupe_key: "k" }]));
    const client = new QueueClient("http://queue.test", "test-key");
    expect((await client.findTaskBySessionId("ses_1"))?.id).toBe(7);
  });

  test("findTaskBySessionId falls back to the cleared binding the queue preserves on the task row", async () => {
    const urls: string[] = [];
    globalThis.fetch = (async (...args: unknown[]) => {
      urls.push(String(args[0]));
      if (urls.length === 1) return Response.json([]);
      return Response.json([{ id: 9, queue: "q", status: "parked", dedupe_key: "k" }]);
    }) as unknown as typeof fetch;
    const client = new QueueClient("http://queue.test", "test-key");
    expect((await client.findTaskBySessionId("ses_1"))?.id).toBe(9);
    expect(urls[0]).toContain("progress->>session_id=eq.ses_1");
    expect(urls[1]).toContain("progress->session_cleared->>session_id=eq.ses_1");
  });

  test("findTaskBySessionId throws QueueError when the cleared-binding lookup fails", async () => {
    stubFetch(async (n) => (n === 1 ? Response.json([]) : new Response("boom", { status: 500 })));
    const client = new QueueClient("http://queue.test", "test-key");
    await expect(client.findTaskBySessionId("ses_1")).rejects.toBeInstanceOf(QueueError);
  });
});

// Row-body shape validation (#7299): a PostgREST 200 body is
// shape-untrusted in all three row readers. Null, scalar, object,
// and non-JSON bodies must land in the module's own vocabulary —
// QueueError for patchTask and getTask, null ("unknown") only for
// the degrade-by-contract dedupe lookup —
// never a raw TypeError/SyntaxError and never a phantom row.
describe("row body shape validation (#7299)", () => {
  const ROW = { id: 7, queue: "q", status: "queued", dedupe_key: "k" };
  const badBodies: Array<[string, () => Response]> = [
    ["null", () => new Response("null")],
    ["string", () => Response.json("x")],
    ["object", () => Response.json(ROW)],
    ["non-JSON", () => new Response("not json{")],
    ["array of non-object", () => Response.json(["x"])],
  ];

  for (const [name, make] of badBodies) {
    test(`patchTask throws QueueError on a ${name} body`, async () => {
      stubFetch(async () => make());
      const client = new QueueClient("http://queue.test", "test-key");
      await expect(client.patchTask(7, { status: "claimed" })).rejects.toBeInstanceOf(QueueError);
    });

    // A malformed body is a lookup failure, not a missing task:
    // null is reserved for a successful empty array (Codex P2 on
    // PR #8706 — the read route turns null into a definitive 404).
    test(`getTask throws QueueError on a ${name} body`, async () => {
      stubFetch(async () => make());
      const client = new QueueClient("http://queue.test", "test-key");
      await expect(client.getTask(7)).rejects.toBeInstanceOf(QueueError);
    });

    test(`findByDedupeKey resolves null on a ${name} body`, async () => {
      stubFetch(async () => make());
      const client = new QueueClient("http://queue.test", "test-key");
      expect(await client.findByDedupeKey("k")).toBeNull();
    });
  }

  test("the control array body resolves the row in all three methods", async () => {
    const client = new QueueClient("http://queue.test", "test-key");
    stubFetch(async () => Response.json([ROW]));
    expect((await client.patchTask(7, { status: "claimed" }))?.id).toBe(7);
    stubFetch(async () => Response.json([ROW]));
    expect((await client.getTask(7))?.id).toBe(7);
    stubFetch(async () => Response.json([ROW]));
    expect((await client.findByDedupeKey("k"))?.id).toBe(7);
  });

  test("findTaskBySessionId fails closed with QueueError on a malformed body", async () => {
    stubFetch(async () => new Response("not json{"));
    const client = new QueueClient("http://queue.test", "test-key");
    await expect(client.findTaskBySessionId("ses_1")).rejects.toBeInstanceOf(QueueError);
  });
});

afterEach(() => {
  globalThis.fetch = realFetch;
});

function stubFetch(handler: (call: number) => Promise<Response>) {
  let calls = 0;
  globalThis.fetch = (async () => handler(++calls)) as unknown as typeof fetch;
  return () => calls;
}

describe("ensureWorker registration memo", () => {
  test("first attempt fails, second succeeds, later calls reuse the success", async () => {
    const calls = stubFetch(async (n) =>
      n === 1 ? new Response("postgrest unavailable", { status: 503 }) : new Response(null, { status: 201 }),
    );
    const client = new QueueClient("http://queue.test", "test-key");

    await expect(client.ensureWorker()).rejects.toBeInstanceOf(QueueError);
    expect(calls()).toBe(1);

    // The rejection was not cached: the next call retries the upsert.
    await client.ensureWorker();
    expect(calls()).toBe(2);

    // The success IS cached: later calls do not re-fetch.
    await client.ensureWorker();
    await client.ensureWorker();
    expect(calls()).toBe(2);
  });

  test("a fetch throw also resets the memo", async () => {
    const calls = stubFetch(async (n) => {
      if (n === 1) throw new Error("network blip");
      return new Response(null, { status: 201 });
    });
    const client = new QueueClient("http://queue.test", "test-key");

    await expect(client.ensureWorker()).rejects.toThrow("network blip");
    await client.ensureWorker();
    expect(calls()).toBe(2);
  });

  test("concurrent callers share one attempt and one retry", async () => {
    const calls = stubFetch(async (n) =>
      n === 1 ? new Response("boom", { status: 500 }) : new Response(null, { status: 201 }),
    );
    const client = new QueueClient("http://queue.test", "test-key");

    const [a, b] = await Promise.allSettled([client.ensureWorker(), client.ensureWorker()]);
    expect(a.status).toBe("rejected");
    expect(b.status).toBe("rejected");
    expect(calls()).toBe(1);

    await Promise.all([client.ensureWorker(), client.ensureWorker()]);
    expect(calls()).toBe(2);
  });
});

// #10737: the dispatch launch assigns its row with a compare-and-swap
// on status, so a fleet worker that claimed the task first keeps it.
describe("patchTask onlyIfStatus (#10737)", () => {
  const ROW = { id: 7, queue: "q", status: "claimed", dedupe_key: "k" };

  test("filters the PATCH on the required status and returns the row", async () => {
    let url = "";
    globalThis.fetch = (async (input: RequestInfo | URL) => {
      url = String(input);
      return Response.json([ROW]);
    }) as unknown as typeof fetch;
    const client = new QueueClient("http://queue.test", "test-key");
    const row = await client.patchTask(7, { status: "claimed" }, { onlyIfStatus: "queued" });
    expect(row.id).toBe(7);
    expect(url).toBe("http://queue.test/rest/v1/tasks?id=eq.7&status=eq.queued");
  });

  test("a row that is no longer queued throws ClaimLostError", async () => {
    stubFetch(async () => Response.json([]));
    const client = new QueueClient("http://queue.test", "test-key");
    const err = await client.patchTask(7, { status: "claimed" }, { onlyIfStatus: "queued" }).catch((e) => e);
    expect(err).toBeInstanceOf(ClaimLostError);
    expect(err).toBeInstanceOf(QueueError);
    expect((err as ClaimLostError).httpStatus).toBe(409);
  });

  test("an unconditional PATCH that matches nothing stays a plain QueueError", async () => {
    stubFetch(async () => Response.json([]));
    const client = new QueueClient("http://queue.test", "test-key");
    const err = await client.patchTask(7, { last_error: "x" }).catch((e) => e);
    expect(err).toBeInstanceOf(QueueError);
    expect(err).not.toBeInstanceOf(ClaimLostError);
  });
});
