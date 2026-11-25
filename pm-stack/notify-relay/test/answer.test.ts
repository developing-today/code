// Oracles for Phase-2 step 3: offline answers are authorized against the
// mirrored question, stored once, and retained until an authenticated ACK.
import { describe, expect, test } from "bun:test";
import worker, { PmNotify, type Env } from "../src/index.ts";
import type { MirroredQuestion, RelayState } from "../src/relay.ts";

function fixture() {
  const values = new Map<string, unknown>();
  const durableState = {
    storage: {
      get: async <T>(key: string) => values.get(key) as T | undefined,
      put: async (key: string, value: unknown) => void values.set(key, value),
      setAlarm: async () => {},
    },
  };
  const relay = new PmNotify(durableState as never, {});
  const stub = { fetch: (request: Request) => relay.fetch(request) };
  const env = {
    RELAY_SHARED_SECRET: "relay-secret",
    PM_NOTIFY: { idFromName: () => "singleton", get: () => stub },
  } as unknown as Env;
  return { env, values, relay };
}

const token = "per-question-secret";
const tokenHash = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(token)).then((bytes) =>
  [...new Uint8Array(bytes)].map((byte) => byte.toString(16).padStart(2, "0")).join(""),
);
const question: MirroredQuestion = {
  ledgerId: "question-42", question: "Choose an option", context: "Context",
  options: [{ id: "a", label: "First" }, { id: "b", label: "Second" }, { id: "c", label: "Third" }],
  links: [], raisedAt: new Date().toISOString(), answerTokenHash: tokenHash,
};

const request = (env: Env, path: string, init: RequestInit = {}) => worker.fetch(
  new Request(`https://pm-notify.example.workers.dev${path}`, init), env,
);
const mirror = (env: Env, questions: MirroredQuestion[]) => request(env, "/mirror/questions", {
  method: "POST", headers: { authorization: "Bearer relay-secret", "content-type": "application/json" },
  body: JSON.stringify({ questions }),
});

describe("edge answers and durable pending mailbox", () => {
  test("answer page displays the entire option list, not only push actions", async () => {
    const { env } = fixture();
    await mirror(env, [question]);
    const response = await request(env, `/answer/${question.ledgerId}?t=${token}`);
    const page = await response.text();
    expect(response.status).toBe(200);
    expect(page).toContain("First");
    expect(page).toContain("Second");
    expect(page).toContain("Third");
  });

  test("invalid token is denied and leaves durable relay state unchanged", async () => {
    const { env, values } = fixture();
    await mirror(env, [question]);
    const before = structuredClone(values.get("relay-state"));
    const response = await request(env, `/api/needs-drew/${question.ledgerId}/answer`, {
      method: "POST", headers: { "content-type": "application/json" },
      body: JSON.stringify({ token: "wrong", optionId: "a" }),
    });
    expect(response.status).toBe(403);
    expect(values.get("relay-state")).toEqual(before);
  });

  test("a duplicate POST is idempotent and heartbeat reports one pending answer", async () => {
    const { env, values } = fixture();
    await mirror(env, [question]);
    const answer = () => request(env, `/api/needs-drew/${question.ledgerId}/answer`, {
      method: "POST", headers: { "content-type": "application/json" },
      body: JSON.stringify({ token, optionId: "c" }),
    });
    const first = await answer();
    const duplicate = await answer();
    expect(first.status).toBe(200);
    expect((await duplicate.json()).duplicate).toBe(true);
    const heartbeat = await request(env, "/heartbeat", {
      method: "POST", headers: { authorization: "Bearer relay-secret", "content-type": "application/json" },
      body: JSON.stringify({ stackEpoch: "epoch", seq: 1 }),
    });
    expect((await heartbeat.json()).pendingAnswers).toBe(1);
    expect((values.get("relay-state") as RelayState).pendingAnswers).toHaveLength(1);
  });

  test("pending answer requires shared auth; an unacked crash re-offers it and ACK removes it", async () => {
    const { env, values, relay } = fixture();
    await mirror(env, [question]);
    await request(env, `/api/needs-drew/${question.ledgerId}/answer`, {
      method: "POST", headers: { "content-type": "application/json" },
      body: JSON.stringify({ token, optionId: "b" }),
    });
    expect((await request(env, "/pending-answers")).status).toBe(401);
    const fetchPending = () => request(env, "/pending-answers", { headers: { authorization: "Bearer relay-secret" } });
    const beforeCrash = await fetchPending();
    const answer = (await beforeCrash.json()).answers[0];
    // A crash after the existing writer commits but before edge ACK leaves
    // both the local ledger answer and relay mailbox available for retry.
    const ledger = new Map<string, unknown>();
    ledger.set(answer.ledgerId, { optionId: answer.optionId, answerId: answer.answerId });
    await relay.alarm(); // fresh DO instance reads the same Durable Object storage
    expect(ledger.get(question.ledgerId)).toEqual({ optionId: "b", answerId: answer.answerId });
    expect((await (await fetchPending()).json()).answers).toHaveLength(1);
    const ack = await request(env, `/pending-answers/${answer.answerId}/ack`, {
      method: "POST", headers: { authorization: "Bearer relay-secret" },
    });
    expect((await ack.json()).acknowledged).toBe(true);
    expect((await (await fetchPending()).json()).answers).toEqual([]);
  });

  test("expired or withdrawn question refuses storage", async () => {
    const { env, values } = fixture();
    const expired = { ...question, raisedAt: new Date(Date.now() - 8 * 24 * 60 * 60_000).toISOString() };
    await mirror(env, [expired]);
    const expiredResponse = await request(env, `/api/needs-drew/${question.ledgerId}/answer`, {
      method: "POST", headers: { "content-type": "application/json" },
      body: JSON.stringify({ token, optionId: "a" }),
    });
    expect(expiredResponse.status).toBe(410);
    await mirror(env, []);
    const withdrawnResponse = await request(env, `/api/needs-drew/${question.ledgerId}/answer`, {
      method: "POST", headers: { "content-type": "application/json" },
      body: JSON.stringify({ token, optionId: "a" }),
    });
    expect(withdrawnResponse.status).toBe(410);
    expect((values.get("relay-state") as RelayState).pendingAnswers).toEqual([]);
  });
});
