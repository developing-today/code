// Oracles for Phase-2 step 3 (#13469): the edge answer intake. A
// needs-Drew question mirrored with its token hash is answerable at
// the edge; the first answer is kept exactly once until the
// aggregator drains and acks it; a withdrawn or expired question
// refuses the answer instead of storing it.
import { describe, expect, test } from "bun:test";
import {
  QUESTION_MAX_AGE_MS,
  acceptAnswer,
  hashToken,
  lookupQuestion,
  questionFromEvent,
  validateAnswerChoice,
} from "../src/answers.ts";
import worker, { PmNotify, type Env } from "../src/index.ts";

const BASE = "https://pm-notify.example.workers.dev";
const TOKEN = "tok-0123456789abcdef0123456789abcdef";

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
    RELAY_SHARED_SECRET: "test-secret",
    PM_NOTIFY: { idFromName: () => "singleton", get: () => stub },
  } as unknown as Env;
  return { env, values };
}

async function raisedEvent(
  over: { ledgerId?: string; raisedAt?: string; answerType?: string; mirrorId?: string; token?: string } = {},
) {
  const ledgerId = over.ledgerId ?? "q-1";
  return {
    mirrorId: over.mirrorId ?? `raised-${ledgerId}`,
    objectRef: `needs-drew:${ledgerId}`,
    transition: "raised",
    raisedAt: over.raisedAt ?? new Date().toISOString(),
    payload: {
      class: "needs-user",
      title: "Needs Drew",
      needsDrew: {
        ledgerId,
        question: "Ship <b>it</b>?",
        context: "Context line",
        options: [
          { id: "yes", label: "Yes" },
          { id: "no", label: "No" },
        ],
        answerType: over.answerType ?? "radio",
        links: ["https://example.com/pr/1", "javascript:alert(1)"],
        tokenHash: await hashToken(over.token ?? TOKEN),
      },
    },
  };
}

async function raisedNeedsUserEvent() {
  const legacy = await raisedEvent();
  const { needsDrew, ...payload } = legacy.payload;
  return {
    ...legacy,
    objectRef: "needs-user:q-1",
    payload: { ...payload, needsUser: needsDrew },
  };
}

const clearedEvent = (ledgerId = "q-1") => ({
  mirrorId: `cleared-${ledgerId}`,
  objectRef: `needs-drew:${ledgerId}`,
  transition: "cleared",
  raisedAt: new Date().toISOString(),
  payload: { class: "needs-user" },
});

const authed = (env: Env, path: string, init: RequestInit = {}) =>
  worker.fetch(
    new Request(`${BASE}${path}`, {
      ...init,
      headers: { "content-type": "application/json", authorization: "Bearer test-secret" },
    }),
    env,
  );

const mirror = (env: Env, event: unknown) =>
  authed(env, "/mirror/event", { method: "POST", body: JSON.stringify(event) });

const postAnswer = (env: Env, ledgerId: string, body: unknown) =>
  postAnswerAt(env, `/api/needs-drew/${ledgerId}/answer`, body);

