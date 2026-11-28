// Fleet mailboxes (src/fleet.js): routed end to end through route(), with
// the real fleetOp SQL running on node:sqlite in place of the Durable
// Object's storage (same exec(query, ...args).toArray() shape).
import assert from "node:assert/strict";
import { test } from "node:test";
import { DatabaseSync } from "node:sqlite";
import { deriveFleetToken, fleetCall, fleetOp, fleetSchema, fleetToken, getDistribution, parseTargets } from "../src/fleet.js";
import { MemoryStore } from "../src/messages.js";
import { route } from "../src/routes.js";

const TOKEN = "o".repeat(20) + "0123456789abcdefXYZ_-";
const CHATGPT = "c".repeat(40);
const env = { ACCESS_TOKEN: TOKEN, CHATGPT_TOKEN: CHATGPT };
const BASE = "https://bridge.example";

/** The Durable Object's sql.exec over node:sqlite. */
function sqlite() {
  const db = new DatabaseSync(":memory:");
  return {
    exec(q, ...args) {
      const out = db.prepare(q).all(...args);
      return { toArray: () => out, one: () => out[0] };
    },
  };
}

async function harness() {
  const sql = sqlite();
  fleetSchema(sql);
  fleetSchema(sql); // idempotent, as on every Durable Object start
  let clock = 0;
  const stub = { fleet: (op, args) => fleetOp(sql, op, args, `2026-11-21T00:00:${String(clock++).padStart(2, "0")}Z`) };
  const store = new MemoryStore();
  store.fleet = (op, args) => fleetCall(stub, op, args);
  const call = (path, init) => route(new Request(BASE + path, init), env, () => store);
  const ft = await fleetToken(env);
  const post = (path, body) =>
    call(path, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
  const agent = {
    hello: (body) => post(`/fleet/${ft}/hello`, body),
    inbox: (qs) => call(`/fleet/${ft}/inbox?${new URLSearchParams(qs)}`),
    ack: (body) => post(`/fleet/${ft}/ack`, body),
    send: (body) => post(`/fleet/${ft}/send`, body),
  };
  const owner = {
    send: (body) => post(`/api/${TOKEN}/fleet/send`, body),
    agents: () => call(`/api/${TOKEN}/fleet/agents`),
    messages: (qs = {}) => call(`/api/${TOKEN}/fleet/messages?${new URLSearchParams(qs)}`),
  };
  return { store, call, ft, agent, owner };
}

test("fleet token is derived from the access token, distinct, stable", async () => {
  const ft = await fleetToken(env);
  assert.equal(ft, await deriveFleetToken(TOKEN));
  assert.match(ft, /^[A-Za-z0-9_-]{43}$/);
  assert.notEqual(ft, TOKEN);
  assert.notEqual(await deriveFleetToken(TOKEN + "x"), ft, "rotating the access token rotates it");
  assert.equal(await fleetToken({}), null, "no access token, no fleet token");
  const own = "f".repeat(40);
  assert.equal(await fleetToken({ ...env, FLEET_TOKEN: own }), own, "an explicit FLEET_TOKEN wins");
  assert.equal(await fleetToken({ ...env, FLEET_TOKEN: TOKEN }), ft, "FLEET_TOKEN equal to the access token is ignored");
  assert.equal(await fleetToken({ ...env, FLEET_TOKEN: "short" }), ft);
});

test("targets parse agents, groups and all", () => {
  assert.deepEqual(parseTargets("agent:Muse-1, muse-2,group:muse"), {
    agents: ["muse-1", "muse-2"],
    groups: ["muse"],
    all: false,
  });
  assert.deepEqual(parseTargets(["all"]), { agents: [], groups: [], all: true });
  for (const bad of ["", [], "agent:", "group:", "bad name", [" "], "agent:-x"]) {
    assert.throws(() => parseTargets(bad), /recipient|agent|group/, JSON.stringify(bad));
  }
});

test("wrong tokens are a plain 404 and keys do not cross", async () => {
  const { call, ft } = await harness();
  for (const path of [
    `/fleet/nope/inbox?agent=a`,
    `/fleet/${TOKEN}/inbox?agent=a`, // the owner token is not the fleet token
    `/fleet/${CHATGPT}/inbox?agent=a`,
    `/api/${ft}/fleet/agents`, // the fleet token can't use the owner API
    `/api/${ft}/messages`,
    `/p/${ft}`,
    `/mcp/${ft}`,
    `/api/nope/fleet/agents`,
    `/fleet/${ft}/inbox/extra`,
    `/fleet/${ft}/nothing`,
  ]) {
    assert.equal((await call(path)).status, 404, path);
  }
});

test("round trip: central commands a group, agents read, answer and ack", async () => {
  const { agent, owner } = await harness();

  // No agents yet: a group or "all" has nobody to reach.
  let res = await owner.send({ body: "x", to: "group:muse", kind: "command" });
  assert.equal(res.status, 400);
  assert.match((await res.json()).error, /no registered agent is in group muse/);
  assert.equal((await owner.send({ body: "x", to: "all" })).status, 400);

  // Three agents register; two are in the muse group.
  assert.equal((await agent.hello({ agent: "Muse-1", groups: ["muse", "code-lanes"] })).status, 200);
  assert.equal((await agent.hello({ agent: "muse-2", groups: "muse" })).status, 200);
  const h3 = await (await agent.hello({ agent: "aster" })).json();
  assert.deepEqual(h3.agent.groups, []);
  assert.equal(h3.stats.unread_commands, 0);

  // Central sends one command to the muse group.
  res = await owner.send({ body: "run the backup check", to: "group:muse", kind: "command", sender: "drew" });
  assert.equal(res.status, 201);
  const sent = await res.json();
  assert.deepEqual(
    sent.messages.map((m) => [m.agent, m.kind, m.direction]),
    [
      ["muse-1", "command", "inbound"],
      ["muse-2", "command", "inbound"],
    ],
  );
  assert.equal(new Set(sent.messages.map((m) => m.batch)).size, 1, "one batch for the whole send");
  assert.deepEqual(sent.unregistered, []);

  // And a plain message to one agent by name, plus one not registered yet.
  res = await owner.send({ body: "fyi", to: ["agent:aster", "newbie"] });
  const fyi = await res.json();
  assert.deepEqual(fyi.unregistered, ["newbie"]);
  assert.equal(fyi.messages[0].kind, "message");

  // muse-1 reads only its commands; aster sees nothing of the command.
  let inbox = await (await agent.inbox({ agent: "muse-1", kind: "command" })).json();
  assert.equal(inbox.messages.length, 1);
  const cmd = inbox.messages[0];
  assert.equal(cmd.body, "run the backup check");
  assert.equal(cmd.sender, null, "a command reaches the agent with no sender");
  assert.equal(inbox.stats.unread_commands, 1);
  inbox = await (await agent.inbox({ agent: "aster" })).json();
  assert.deepEqual(
    inbox.messages.map((m) => m.body),
    ["fyi"],
  );

  // muse-1 answers the command: that marks it read and links the reply.
  res = await agent.send({ agent: "muse-1", body: "backup check ok", reply_to: cmd.id });
  assert.equal(res.status, 201);
  const reply = (await res.json()).message;
  assert.equal(reply.direction, "outbound");
  assert.equal(reply.sender, "muse-1");
  assert.equal(reply.reply_to, cmd.id);
  assert.equal(reply.batch, cmd.batch);
  inbox = await (await agent.inbox({ agent: "muse-1" })).json();
  assert.equal(inbox.messages.length, 0, "answered command is read");
  assert.equal(inbox.stats.unread_commands, 0);

  // An agent can't answer another agent's message.
  res = await agent.send({ agent: "aster", body: "x", reply_to: cmd.id });
  assert.equal(res.status, 400);

  // muse-2 acks without answering; then reading history still shows it.
  inbox = await (await agent.inbox({ agent: "muse-2" })).json();
  res = await agent.ack({ agent: "muse-2", up_to_id: inbox.next_since_id });
  assert.equal((await res.json()).acked, 1);
  inbox = await (await agent.inbox({ agent: "muse-2", unread_only: "false" })).json();
  assert.equal(inbox.messages.length, 1);
  assert.ok(inbox.messages[0].read_at);

  // Central sees the agents and the conversation.
  const agents = (await (await owner.agents()).json()).agents;
  assert.deepEqual(
    agents.map((a) => [a.name, a.groups, a.unread_commands, a.unread_messages]),
    [
      ["aster", [], 0, 1], // read but not acked: still unread
      ["muse-1", ["code-lanes", "muse"], 0, 0],
      ["muse-2", ["muse"], 0, 0],
    ],
  );
  const replies = (await (await owner.messages({ direction: "outbound" })).json()).messages;
  assert.deepEqual(
    replies.map((m) => [m.agent, m.body, m.reply_to]),
    [["muse-1", "backup check ok", cmd.id]],
  );
  const toNewbie = (await (await owner.messages({ agent: "newbie" })).json()).messages;
  assert.equal(toNewbie.length, 1);

  // "all" reaches every registered agent (newbie isn't registered).
  const all = await (await owner.send({ body: "pause", to: "all", kind: "command" })).json();
  assert.deepEqual(
    all.messages.map((m) => m.agent),
    ["aster", "muse-1", "muse-2"],
  );
});

test("agents can't send commands, and input is checked", async () => {
  const { agent, owner } = await harness();
  // The agent send route has no kind: whatever it says, it's a message.
  const res = await agent.send({ agent: "a1", body: "hi", kind: "command" });
  assert.equal((await res.json()).message.kind, "message");
  for (const [send, body] of [
    [agent.send, { agent: "a1", body: "" }],
    [agent.send, { agent: "A B", body: "x" }],
    [agent.ack, { agent: "a1" }],
    [agent.ack, { agent: "a1", ids: [0] }],
    [agent.hello, { agent: "a1", groups: ["bad group"] }],
    [owner.send, { body: "x", to: "a1", kind: "shout" }],
    [owner.send, { body: "x" }],
  ]) {
    const r = await send(body);
    assert.equal(r.status, 400, JSON.stringify(body));
    assert.ok((await r.json()).error);
  }
  assert.equal((await agent.inbox({ agent: "a1", limit: "0" })).status, 400);
  assert.equal((await agent.inbox({})).status, 400);
});

test("groups are replaced on hello and kept when omitted", async () => {
  const { agent } = await harness();
  await agent.hello({ agent: "m", groups: ["a", "b"] });
  let h = await (await agent.inbox({ agent: "m" })).json();
  assert.deepEqual(h.agent.groups, ["a", "b"], "inbox without groups keeps them");
  h = await (await agent.hello({ agent: "m", groups: ["c"] })).json();
  assert.deepEqual(h.agent.groups, ["c"]);
});

test("inbox mark_read hands each message out once", async () => {
  const { agent, owner } = await harness();
  await agent.hello({ agent: "m" });
  await owner.send({ body: "one", to: "m", kind: "command" });
  await owner.send({ body: "two", to: "m" });
  let box = await (await agent.inbox({ agent: "m", mark_read: "true", limit: "1" })).json();
  assert.deepEqual(box.messages.map((m) => m.body), ["one"]);
  assert.ok(box.messages[0].read_at);
  box = await (await agent.inbox({ agent: "m", mark_read: "1" })).json();
  assert.deepEqual(box.messages.map((m) => m.body), ["two"]);
  box = await (await agent.inbox({ agent: "m" })).json();
  assert.equal(box.messages.length, 0);
  assert.equal(box.stats.unread_commands + box.stats.unread_messages, 0);
});

test("glob targets parse like names, and malformed globs are refused", () => {
  assert.deepEqual(parseTargets("agent:f*"), { agents: ["f*"], groups: [], all: false });
  assert.deepEqual(parseTargets(["agent:Muse-*", "group:muse"]), { agents: ["muse-*"], groups: ["muse"], all: false });
  for (const bad of ["agent:f o*", "agent:-*", `agent:${"*".repeat(65)}`]) {
    assert.throws(() => parseTargets(bad), /pattern|agent/, JSON.stringify(bad));
  }
});

test("pattern send with limit: first match in name order, match report, broaden remainder", async () => {
  const { agent, owner } = await harness();
  await agent.hello({ agent: "fable" });
  await agent.hello({ agent: "falcon" });
  await agent.hello({ agent: "muse-1", groups: ["muse"] });

  // Oracle (#14367): agent:f* matches two registered agents; limit 1
  // delivers to exactly the first by name order and reports the rest.
  const res = await owner.send({ body: "take the next task", to: "agent:f*", kind: "command", limit: 1 });
  assert.equal(res.status, 201);
  const sent = await res.json();
  assert.equal(sent.matched, 2);
  assert.equal(sent.delivered, 1);
  assert.deepEqual(sent.recipients, ["fable"]);
  assert.equal(sent.remaining, 1);
  assert.equal((await (await agent.inbox({ agent: "fable" })).json()).messages.length, 1);
  assert.equal((await (await agent.inbox({ agent: "falcon" })).json()).messages.length, 0);

  // Broaden with limit 2 delivers to exactly the remaining agent.
  const b1 = await (await owner.send({ broaden: sent.batch, limit: 2 })).json();
  assert.equal(b1.matched, 2);
  assert.equal(b1.delivered, 1);
  assert.deepEqual(b1.recipients, ["falcon"]);
  assert.equal(b1.remaining, 0);
  const falconBox = (await (await agent.inbox({ agent: "falcon" })).json()).messages;
  assert.equal(falconBox.length, 1);
  assert.equal(falconBox[0].body, "take the next task");
  assert.equal(falconBox[0].batch, sent.batch);

  // A second identical broaden delivers nothing — never a duplicate.
  const b2 = await (await owner.send({ broaden: sent.batch, limit: 2 })).json();
  assert.equal(b2.delivered, 0);
  assert.deepEqual(b2.messages, []);
  assert.equal((await (await agent.inbox({ agent: "fable" })).json()).messages.length, 1);
  assert.equal((await (await agent.inbox({ agent: "falcon" })).json()).messages.length, 1);
});

test("a glob matching nobody is an error, like an empty group", async () => {
  const { agent, owner } = await harness();
  await agent.hello({ agent: "fable" });
  const res = await owner.send({ body: "x", to: "agent:z*" });
  assert.equal(res.status, 400);
  assert.match((await res.json()).error, /no registered agent matches agent:z\*/);
});

test("limit applies across group and all sends, in name order", async () => {
  const { agent, owner } = await harness();
  await agent.hello({ agent: "fable" });
  await agent.hello({ agent: "falcon", groups: ["crew"] });
  await agent.hello({ agent: "muse-1", groups: ["crew"] });
  const sent = await (await owner.send({ body: "crew task", to: "group:crew", limit: 1 })).json();
  assert.equal(sent.matched, 2);
  assert.deepEqual(sent.recipients, ["falcon"]);
  assert.equal(sent.remaining, 1);
  const all = await (await owner.send({ body: "everyone", to: "all", limit: 2 })).json();
  assert.equal(all.matched, 3);
  assert.deepEqual(all.recipients, ["fable", "falcon"]);
  assert.equal(all.remaining, 1);
});

test("broaden can widen the pattern and still never re-delivers", async () => {
  const { agent, owner } = await harness();
  await agent.hello({ agent: "fable" });
  await agent.hello({ agent: "falcon" });
  await agent.hello({ agent: "muse-1" });
  const sent = await (await owner.send({ body: "widen me", to: "agent:f*", limit: 1 })).json();
  assert.deepEqual(sent.recipients, ["fable"]);
  const b = await (await owner.send({ broaden: sent.batch, to: "agent:m*", limit: 5 })).json();
  assert.equal(b.matched, 3);
  assert.deepEqual(b.recipients, ["falcon", "muse-1"]);
  assert.equal(b.remaining, 0);
  assert.equal((await (await agent.inbox({ agent: "fable" })).json()).messages.length, 1);
});

test("every pattern send writes its bridge_distribution record", async () => {
  const sql = sqlite();
  fleetSchema(sql);
  const stub = { fleet: (op, args) => fleetOp(sql, op, args, "2026-11-24T00:00:00.000Z") };
  const call = (op, args) => fleetCall(stub, op, args);
  await call("hello", { agent: "fable" });
  await call("hello", { agent: "falcon" });
  const sent = await call("send", { body: "b", to: "agent:f*", limit: 1 });
  const dist = getDistribution(sql, sent.batch);
  assert.equal(dist.pattern, "agent:f*");
  assert.equal(dist.limit_n, 1);
  assert.equal(dist.sender_addr, null);
  assert.deepEqual(dist.matched, ["fable", "falcon"]);
  assert.deepEqual(dist.delivered, ["fable"]);
  await call("broaden", { batch: sent.batch, limit: 5 });
  assert.deepEqual(getDistribution(sql, sent.batch).delivered, ["fable", "falcon"]);
});
