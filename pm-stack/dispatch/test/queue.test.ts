import { afterEach, describe, expect, test } from "bun:test";
import { QueueClient, QueueError } from "../src/queue.ts";

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
});

afterEach(() => {
  globalThis.fetch = realFetch;
});

function stubFetch(handler: (call: number) => Promise<Response>) {
  let calls = 0;
  globalThis.fetch = (async () => handler(++calls)) as typeof fetch;
  return () => calls;
}

describe("ensureWorker registration memo", () => {
  test("first attempt fails, second succeeds, later calls reuse the success", async () => {
    const calls = stubFetch(async (n) =>
      n === 1
        ? new Response("postgrest unavailable", { status: 503 })
        : new Response(null, { status: 201 }),
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
      n === 1
        ? new Response("boom", { status: 500 })
        : new Response(null, { status: 201 }),
    );
    const client = new QueueClient("http://queue.test", "test-key");

    const [a, b] = await Promise.allSettled([
      client.ensureWorker(),
      client.ensureWorker(),
    ]);
    expect(a.status).toBe("rejected");
    expect(b.status).toBe("rejected");
    expect(calls()).toBe(1);

    await Promise.all([client.ensureWorker(), client.ensureWorker()]);
    expect(calls()).toBe(2);
  });
});
