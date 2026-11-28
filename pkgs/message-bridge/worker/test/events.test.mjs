// ChatGPT MCP Events: subscribe with a verified, signed callback, and a
// message for ChatGPT wakes every live subscription with a signed event.
import assert from "node:assert/strict";
import { createHmac } from "node:crypto";
import { test } from "node:test";
import { EVENT_NAME, callbackUrl, secretKey } from "../src/events.js";
import { MemoryStore } from "../src/messages.js";
import { route } from "../src/routes.js";

const TOKEN = "o".repeat(20) + "0123456789abcdefXYZ_-";
const GPT = "g".repeat(20) + "0123456789abcdefXYZ_-";
const ORIGIN = "https://bridge.example";
const HOOK = "https://hooks.chatgpt.com/mcp-events/abc123";
const SECRET = "whsec_" + Buffer.alloc(32, 7).toString("base64");
const SECRET2 = "whsec_" + Buffer.alloc(32, 9).toString("base64");

/** A fake callback endpoint: records requests, answers by a scripted rule. */
function egress() {
  const calls = [];
  let answer = (body) => (body.type === "verification" ? Response.json({ challenge: body.challenge }) : new Response(null, { status: 204 }));
  return {
    calls,
    answerWith(fn) {
      answer = fn;
    },
    async fetch(url, init) {
      const body = JSON.parse(init.body);
      calls.push({ url, init, body });
      return answer(body, calls.length);
    },
  };
}

