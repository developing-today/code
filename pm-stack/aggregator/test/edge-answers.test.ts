// #13469 (notifications Phase 2 step 3) — the laptop half of the edge
// answer intake:
//  - a needs-Drew candidate mirrors what the edge needs to answer it
//    (every answerable option, the token hash, never the token), under
//    a new mirror id so an older mirror is re-sent once;
//  - a question that stops candidating is withdrawn at the edge with
//    one cleared event; a new state of the same question is not;
//  - a heartbeat reporting pending answers drains them in the same
//    contact: each is applied once and acked; one whose application
//    throws stays at the edge for the next contact;
//  - applying a drained answer goes through the laptop's own answer
//    path: token hash checked, first answer wins, side effects run.
import { afterEach, describe, expect, test } from "bun:test";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { applyEdgeAnswer, tokenHashOf, type EdgeAnswer } from "../src/needs-drew-answer.ts";
import { getNeedsDrew, raiseNeedsDrew, recordAnswer } from "../src/needs-drew.ts";
import { handleDecisionRoutes } from "../src/server-decision-routes.ts";
import type { NotificationCandidate } from "../src/notifications.ts";
import { NotifyRelayClient, notificationMirrorEvent } from "../src/notify-relay.ts";

interface RecordedCall {
  url: string;
  method: string;
  body: Record<string, unknown> | null;
}

const dirs: string[] = [];
const clients: NotifyRelayClient[] = [];
afterEach(async () => {
  for (const client of clients.splice(0)) client.stop();
  for (const dir of dirs.splice(0)) await rm(dir, { recursive: true, force: true });
});

async function tempDir(): Promise<string> {
  const dir = await mkdtemp(join(tmpdir(), "edge-answers-test-"));
  dirs.push(dir);
  return dir;
}

async function makeClient(
  handler: (call: RecordedCall) => { status: number; body?: unknown },
  applyAnswer?: (answer: EdgeAnswer) => Promise<"applied" | "duplicate" | "unknown" | "rejected">,
) {
  const dir = await tempDir();
  const calls: RecordedCall[] = [];
  const client = new NotifyRelayClient({
    url: "https://pm-notify.example.workers.dev",
    stateFile: join(dir, "notify-relay.json"),
    getStore: async () => ({ listSubscriptions: async () => [] }),
    resolveSecret: async () => "test-secret",
    fetchImpl: (async (url: string, init: RequestInit = {}) => {
      const call: RecordedCall = {
        url,
        method: init.method ?? "GET",
        body: typeof init.body === "string" ? JSON.parse(init.body) : null,
      };
      calls.push(call);
      const out = handler(call);
      return {
        status: out.status,
        ok: out.status >= 200 && out.status < 300,
        json: async () => out.body ?? {},
      } as Response;
    }) as typeof fetch,
    mintEpoch: () => "epoch-test",
    now: () => Date.parse("2026-11-25T12:00:00Z"),
    ...(applyAnswer ? { applyAnswer } : {}),
  });
  clients.push(client);
  return { client, calls };
}

const needsDrewCandidate = (
  over: Partial<NotificationCandidate["needsDrew"]> = {},
  raisedAt = "2026-11-25T11:00:00Z",
) =>
  ({
    class: "needs-user",
    objectRef: "needs-drew:q-1",
    stateFingerprint: JSON.stringify({ raisedAt }),
    title: "Ship it?",
    body: "Context",
    url: "/answer/q-1",
    observedAt: raisedAt,
    breakthrough: true,
    needsDrew: {
      ledgerId: "q-1",
      question: "Ship it?",
      options: [
        { id: "yes", label: "Yes" },
        { id: "no", label: "No" },
      ],
      answerOptions: [
        { id: "yes", label: "Yes" },
        { id: "no", label: "No" },
        { id: "send-back", label: "Send back" },
      ],
      answerType: "radio",
      links: [],
      tokenHash: tokenHashOf("token-1"),
      ...over,
    },
  }) as NotificationCandidate;

