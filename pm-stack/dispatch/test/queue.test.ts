import { afterEach, describe, expect, test } from "bun:test";
import { ClaimLostError, QueueClient, QueueError } from "../src/queue.ts";
import type { Packet } from "../src/packet.ts";

// #9028: the queue client talks to the fleet queue broker, not
// PostgREST. These tests stub fetch at the broker boundary: every
// call is a POST to a broker verb path with the X-Agent-Token header,
// and every answer is a broker envelope ({ok, row|rows, error}).

const origFetch = globalThis.fetch;
afterEach(() => {
  globalThis.fetch = origFetch;
});

interface SeenCall {
  url: string;
  body: Record<string, unknown>;
  token: string | null;
}

function stubBroker(handler: (path: string, body: Record<string, unknown>) => unknown) {
  const calls: SeenCall[] = [];
  globalThis.fetch = (async (input: unknown, init?: { headers?: Record<string, string>; body?: string }) => {
    const url = String(input);
    const body = JSON.parse(init?.body ?? "{}") as Record<string, unknown>;
    calls.push({ url, body, token: init?.headers?.["X-Agent-Token"] ?? null });
    const path = new URL(url).pathname;
    return Response.json(handler(path, body));
  }) as typeof fetch;
  return calls;
}

const packet = { title: "t", body: "b", priority: 30, dedupe_key: "k", payload: { schema: "repo-task/v1" } } as unknown as Packet;
const row7 = { id: 7, queue: "repo-health-code-pilot", status: "queued", dedupe_key: "k" };

describe("broker enqueue", () => {
  test("posts the packet to /enqueue with the mapped queue and the broker token", async () => {
    const calls = stubBroker((path) => {
      expect(path).toBe("/enqueue");
      return { ok: true, row: row7 };
    });
    const client = new QueueClient("http://broker.test", "tok-1");
    const row = await client.enqueue(packet, "developing-today/code");
    expect(row.id).toBe(7);
    expect(calls[0].body.queue).toBe("repo-health-code-pilot");
    expect((calls[0].body.packet as Packet).dedupe_key).toBe("k");
    expect(calls[0].token).toBe("tok-1");
  });

  test("a broker validation refusal is a QueueError carrying its message", async () => {
    stubBroker(() => ({ ok: false, exit: 2, error: "packet validation failed (1 error(s))" }));
    const client = new QueueClient("http://broker.test", "tok-1");
    await expect(client.enqueue(packet, "developing-today/code")).rejects.toThrow("packet validation failed");
  });

  test("an envelope whose row is not a task row is a QueueError, never a phantom row", async () => {
    stubBroker(() => ({ ok: true, row: "nope" }));
    const client = new QueueClient("http://broker.test", "tok-1");
    await expect(client.enqueue(packet, "developing-today/code")).rejects.toBeInstanceOf(QueueError);
  });

  test("an unmapped repo is refused before any broker call", async () => {
    const calls = stubBroker(() => ({ ok: true, row: row7 }));
    const client = new QueueClient("http://broker.test", "tok-1");
    await expect(client.enqueue(packet, "nobody/nothing")).rejects.toThrow("no queue mapped");
    expect(calls).toHaveLength(0);
  });
});

describe("broker claimTask (claim-flow, #10737 + #9028)", () => {
  test("claims the exact enqueued row and binds the session in one call", async () => {
    const calls = stubBroker((path) => {
      expect(path).toBe("/claim-flow");
      return { ok: true, row: { ...row7, status: "claimed" } };
    });
    const client = new QueueClient("http://broker.test", "tok-1");
    const row = await client.claimTask(row7 as never, "ses_1", "tester", 3600);
    expect(row.status).toBe("claimed");
    expect(calls[0].body).toMatchObject({
      task_id: 7,
      worker: "dispatch-opencode",
      queue: "repo-health-code-pilot",
      session_id: "ses_1",
      launched_by: "tester",
      lease_seconds: 3600,
    });
  });

  test("a null row (another worker claimed it first) is ClaimLostError", async () => {
    stubBroker(() => ({ ok: true, row: null }));
    const client = new QueueClient("http://broker.test", "tok-1");
    const err = await client.claimTask(row7 as never, "ses_1", "tester", 3600).catch((e) => e);
    expect(err).toBeInstanceOf(ClaimLostError);
    expect((err as ClaimLostError).taskId).toBe(7);
  });

  test("a bind-step partial throws — the session binding is the point of the claim", async () => {
    stubBroker(() => ({ ok: true, row: { ...row7, status: "claimed" }, claim_flow_partial: { step: "bind", error: "no lane ledger" } }));
    const client = new QueueClient("http://broker.test", "tok-1");
    await expect(client.claimTask(row7 as never, "ses_1", "tester", 3600)).rejects.toThrow("bind failed");
  });

  test("a lane-step partial is noted on the task and the claim still returns", async () => {
    const calls = stubBroker((path) => {
      if (path === "/claim-flow") {
        return { ok: true, row: { ...row7, status: "claimed" }, claim_flow_partial: { step: "lane", error: "lane rpc 500" } };
      }
      return { ok: true, row: { ...row7, status: "claimed" } };
    });
    const client = new QueueClient("http://broker.test", "tok-1");
    const row = await client.claimTask(row7 as never, "ses_1", "tester", 3600);
    expect(row.status).toBe("claimed");
    const note = calls.find((c) => c.url.endsWith("/checkpoint"));
    expect(note).toBeTruthy();
    expect((note!.body.progress as { last_error: string }).last_error).toContain("lane rpc 500");
  });
});