const postAnswerAt = (env: Env, path: string, body: unknown) =>
  worker.fetch(
    new Request(`${BASE}${path}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    }),
    env,
  );

const heartbeat = (env: Env, seq = 1) =>
  authed(env, "/heartbeat", { method: "POST", body: JSON.stringify({ stackEpoch: "e1", seq }) });

describe("answer page", () => {
  test("an open question renders its options and context, escaped, with only http links", async () => {
    const { env } = fixture();
    await mirror(env, await raisedEvent());
    const res = await worker.fetch(new Request(`${BASE}/answer/q-1?t=${TOKEN}`), env);
    const body = await res.text();
    expect(res.status).toBe(200);
    expect(res.headers.get("content-type")).toContain("text/html");
    expect(res.headers.get("cache-control")).toBe("no-store");
    expect(body).toContain("Ship &lt;b&gt;it&lt;/b&gt;?");
    expect(body).toContain('data-option="yes"');
    expect(body).toContain("https://example.com/pr/1");
    expect(body).not.toContain("javascript:alert");
    expect(body).toContain("/api/needs-drew/q-1/answer");
  });

  test("a needs-user mirror key and object ref use the same answer flow as the legacy form", async () => {
    const { env, values } = fixture();
    await mirror(env, await raisedNeedsUserEvent());

    const page = await worker.fetch(new Request(`${BASE}/answer/q-1?t=${TOKEN}`), env);
    const html = await page.text();
    expect(page.status).toBe(200);
    expect(html).toContain("/api/needs-user/q-1/answer");

    const answer = await postAnswerAt(env, "/api/needs-user/q-1/answer", { token: TOKEN, optionId: "yes" });
    expect(answer.status).toBe(200);
    const state = values.get("relay-state") as { pendingAnswers: Array<{ ledgerId: string; optionId: string }> };
    expect(state.pendingAnswers).toEqual([expect.objectContaining({ ledgerId: "q-1", optionId: "yes" })]);
  });

  test("a wrong token and an unknown question both get the same forbidden page", async () => {
    const { env } = fixture();
    await mirror(env, await raisedEvent());
    const wrong = await worker.fetch(new Request(`${BASE}/answer/q-1?t=nope`), env);
    const unknown = await worker.fetch(new Request(`${BASE}/answer/q-missing?t=${TOKEN}`), env);
    const missing = await worker.fetch(new Request(`${BASE}/answer/q-1`), env);
    for (const res of [wrong, unknown, missing]) {
      expect(res.status).toBe(403);
      const text = await res.text();
      expect(text).not.toContain("Ship");
    }
  });

  test("a withdrawn question's page refuses with 410", async () => {
    const { env } = fixture();
    await mirror(env, await raisedEvent());
    await mirror(env, clearedEvent());
    const res = await worker.fetch(new Request(`${BASE}/answer/q-1?t=${TOKEN}`), env);
    expect(res.status).toBe(410);
    expect(await res.text()).toContain("no longer takes answers");
  });

  test("an expired question's page refuses with 410", async () => {
    const { env } = fixture();
    await mirror(
      env,
      await raisedEvent({ raisedAt: new Date(Date.now() - QUESTION_MAX_AGE_MS - 60_000).toISOString() }),
    );
    const res = await worker.fetch(new Request(`${BASE}/answer/q-1?t=${TOKEN}`), env);
    expect(res.status).toBe(410);
    expect(await res.text()).toContain("more than 7 days ago");
  });

  test("a question mirrored before step 3 (no token hash) is unknown at the edge", async () => {
    const { env } = fixture();
    const legacy = await raisedEvent();
    const { tokenHash: _drop, ...rest } = legacy.payload.needsDrew;
    await mirror(env, { ...legacy, payload: { ...legacy.payload, needsDrew: rest } });
    const res = await worker.fetch(new Request(`${BASE}/answer/q-1?t=${TOKEN}`), env);
    expect(res.status).toBe(403);
  });

  test("a public caller cannot rewrite the origin the DO learned from the laptop", async () => {
    const { env, values } = fixture();
    await heartbeat(env);
    await worker.fetch(new Request(`${BASE}/status`, { headers: { "X-Relay-Origin": "https://evil.example" } }), env);
    const state = values.get("relay-state") as { origin: string };
    expect(state.origin).toBe(BASE);
  });
});

describe("POST /api/needs-drew/:ledgerId/answer", () => {
  test("an answer submitted twice (double-tap, retry) is stored exactly once", async () => {
    const { env, values } = fixture();
    await mirror(env, await raisedEvent());
    const first = await postAnswer(env, "q-1", { token: TOKEN, optionId: "yes" });
    const retry = await postAnswer(env, "q-1", { token: TOKEN, optionId: "no" });
    const firstBody = (await first.json()) as { duplicate: boolean; answerId: string };
    const retryBody = (await retry.json()) as { duplicate: boolean; answerId: string };
    expect(first.status).toBe(200);
    expect(firstBody.duplicate).toBe(false);
    expect(retry.status).toBe(200);
    expect(retryBody.duplicate).toBe(true);
    expect(retryBody.answerId).toBe(firstBody.answerId);
    const state = values.get("relay-state") as { pendingAnswers: Array<{ optionId: string; ledgerId: string }> };
    expect(state.pendingAnswers).toHaveLength(1);
    expect(state.pendingAnswers[0]).toMatchObject({ ledgerId: "q-1", optionId: "yes" });
  });

  test("concurrent submissions still store one answer", async () => {
    const { env, values } = fixture();
    await mirror(env, await raisedEvent());
    await Promise.all([
      postAnswer(env, "q-1", { token: TOKEN, optionId: "yes" }),
      postAnswer(env, "q-1", { token: TOKEN, optionId: "no" }),
      postAnswer(env, "q-1", { token: TOKEN, freeText: "maybe" }),
    ]);
    const state = values.get("relay-state") as { pendingAnswers: unknown[]; answered: unknown[] };
    expect(state.pendingAnswers).toHaveLength(1);
    expect(state.answered).toHaveLength(1);
  });

  test("a withdrawn question refuses the answer instead of storing it", async () => {
    const { env, values } = fixture();
    await mirror(env, await raisedEvent());
    await mirror(env, clearedEvent());
    const res = await postAnswer(env, "q-1", { token: TOKEN, optionId: "yes" });
    expect(res.status).toBe(410);
    const state = values.get("relay-state") as { pendingAnswers: unknown[] };
    expect(state.pendingAnswers).toEqual([]);
  });

  test("an expired question refuses the answer instead of storing it", async () => {
    const { env, values } = fixture();
    await mirror(env, await raisedEvent({ raisedAt: new Date(Date.now() - QUESTION_MAX_AGE_MS - 1).toISOString() }));
    const res = await postAnswer(env, "q-1", { token: TOKEN, optionId: "yes" });
    expect(res.status).toBe(410);
    const state = values.get("relay-state") as { pendingAnswers: unknown[] };
    expect(state.pendingAnswers).toEqual([]);
  });

  test("the answer POST is open to the console's service worker across origins", async () => {
    const { env } = fixture();
    await mirror(env, await raisedEvent());
    const preflight = await worker.fetch(new Request(`${BASE}/api/needs-drew/q-1/answer`, { method: "OPTIONS" }), env);
    expect(preflight.status).toBe(204);
    expect(preflight.headers.get("access-control-allow-headers")).toBe("content-type");
    const res = await postAnswer(env, "q-1", { token: TOKEN, optionId: "yes" });
    expect(res.headers.get("access-control-allow-origin")).toBe("*");
  });

  test("a wrong token stores nothing and says nothing about the question", async () => {
    const { env, values } = fixture();
    await mirror(env, await raisedEvent());
    const res = await postAnswer(env, "q-1", { token: "wrong", optionId: "yes" });
    expect(res.status).toBe(403);
    const state = values.get("relay-state") as { pendingAnswers: unknown[] };
    expect(state.pendingAnswers).toEqual([]);
  });

  test("an option that is not the question's is rejected", async () => {
    const { env } = fixture();
    await mirror(env, await raisedEvent());
    const res = await postAnswer(env, "q-1", { token: TOKEN, optionId: "maybe" });
    expect(res.status).toBe(400);
  });

  test("an answer after the drain is still told it was already answered", async () => {
    const { env, values } = fixture();
    await mirror(env, await raisedEvent());
    const first = (await (await postAnswer(env, "q-1", { token: TOKEN, optionId: "yes" })).json()) as {
      answerId: string;
    };
    await authed(env, "/answers/ack", { method: "POST", body: JSON.stringify({ answerIds: [first.answerId] }) });
    await mirror(env, clearedEvent());
    const retry = await postAnswer(env, "q-1", { token: TOKEN, optionId: "no" });
    expect(retry.status).toBe(200);
    expect(await retry.json()).toMatchObject({ duplicate: true, answerId: first.answerId });
    const page = await worker.fetch(new Request(`${BASE}/answer/q-1?t=${TOKEN}`), env);
    expect(page.status).toBe(200);
    expect(await page.text()).toContain("Your answer was received");
    const state = values.get("relay-state") as { pendingAnswers: unknown[] };
    expect(state.pendingAnswers).toEqual([]);
  });
});

describe("drain: heartbeat count, GET /answers, POST /answers/ack", () => {
  test("the heartbeat reports the pending count and the drain empties it", async () => {
    const { env } = fixture();
    await mirror(env, await raisedEvent());
    await mirror(env, await raisedEvent({ ledgerId: "q-2", token: "other-token-value-123456" }));
    expect(await (await heartbeat(env, 1)).json()).toMatchObject({ pendingAnswers: 0 });
    await postAnswer(env, "q-1", { token: TOKEN, optionId: "yes" });
    await postAnswer(env, "q-2", { token: "other-token-value-123456", freeText: "later" });
    expect(await (await heartbeat(env, 2)).json()).toMatchObject({ pendingAnswers: 2 });

    const drained = (await (await authed(env, "/answers")).json()) as {
      answers: Array<{ answerId: string; ledgerId: string; answerTokenHash: string }>;
    };
    expect(drained.answers.map((a) => a.ledgerId)).toEqual(["q-1", "q-2"]);
    expect(drained.answers[0].answerTokenHash).toBe(await hashToken(TOKEN));

    const ack = await authed(env, "/answers/ack", {
      method: "POST",
      body: JSON.stringify({ answerIds: [drained.answers[0].answerId] }),
    });
    expect(await ack.json()).toMatchObject({ ok: true, acked: 1, pendingAnswers: 1 });
    // A retried ack after a lost response changes nothing.
    const again = await authed(env, "/answers/ack", {
      method: "POST",
      body: JSON.stringify({ answerIds: [drained.answers[0].answerId] }),
    });
    expect(await again.json()).toMatchObject({ ok: true, acked: 0, pendingAnswers: 1 });
    expect(await (await heartbeat(env, 3)).json()).toMatchObject({ pendingAnswers: 1 });
  });

  test("the drain routes require the shared secret", async () => {
    const { env } = fixture();
    const list = await worker.fetch(new Request(`${BASE}/answers`), env);
    const ack = await worker.fetch(
      new Request(`${BASE}/answers/ack`, { method: "POST", body: JSON.stringify({ answerIds: [] }) }),
      env,
    );
    expect(list.status).toBe(401);
    expect(ack.status).toBe(401);
  });

  test("state stored before step 3 loads with an empty mailbox", async () => {
    const { env, values } = fixture();
    values.set("relay-state", { snapshot: null, mirrorEvents: [] });
    expect(await (await heartbeat(env)).json()).toMatchObject({ pendingAnswers: 0 });
    expect(await (await authed(env, "/answers")).json()).toMatchObject({ answers: [] });
  });
});

describe("pure helpers", () => {
  test("questionFromEvent requires a hex token hash and caps options", async () => {
    const event = await raisedEvent();
    expect(questionFromEvent(event)?.links).toEqual(["https://example.com/pr/1"]);
    const badHash = {
      ...event,
      payload: { ...event.payload, needsDrew: { ...event.payload.needsDrew, tokenHash: "x" } },
    };
    expect(questionFromEvent(badHash)).toBeNull();
    const ten = Array.from({ length: 10 }, (_, i) => ({ id: `o${i}`, label: `O${i}` }));
    const tooMany = {
      ...event,
      payload: { ...event.payload, needsDrew: { ...event.payload.needsDrew, options: ten } },
    };
    expect(questionFromEvent(tooMany)).toBeNull();
  });

  test("answerOptions (Send back included) are the answerable options when mirrored", async () => {
    const event = await raisedEvent();
    const answerOptions = [...event.payload.needsDrew.options, { id: "send-back", label: "Send back" }];
    const question = questionFromEvent({
      ...event,
      payload: { ...event.payload, needsDrew: { ...event.payload.needsDrew, answerOptions } },
    });
    expect(question?.options.map((o) => o.id)).toEqual(["yes", "no", "send-back"]);
  });

  test("a re-raise after a clear reopens the question", async () => {
    const events = [await raisedEvent(), clearedEvent(), await raisedEvent({ mirrorId: "raised-again" })];
    expect(lookupQuestion(events, "q-1", Date.now()).kind).toBe("open");
  });

  test("checkbox answers dedupe option ids; empty answers are rejected", async () => {
    const question = questionFromEvent(await raisedEvent({ answerType: "checkbox" }));
    if (!question) throw new Error("fixture question");
    expect(validateAnswerChoice(question, { optionIds: ["yes", "yes"] })).toEqual({
      ok: true,
      value: { optionIds: ["yes"] },
    });
    expect(validateAnswerChoice(question, { freeText: "   " }).ok).toBe(false);
  });

  test("the mailbox refuses past its cap instead of growing without bound", async () => {
    const question = questionFromEvent(await raisedEvent());
    if (!question) throw new Error("fixture question");
    const pendingAnswers = Array.from({ length: 256 }, (_, i) => ({
      answerId: `a${i}`,
      ledgerId: `other-${i}`,
      answeredAt: new Date().toISOString(),
      answerTokenHash: question.tokenHash,
    }));
    const outcome = acceptAnswer({ pendingAnswers, answered: [] }, question, { optionId: "yes" }, Date.now(), "new");
    expect(outcome.outcome).toBe("full");
  });
});