test("the new needs-user answer URL uses the same token gate as the legacy URL", async () => {
  const ledgerId = `compat-probe-${Date.now()}`;
  const path = `/api/needs-user/${ledgerId}/answer`;
  const request = new Request(`https://pm.example${path}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ token: "invalid-token", optionId: "yes" }),
  });
  const response = await handleDecisionRoutes(request, new URL(request.url), path);
  expect(response?.status).toBe(403);
});

const answer = (over: Partial<EdgeAnswer> = {}): EdgeAnswer => ({
  answerId: "a-1",
  ledgerId: "q-1",
  optionId: "yes",
  answeredAt: "2026-11-25T11:30:00.000Z",
  answerTokenHash: tokenHashOf("token-1"),
  ...over,
});

describe("mirroring an answerable question", () => {
  test("the payload carries the token hash and answer options, never the token, under a new id", () => {
    const answerable = notificationMirrorEvent(needsDrewCandidate(), "occ-1");
    const legacy = notificationMirrorEvent(needsDrewCandidate({ tokenHash: undefined }), "occ-1");
    expect(answerable.mirrorId).not.toBe(legacy.mirrorId);
    expect(JSON.stringify(answerable)).not.toContain("token-1");
    expect(answerable.payload.needsDrew).toMatchObject({
      tokenHash: tokenHashOf("token-1"),
      answerOptions: [{ id: "yes" }, { id: "no" }, { id: "send-back" }],
    });
  });

  test("a question that stops candidating is withdrawn once; a new state of it is not a withdrawal", async () => {
    const { client, calls } = await makeClient((call) =>
      call.url.endsWith("/heartbeat") ? { status: 200, body: { pendingAnswers: 0 } } : { status: 200 },
    );
    await client.mirrorCandidates([needsDrewCandidate()]);
    await client.mirrorCandidates([needsDrewCandidate({}, "2026-11-25T11:05:00Z")]);
    await client.beat();
    let events = calls.filter((c) => c.url.endsWith("/mirror/event")).map((c) => c.body!);
    expect(events.map((e) => e.transition)).toEqual(["raised", "raised"]);

    await client.mirrorCandidates([]);
    await client.mirrorCandidates([]);
    await client.beat();
    events = calls.filter((c) => c.url.endsWith("/mirror/event")).map((c) => c.body!);
    expect(events.map((e) => e.transition)).toEqual(["raised", "raised", "cleared"]);
    expect(events[2]).toMatchObject({ objectRef: "needs-drew:q-1", payload: { class: "needs-user" } });
  });

  test("a quota wall that recovers is not withdrawn (only needs-Drew questions are)", async () => {
    const { client, calls } = await makeClient((call) =>
      call.url.endsWith("/heartbeat") ? { status: 200, body: { pendingAnswers: 0 } } : { status: 200 },
    );
    const wall = {
      class: "quota-wall",
      objectRef: "quota:pool-a",
      stateFingerprint: "walled",
      title: "Quota wall",
      body: "",
      url: "/quota",
      observedAt: "2026-11-25T11:00:00Z",
      breakthrough: true,
    } as NotificationCandidate;
    await client.mirrorCandidates([wall]);
    await client.mirrorCandidates([]);
    await client.beat();
    const transitions = calls.filter((c) => c.url.endsWith("/mirror/event")).map((c) => c.body!.transition);
    expect(transitions).toEqual(["raised"]);
  });
});

describe("the drain", () => {
  const pending = [
    answer(),
    answer({ answerId: "a-2", ledgerId: "q-2" }),
    answer({ answerId: "a-3", ledgerId: "q-3" }),
  ];

  test("a heartbeat with pending answers drains them in the same contact; failures stay at the edge", async () => {
    const applied: string[] = [];
    const { client, calls } = await makeClient(
      (call) => {
        if (call.url.endsWith("/heartbeat")) return { status: 200, body: { pendingAnswers: 3 } };
        if (call.url.endsWith("/answers")) return { status: 200, body: { ok: true, answers: pending } };
        if (call.url.endsWith("/answers/ack")) return { status: 200, body: { ok: true, pendingAnswers: 1 } };
        return { status: 200 };
      },
      async (a) => {
        if (a.ledgerId === "q-3") throw new Error("ledger unreadable");
        applied.push(a.answerId);
        return a.ledgerId === "q-2" ? "duplicate" : "applied";
      },
    );
    await client.beat();
    expect(applied).toEqual(["a-1", "a-2"]);
    const ack = calls.find((c) => c.url.endsWith("/answers/ack"));
    expect(ack?.body).toEqual({ answerIds: ["a-1", "a-2"] });
    expect(client.lastDrain).toMatchObject({ applied: 1, duplicate: 1, failed: 1, acked: 2 });
    expect(client.lastPendingAnswers).toBe(1);
    const order = calls.map((c) => new URL(c.url).pathname);
    expect(order.indexOf("/answers")).toBeGreaterThan(order.indexOf("/heartbeat"));
  });

  test("no pending answers: no drain call", async () => {
    const { client, calls } = await makeClient(
      (call) => (call.url.endsWith("/heartbeat") ? { status: 200, body: { pendingAnswers: 0 } } : { status: 200 }),
      async () => "applied",
    );
    await client.beat();
    expect(calls.some((c) => c.url.endsWith("/answers"))).toBe(false);
  });

  test("a failed drain is fail-soft and acks nothing", async () => {
    const { client, calls } = await makeClient(
      (call) => {
        if (call.url.endsWith("/heartbeat")) return { status: 200, body: { pendingAnswers: 2 } };
        if (call.url.endsWith("/answers")) return { status: 503 };
        return { status: 200 };
      },
      async () => "applied",
    );
    await client.beat();
    expect(calls.some((c) => c.url.endsWith("/answers/ack"))).toBe(false);
    expect(client.lastError).toContain("answers GET failed");
  });
});

describe("applyEdgeAnswer", () => {
  async function ledger() {
    const file = join(await tempDir(), "needs-drew.json");
    const item = await raiseNeedsDrew(file, {
      ledgerId: "q-1",
      question: "Ship it?",
      options: [
        { id: "yes", label: "Yes" },
        { id: "no", label: "No" },
      ],
      links: [],
    });
    return { file, item };
  }

  test("an edge answer lands in the ledger once, via edge, with its side effects", async () => {
    const { file, item } = await ledger();
    const after: string[] = [];
    const effects = async (q: { ledgerId: string }) => void after.push(q.ledgerId);
    const hash = tokenHashOf(item.token);
    expect(await applyEdgeAnswer(file, answer({ answerTokenHash: hash }), effects)).toBe("applied");
    expect(await applyEdgeAnswer(file, answer({ answerTokenHash: hash, optionId: "no" }), effects)).toBe("duplicate");
    const stored = await getNeedsDrew(file, "q-1");
    expect(stored?.answer).toMatchObject({ optionId: "yes", via: "edge" });
    expect(after).toEqual(["q-1"]);
  });

  test("an answer the laptop already took wins; the edge answer is a duplicate", async () => {
    const { file, item } = await ledger();
    await recordAnswer(file, "q-1", { optionId: "no", via: "api" });
    const outcome = await applyEdgeAnswer(file, answer({ answerTokenHash: tokenHashOf(item.token) }), async () => {});
    expect(outcome).toBe("duplicate");
    expect((await getNeedsDrew(file, "q-1"))?.answer).toMatchObject({ optionId: "no", via: "api" });
  });

  test("a wrong token hash, an unknown question and a foreign option are final without a write", async () => {
    const { file, item } = await ledger();
    const hash = tokenHashOf(item.token);
    expect(await applyEdgeAnswer(file, answer({ answerTokenHash: tokenHashOf("other") }), async () => {})).toBe(
      "rejected",
    );
    expect(await applyEdgeAnswer(file, answer({ ledgerId: "q-missing", answerTokenHash: hash }), async () => {})).toBe(
      "unknown",
    );
    expect(await applyEdgeAnswer(file, answer({ answerTokenHash: hash, optionId: "maybe" }), async () => {})).toBe(
      "rejected",
    );
    expect((await getNeedsDrew(file, "q-1"))?.answer).toBeUndefined();
  });

  test("Send back, offered at the edge, applies like any option", async () => {
    const { file, item } = await ledger();
    const outcome = await applyEdgeAnswer(
      file,
      answer({ answerTokenHash: tokenHashOf(item.token), optionId: "send-back" }),
      async () => {},
    );
    expect(outcome).toBe("applied");
  });
});