function harness(extraEnv = {}) {
  const store = new MemoryStore(() => "2026-11-21T00:00:00.000Z");
  const hook = egress();
  const env = { ACCESS_TOKEN: TOKEN, CHATGPT_TOKEN: GPT, EVENTS_EGRESS: hook, ...extraEnv };
  const pending = [];
  const call = (path, init, e = env) => route(new Request(ORIGIN + path, init), e, () => store, (p) => pending.push(p));
  const rpc = async (token, method, params, e = env) => {
    const res = await call(`/mcp/${token}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }),
    }, e);
    return res.json();
  };
  const send = async (to, body = "hello ChatGPT", e = env) => {
    const res = await call(`/api/${TOKEN}/messages`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ body, to }),
    }, e);
    assert.equal(res.status, 201);
    await Promise.all(pending.splice(0));
  };
  return { store, hook, env, call, rpc, send };
}

const sub = (secret = SECRET, url = HOOK, extra = {}) => ({
  name: EVENT_NAME,
  arguments: {},
  delivery: { mode: "webhook", url, secret },
  cursor: null,
  ...extra,
});

/** Checks a request's Standard Webhooks signature against each given secret. */
function signedBy(call, ...secrets) {
  const h = call.init.headers;
  const signed = `${h["webhook-id"]}.${h["webhook-timestamp"]}.${call.init.body}`;
  const sigs = h["webhook-signature"].split(" ");
  return secrets.every((s) => sigs.includes("v1," + createHmac("sha256", secretKey(s)).update(signed).digest("base64")));
}

test("only the ChatGPT channel advertises events", async () => {
  const h = harness();
  const gpt = await h.rpc(GPT, "initialize", { protocolVersion: "2025-03-26" });
  assert.deepEqual(gpt.result.capabilities.events, {});
  assert.deepEqual((await h.rpc(GPT, "server/discover")).result.capabilities.events, {});
  const claude = await h.rpc(TOKEN, "initialize", { protocolVersion: "2025-03-26" });
  assert.equal(claude.result.capabilities.events, undefined);
  assert.equal((await h.rpc(TOKEN, "events/list")).error.code, -32601);
  const list = (await h.rpc(GPT, "events/list")).result.events;
  assert.equal(list.length, 1);
  assert.equal(list[0].name, EVENT_NAME);
  assert.deepEqual(list[0].delivery, ["webhook"]);
});

test("subscribe refuses bad secrets, hosts, arguments and modes without calling out", async () => {
  const h = harness();
  const bad = [
    sub("whsec_short"),
    sub("nope_" + Buffer.alloc(32, 7).toString("base64")),
    sub(SECRET, "https://evil.example/hook"),
    sub(SECRET, "http://hooks.chatgpt.com/x"),
    sub(SECRET, "https://user:pw@hooks.chatgpt.com/x"),
    sub(SECRET, "https://hooks.chatgpt.com:8443/x"),
    sub(SECRET, "https://chatgpt.com.evil.example/x"),
    { ...sub(), arguments: { since: 1 } },
    { ...sub(), delivery: { mode: "poll", url: HOOK, secret: SECRET } },
    { ...sub(), name: "other.event" },
    sub(SECRET, HOOK, { ttlMs: -5 }),
  ];
  for (const params of bad) {
    const r = await h.rpc(GPT, "events/subscribe", params);
    assert.equal(r.error?.code, -32602, JSON.stringify(params));
  }
  assert.equal(h.hook.calls.length, 0);
});

test("subscribe verifies the callback with a signed challenge before storing it", async () => {
  const h = harness();
  h.hook.answerWith((body) => Response.json({ challenge: "wrong" }));
  const failed = await h.rpc(GPT, "events/subscribe", sub());
  assert.equal(failed.error.code, -32015);
  assert.equal(failed.error.data.reason, "challenge_failed");
  h.hook.answerWith(() => new Response(null, { status: 302, headers: { location: "https://evil.example" } }));
  assert.equal((await h.rpc(GPT, "events/subscribe", sub())).error.code, -32015);
  assert.equal((await h.store.eventSubsLive("chatgpt", Date.now())).length, 0);

  h.hook.answerWith((body) => Response.json({ challenge: body.challenge }));
  const ok = (await h.rpc(GPT, "events/subscribe", sub())).result;
  assert.match(ok.id, /^sub_[0-9a-f]{40}$/);
  assert.equal(ok.cursor, null);
  assert.equal(ok.truncated, false);
  assert.ok(Date.parse(ok.refreshBefore) > Date.now());
  const verify = h.hook.calls.at(-1);
  assert.equal(verify.body.type, "verification");
  assert.equal(verify.init.redirect, "manual");
  assert.equal(verify.init.headers["x-mcp-subscription-id"], ok.id);
  assert.ok(signedBy(verify, SECRET));

  // Same identity, same secret, recently verified: renewed without a new challenge.
  const before = h.hook.calls.length;
  assert.equal((await h.rpc(GPT, "events/subscribe", sub())).result.id, ok.id);
  assert.equal(h.hook.calls.length, before);
  assert.equal((await h.store.eventSubsLive("chatgpt", Date.now())).length, 1);
});

test("a message for ChatGPT wakes the subscription with a signed event; Claude-only doesn't", async () => {
  const h = harness();
  const { id } = (await h.rpc(GPT, "events/subscribe", sub())).result;
  const before = h.hook.calls.length;
  await h.send("claude", "just for Claude");
  assert.equal(h.hook.calls.length, before);

  await h.send(["claude", "chatgpt"], "for both");
  const event = h.hook.calls.at(-1);
  assert.equal(h.hook.calls.length, before + 1);
  assert.equal(event.url, HOOK);
  assert.equal(event.body.name, EVENT_NAME);
  assert.equal(event.body.data.body, "for both");
  assert.equal(event.body.data.kind, "message");
  assert.equal(event.body.cursor, null);
  assert.equal(event.init.headers["webhook-id"], event.body.eventId);
  assert.equal(event.init.headers["x-mcp-subscription-id"], id);
  assert.ok(signedBy(event, SECRET));
  const log = await h.store.oauthEvents();
  assert.equal(log[0].kind, "event_delivered");
  assert.equal(log[0].message_id, event.body.data.message_id);
});

test("the page form wakes ChatGPT too", async () => {
  const h = harness();
  await h.rpc(GPT, "events/subscribe", sub());
  const before = h.hook.calls.length;
  const pending = [];
  const res = await route(
    new Request(`${ORIGIN}/p/${TOKEN}`, {
      method: "POST",
      headers: { "content-type": "application/x-www-form-urlencoded" },
      body: new URLSearchParams([["body", "from the page"], ["to", "chatgpt"], ["kind", "instruction"]]).toString(),
    }),
    h.env,
    () => h.store,
    (p) => pending.push(p),
  );
  assert.equal(res.status, 303);
  await Promise.all(pending);
  assert.equal(h.hook.calls.length, before + 1);
  assert.equal(h.hook.calls.at(-1).body.data.kind, "instruction");
});

test("a failed delivery retries with the same event id; 410 ends the subscription", async () => {
  const h = harness();
  await h.rpc(GPT, "events/subscribe", sub());
  let n = 0;
  h.hook.answerWith(() => new Response(null, { status: n++ === 0 ? 503 : 204 }));
  const before = h.hook.calls.length;
  await h.send("chatgpt");
  const tries = h.hook.calls.slice(before);
  assert.equal(tries.length, 2);
  assert.equal(tries[0].body.eventId, tries[1].body.eventId);
  assert.ok(signedBy(tries[1], SECRET));

  h.hook.answerWith(() => new Response(null, { status: 410 }));
  await h.send("chatgpt");
  assert.equal((await h.store.eventSubsLive("chatgpt", Date.now())).length, 0);
  const after = h.hook.calls.length;
  await h.send("chatgpt");
  assert.equal(h.hook.calls.length, after);
});

test("a new secret re-verifies and both secrets sign during rotation", async () => {
  const h = harness();
  await h.rpc(GPT, "events/subscribe", sub(SECRET));
  const before = h.hook.calls.length;
  await h.rpc(GPT, "events/subscribe", sub(SECRET2));
  assert.equal(h.hook.calls.length, before + 1, "the new secret is verified");
  await h.send("chatgpt");
  assert.ok(signedBy(h.hook.calls.at(-1), SECRET2, SECRET));
});

test("rotating the ChatGPT token ends its subscriptions", async () => {
  const h = harness();
  await h.rpc(GPT, "events/subscribe", sub());
  const rotated = { ...h.env, CHATGPT_TOKEN: "r".repeat(20) + "0123456789abcdefXYZ_-" };
  const before = h.hook.calls.length;
  await h.send("chatgpt", "after rotation", rotated);
  assert.equal(h.hook.calls.length, before);
  assert.equal((await h.store.eventSubsLive("chatgpt", Date.now())).length, 0);
});

test("unsubscribe stops delivery and is idempotent; the owner can list subscriptions without secrets", async () => {
  const h = harness();
  await h.rpc(GPT, "events/subscribe", sub());
  const listed = await (await h.call(`/api/${TOKEN}/event-subscriptions`)).json();
  assert.equal(listed.subscriptions.length, 1);
  assert.equal(listed.subscriptions[0].host, "hooks.chatgpt.com");
  assert.ok(!JSON.stringify(listed).includes("whsec_"));
  assert.ok(!JSON.stringify(listed).includes("abc123"));
  assert.equal((await h.call(`/api/${GPT}/event-subscriptions`)).status, 404);

  const unsub = { name: EVENT_NAME, arguments: {}, delivery: { mode: "webhook", url: HOOK } };
  assert.deepEqual((await h.rpc(GPT, "events/unsubscribe", unsub)).result, {});
  assert.deepEqual((await h.rpc(GPT, "events/unsubscribe", unsub)).result, {});
  const before = h.hook.calls.length;
  await h.send("chatgpt");
  assert.equal(h.hook.calls.length, before);
});

test("callback hosts: OpenAI domains and configured exact hosts only", () => {
  assert.ok(callbackUrl({}, "https://chatgpt.com/hooks/x"));
  assert.ok(callbackUrl({}, "https://api.openai.com/v1/mcp/events/x"));
  assert.equal(callbackUrl({}, "https://notopenai.com/x"), null);
  assert.equal(callbackUrl({}, "https://openai.com.evil.example/x"), null);
  assert.equal(callbackUrl({}, "https://hooks.example.net/x"), null);
  assert.ok(callbackUrl({ EVENTS_CALLBACK_HOSTS: "hooks.example.net" }, "https://hooks.example.net/x"));
  assert.equal(callbackUrl({ EVENTS_CALLBACK_HOSTS: "hooks.example.net" }, "https://a.hooks.example.net/x"), null);
});