describe("broker reads", () => {
  test("getTask returns the row, and null only for a successful empty result", async () => {
    stubBroker(() => ({ ok: true, rows: [row7], total: 1 }));
    const client = new QueueClient("http://broker.test", "tok-1");
    expect((await client.getTask(7))?.id).toBe(7);
    stubBroker(() => ({ ok: true, rows: [], total: 0 }));
    expect(await client.getTask(7)).toBeNull();
  });

  test("getTask throws QueueError on a broker refusal — a failed lookup is not a missing task", async () => {
    stubBroker(() => ({ ok: false, error: "broker down" }));
    const client = new QueueClient("http://broker.test", "tok-1");
    await expect(client.getTask(7)).rejects.toBeInstanceOf(QueueError);
  });

  test("findByDedupeKey finds the row and degrades to null on a broker failure (#4661)", async () => {
    stubBroker(() => ({ ok: true, rows: [row7], total: 1 }));
    const client = new QueueClient("http://broker.test", "tok-1");
    expect((await client.findByDedupeKey("k"))?.id).toBe(7);
    stubBroker(() => ({ ok: false, error: "broker down" }));
    expect(await client.findByDedupeKey("k")).toBeNull();
  });

  test("findTaskBySessionId matches the live binding, then the cleared record (#5555)", async () => {
    const live = { ...row7, id: 8, status: "claimed", progress: { session_id: "ses_live" } };
    const cleared = { ...row7, id: 9, status: "done", progress: { session_cleared: { session_id: "ses_old" } } };
    stubBroker((_path, body) => {
      const rows = body.status === "claimed" ? [live] : body.status === "done" ? [cleared] : [];
      return { ok: true, rows, total: rows.length };
    });
    const client = new QueueClient("http://broker.test", "tok-1");
    expect((await client.findTaskBySessionId("ses_live"))?.id).toBe(8);
    expect((await client.findTaskBySessionId("ses_old"))?.id).toBe(9);
    expect(await client.findTaskBySessionId("ses_nope")).toBeNull();
  });

  test("findTaskBySessionId fails closed on a broker refusal (it backs the repo gate)", async () => {
    stubBroker(() => ({ ok: false, error: "broker down" }));
    const client = new QueueClient("http://broker.test", "tok-1");
    await expect(client.findTaskBySessionId("ses_x")).rejects.toBeInstanceOf(QueueError);
  });
});

describe("broker notes and cancels", () => {
  test("noteTask records the note as a checkpoint progress merge", async () => {
    const calls = stubBroker(() => ({ ok: true, row: { ...row7, status: "claimed" } }));
    const client = new QueueClient("http://broker.test", "tok-1");
    await client.noteTask(7, "lane assignment failed (launch unaffected): x");
    expect(calls[0].url).toContain("/checkpoint");
    expect(calls[0].body).toMatchObject({ task_id: 7, worker: "dispatch-opencode", stage: "dispatch_note" });
    expect((calls[0].body.progress as { dispatch_note: string }).dispatch_note).toContain("lane assignment failed");
  });

  test("cancelTask posts /cancel with the reason", async () => {
    const calls = stubBroker(() => ({ ok: true, row: { ...row7, status: "cancelled" } }));
    const client = new QueueClient("http://broker.test", "tok-1");
    await client.cancelTask(7, "dispatch launch failed: boom");
    expect(calls[0].url).toContain("/cancel");
    expect(calls[0].body).toMatchObject({ task_id: 7, reason: "dispatch launch failed: boom" });
  });

  test("a transport failure is a QueueError, not a silent success", async () => {
    globalThis.fetch = (async () => {
      throw new Error("connect refused");
    }) as unknown as typeof fetch;
    const client = new QueueClient("http://broker.test", "tok-1");
    await expect(client.getTask(7)).rejects.toBeInstanceOf(QueueError);
  });
});
