import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import test from "node:test";

import worker, { Limiter } from "./src/index.js";

// Runtime-contract tests for the Worker handler and the Limiter Durable
// Object (#5739). The deploy.test.mjs suite covers deploy.mjs argument
// parsing only; these tests drive the real fetch handler and the real
// Limiter class against in-memory fakes of the DO storage/namespace APIs.

class FakeStorage {
  constructor() {
    this.data = new Map();
    this.alarmAt = null;
  }
  async get(keys) {
    return new Map(keys.filter((k) => this.data.has(k)).map((k) => [k, this.data.get(k)]));
  }
  async put(entries) {
    for (const [k, v] of Object.entries(entries)) this.data.set(k, v);
  }
  async deleteAll() {
    this.data.clear();
  }
  async setAlarm(at) {
    this.alarmAt = at;
  }
  async getAlarm() {
    return this.alarmAt;
  }
}

function makeLimiterNamespace() {
  const calls = [];
  const instances = new Map();
  return {
    calls,
    idFromName(key) {
      calls.push(key);
      return key;
    },
    get(id) {
      if (!instances.has(id)) {
        instances.set(id, new Limiter({ storage: new FakeStorage() }));
      }
      const limiter = instances.get(id);
      return { fetch: (url, init) => limiter.fetch(new Request(url, init)) };
    },
  };
}

const TOKEN = "secret-token";
const CALLER = {
  sha256: createHash("sha256").update(TOKEN).digest("hex"),
  transports: {
    email: { mode: "free", from: ["alice@example.com"], maxRecipients: 2 },
  },
};

function makeEnv({ callerPerWindow = 10, recipientPerWindow = 10 } = {}) {
  const sent = [];
  return {
    sent,
    LIMITER_NS: makeLimiterNamespace(),
    env: null,
    build() {
      const ns = this.LIMITER_NS;
      this.env = {
        CALLERS: JSON.stringify({ alice: CALLER }),
        MAIL_CONFIG: JSON.stringify({
          defaultTransport: "email",
          transports: ["email"],
          templates: [],
          limits: { windowSeconds: 60, callerPerWindow, recipientPerWindow },
        }),
        LIMITER: ns,
        EMAIL: {
          send: async ({ to }) => {
            sent.push(to);
            return { messageId: `id-${to}` };
          },
        },
      };
      return this.env;
    },
  };
}

function mailRequest(to) {
  return new Request("https://mail.example/", {
    method: "POST",
    headers: { authorization: `Bearer ${TOKEN}`, "content-type": "application/json" },
    body: JSON.stringify({ to, subject: "hi", text: "hello" }),
  });
}

test("caller denial returns 429 before any recipient Durable Object work", async () => {
  const fx = makeEnv({ callerPerWindow: 0 });
  const env = fx.build();
  const response = await worker.fetch(mailRequest(["one@example.com", "two@example.com"]), env);
  assert.equal(response.status, 429);
  // Only the caller key was resolved; no target:* key was ever looked up.
  assert.deepEqual(fx.LIMITER_NS.calls, ["caller:alice"]);
  assert.deepEqual(fx.sent, []);
});

test("caller-allowed requests deliver and consult recipient limiters", async () => {
  const fx = makeEnv();
  const env = fx.build();
  const response = await worker.fetch(mailRequest(["one@example.com", "two@example.com"]), env);
  assert.equal(response.status, 200);
  const body = await response.json();
  assert.deepEqual(body.messageIds, ["id-one@example.com", "id-two@example.com"]);
  assert.deepEqual(fx.LIMITER_NS.calls, [
    "caller:alice",
    "target:email:one@example.com",
    "target:email:two@example.com",
  ]);
});

test("recipient limit still rejects an over-limit caller-allowed request", async () => {
  const fx = makeEnv({ recipientPerWindow: 1 });
  const env = fx.build();
  const first = await worker.fetch(mailRequest("one@example.com"), env);
  assert.equal(first.status, 200);
  const second = await worker.fetch(mailRequest("one@example.com"), env);
  assert.equal(second.status, 429);
  assert.deepEqual(fx.sent, ["one@example.com"]);
});

test("per-request recipient cap still applies", async () => {
  const fx = makeEnv();
  const env = fx.build();
  const response = await worker.fetch(
    mailRequest(["a@example.com", "b@example.com", "c@example.com"]),
    env,
  );
  assert.equal(response.status, 400);
  assert.deepEqual(fx.sent, []);
});

test("limiter state is alarm-bounded and alarm cleans up dormant keys", async () => {
  const storage = new FakeStorage();
  const limiter = new Limiter({ storage });
  const take = async (limit, windowMs) => {
    const response = await limiter.fetch(
      new Request("https://limiter.internal/", {
        method: "POST",
        body: JSON.stringify({ limit, windowMs }),
      }),
    );
    return (await response.json()).success;
  };
  const before = Date.now();
  assert.equal(await take(1, 60000), true);
  // A successful take arms an alarm at the end of its window: the stored
  // counters cannot outlive the window even if the key goes dormant.
  assert.ok(storage.alarmAt !== null && storage.alarmAt >= before + 60000 - 1000);
  assert.ok(storage.alarmAt <= Date.now() + 60000);
  assert.equal(await take(1, 60000), false);
  await limiter.alarm();
  assert.equal(storage.data.size, 0);
  // After cleanup the same key starts a fresh window.
  assert.equal(await take(1, 60000), true);
});
