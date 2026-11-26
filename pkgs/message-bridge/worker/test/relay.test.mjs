// Any-to-any: connectors and fleet agents message and instruct each
// other through one address grammar (relay.js), every row says who sent
// it, reply_to answers whoever sent a row, and recipients are woken
// (ChatGPT by MCP Events, anyone else by a wake hook, wake.js).
import assert from "node:assert/strict";
import { test } from "node:test";
import { DatabaseSync } from "node:sqlite";
import { fleetCall, fleetOp, fleetSchema, fleetToken } from "../src/fleet.js";
import { EVENT_NAME } from "../src/events.js";
import { MemoryStore } from "../src/messages.js";
import { ADDRESS_HELP, parseAddresses, replyAddress } from "../src/relay.js";
import { route } from "../src/routes.js";
import { wakeText } from "../src/wake.js";

const TOKEN = "o".repeat(20) + "0123456789abcdefXYZ_-";
const GPT = "g".repeat(20) + "0123456789abcdefXYZ_-";
const ORIGIN = "https://bridge.example";
const FIRE = "https://api.anthropic.com/v1/claude_code/routines/trig_01ABCDEFGHJKLMNOPQRSTUVW/fire";
const ROUTINE_TOKEN = "sk-ant-oat01-" + "t".repeat(40);
const HOOK = "https://hooks.chatgpt.com/mcp-events/abc123";
const SECRET = "whsec_" + Buffer.alloc(32, 7).toString("base64");

function sqlite() {
  const db = new DatabaseSync(":memory:");
  return {
    exec(q, ...args) {
      const out = db.prepare(q).all(...args);
      return { toArray: () => out, one: () => out[0] };
    },
  };
}

/** Records outbound calls (ChatGPT events and wake hooks) and answers them. */
function egress() {
  const calls = [];
  return {
    calls,
    async fetch(url, init) {
      const body = JSON.parse(init.body);
      calls.push({ url, init, body });
      if (body.type === "verification") return Response.json({ challenge: body.challenge });
      if (String(url).startsWith("https://api.anthropic.com/")) {
        return Response.json({ type: "routine_fire", claude_code_session_id: "session_x" });
      }
      return new Response(null, { status: 204 });
    },
  };
}

async function harness(extraEnv = {}) {
  const sql = sqlite();
  fleetSchema(sql);
  const store = new MemoryStore(() => "2026-11-21T00:00:00.000Z");
  store.fleet = (op, args) => fleetCall({ fleet: (o, a) => fleetOp(sql, o, a) }, op, args);
  const out = egress();
  const env = { ACCESS_TOKEN: TOKEN, CHATGPT_TOKEN: GPT, EVENTS_EGRESS: out, WAKE_EGRESS: out, ...extraEnv };
  const pending = [];
  const call = async (path, init) => {
    const res = await route(new Request(ORIGIN + path, init), env, () => store, (p) => pending.push(p));
    await Promise.all(pending.splice(0));
    return res;
  };
  const post = (path, body) =>
    call(path, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
  const rpc = async (token, method, params) =>
    (await post(`/mcp/${token}`, { jsonrpc: "2.0", id: 1, method, params })).json();
  const tool = async (token, name, args) => {
    const r = (await rpc(token, "tools/call", { name, arguments: args })).result;
    return r.isError ? { error: JSON.parse(r.content[0].text).error } : r.structuredContent;
  };
  const ft = await fleetToken(env);
  const agent = {
    hello: (body) => post(`/fleet/${ft}/hello`, body),
    inbox: async (name) => (await call(`/fleet/${ft}/inbox?agent=${name}`)).json(),
    send: (body) => post(`/fleet/${ft}/send`, body),
  };
  const setHook = (body) => post(`/api/${TOKEN}/wake-hooks`, body);
  return { store, out, env, call, post, rpc, tool, agent, setHook };
}

const claude = (h, args) => h.tool(TOKEN, "send_message", args);
const chatgpt = (h, args) => h.tool(GPT, "send_message", args);
const unread = async (h, token) => (await h.tool(token, "read_messages", { unread_only: true })).messages;
const fires = (h) => h.out.calls.filter((c) => String(c.url).startsWith("https://api.anthropic.com/"));

test("addresses: owner is the shown role term, drew and central stay aliases (#13507)", () => {
  assert.equal(replyAddress(null), "owner");
  assert.equal(replyAddress(undefined), "owner");
  assert.equal(replyAddress("agent:a1"), "agent:a1");
  assert.ok(ADDRESS_HELP.startsWith("owner,"), ADDRESS_HELP);
  for (const alias of ["owner", "drew", "central", "OWNER"]) {
    assert.equal(parseAddresses(alias, "agent:a1").drew, true, alias);
  }
});

test("addresses: names, groups, everyone but the sender, and refusals", () => {
  assert.deepEqual(parseAddresses(undefined, "channel:claude"), { drew: true, channels: [], fleet: [], all: false });
  assert.deepEqual(parseAddresses("ChatGPT, agent:muse-3, group:lanes", "channel:claude").channels, ["chatgpt"]);
  assert.deepEqual(parseAddresses("muse-3", "channel:claude").fleet, ["agent:muse-3"]);
  const all = parseAddresses("all", "channel:chatgpt");
  assert.deepEqual([all.channels, all.fleet, all.all, all.drew], [["claude"], ["all"], true, false]);
  assert.deepEqual(parseAddresses("connectors", "agent:a1").channels, ["claude", "chatgpt"]);
  assert.deepEqual(parseAddresses(["drew", "agents"], "agent:a1"), { drew: true, channels: [], fleet: ["all"], all: false });
  for (const [to, from] of [
    ["claude", "channel:claude"],
    ["channel:claude", "channel:claude"],
    ["agent:a1", "agent:a1"],
    ["a1", "agent:a1"],
    ["channel:slack", "channel:claude"],
    ["no way!", "channel:claude"],
    [" , ", "channel:claude"],
  ]) {
    assert.throws(() => parseAddresses(to, from), undefined, `${to} from ${from}`);
  }
});

test("Claude instructs ChatGPT: ChatGPT gets it as Drew's, is woken, and reply_to still answers Claude", async () => {
  const h = await harness();
  await h.rpc(GPT, "events/subscribe", {
    name: EVENT_NAME,
    arguments: {},
    delivery: { mode: "webhook", url: HOOK, secret: SECRET },
    cursor: null,
  });
  const sent = await claude(h, { body: "summarize the queue", to: "chatgpt", kind: "instruction" });
  assert.equal(sent.message, undefined, "nothing went to Drew");
  assert.equal(sent.messages.length, 1);
  const event = h.out.calls.at(-1);
  assert.equal(event.url, HOOK);
  assert.equal(event.body.data.from_addr, null, "the event names no sender");
  assert.equal(event.body.data.sender, null);
  assert.equal(event.body.data.kind, "instruction");

  const [got] = await unread(h, GPT);
  assert.equal(got.body, "summarize the queue");
  assert.equal(got.from_addr, null, "an instruction reads as Drew's");
  assert.equal(got.sender, undefined);
  assert.equal(got.source, undefined);
  assert.equal(got.kind, "instruction");
  const stored = (await h.store.list({ channel: "chatgpt" })).messages.find((m) => m.id === got.id);
  assert.equal(stored.from_addr, "channel:claude", "the bridge (and Drew's page) still know who sent it");
  assert.equal((await unread(h, TOKEN)).length, 0, "Claude's own inbox is untouched");

  const answer = await chatgpt(h, { body: "3 open items", reply_to: got.id });
  assert.equal(answer.messages[0].channel, "claude");
  assert.equal(answer.messages[0].from_addr, "channel:chatgpt");
  assert.equal((await unread(h, GPT)).length, 0, "reply_to marks the original read");
  const [back] = await unread(h, TOKEN);
  assert.equal(back.body, "3 open items");

  // Answering Drew's own message still goes to Drew.
  await h.post(`/api/${TOKEN}/messages`, { body: "hi Claude", to: "claude" });
  const fromDrew = (await unread(h, TOKEN)).find((m) => m.body === "hi Claude");
  assert.equal(fromDrew.from_addr, null);
  const toDrew = await claude(h, { body: "hi Drew", reply_to: fromDrew.id });
  assert.equal(toDrew.message.direction, "outbound");
  assert.equal(toDrew.message.channel, "claude");
  // Another channel's message can't be answered.
  assert.match((await claude(h, { body: "x", reply_to: got.id })).error, /not a message to Claude/);
});

test("a wake hook on Claude fires its routine with the routine headers and no message body", async () => {
  const h = await harness();
  assert.equal((await h.setHook({ address: "channel:claude", url: FIRE })).status, 400, "a routine needs its token");
  const set = await (await h.setHook({ address: "channel:claude", url: FIRE, token: ROUTINE_TOKEN })).json();
  assert.equal(set.hook.kind, "claude-routine");
  const listed = JSON.stringify(await (await h.call(`/api/${TOKEN}/wake-hooks`)).json());
  assert.ok(!listed.includes(ROUTINE_TOKEN) && !listed.includes("trig_"), "token and path stay hidden");
  assert.equal((await h.call(`/api/${GPT}/wake-hooks`)).status, 404);

  await chatgpt(h, { body: "secret plan", to: "claude", kind: "instruction" });
  const [fire] = fires(h);
  assert.equal(fire.url, FIRE);
  assert.equal(fire.init.headers.authorization, `Bearer ${ROUTINE_TOKEN}`);
  assert.equal(fire.init.headers["anthropic-version"], "2023-06-01");
  assert.equal(fire.init.headers["anthropic-beta"], "experimental-cc-routine-2026-04-01");
  assert.equal(fire.init.redirect, "manual");
  assert.deepEqual(Object.keys(fire.body), ["text"]);
  assert.match(fire.body.text, /for Claude/);
  assert.match(fire.body.text, /from Drew\b/, "an instruction wakes as Drew's");
  assert.ok(!fire.body.text.includes("ChatGPT"));
  assert.ok(!fire.body.text.includes("secret plan"));
  const log = await h.store.oauthEvents();
  assert.ok(log.some((e) => e.kind === "wake_fired" && e.address === "channel:claude" && e.status === 200));
  assert.ok(!JSON.stringify(log).includes(ROUTINE_TOKEN));

  // Drew's page and API wake Claude too; Claude-to-Drew wakes nobody.
  await h.post(`/api/${TOKEN}/messages`, { body: "from Drew", to: "claude" });
  assert.equal(fires(h).length, 2);
  await claude(h, { body: "to Drew" });
  assert.equal(fires(h).length, 2);

  assert.deepEqual(await (await h.setHook({ address: "channel:claude", url: null })).json(), { removed: true });
  await chatgpt(h, { body: "again", to: "claude" });
  assert.equal(fires(h).length, 2);
});

test("wakes per address are capped each hour, so two assistants can't ping-pong forever", async () => {
  const h = await harness({ WAKE_MAX_PER_HOUR: "2" });
  await h.setHook({ address: "channel:claude", url: FIRE, token: ROUTINE_TOKEN });
  for (let i = 0; i < 4; i++) await chatgpt(h, { body: `ping ${i}`, to: "claude" });
  assert.equal(fires(h).length, 2);
  assert.equal((await h.store.oauthEvents()).filter((e) => e.kind === "wake_capped").length, 2);
  assert.equal((await unread(h, TOKEN)).length, 4, "capped messages still wait, unread");
});

test("agents and connectors reach each other, and an agent answers whoever sent the row", async () => {
  const h = await harness();
  await h.agent.hello({ agent: "a1", groups: ["lanes"] });
  await h.agent.hello({ agent: "a2", groups: ["lanes"] });
  await h.setHook({ address: "agent:a2", url: "https://hooks.example.net/a2", token: "hooktoken123" });

  // Claude to everyone: ChatGPT and both agents, not Claude, not Drew.
  const all = await claude(h, { body: "status?", to: "all", kind: "instruction" });
  assert.deepEqual(all.messages.map((m) => m.channel), ["chatgpt"]);
  assert.deepEqual(all.fleet.map((m) => [m.agent, m.kind, m.from_addr]), [
    ["a1", "command", "channel:claude"],
    ["a2", "command", "channel:claude"],
  ]);
  const hook = h.out.calls.find((c) => c.url === "https://hooks.example.net/a2");
  assert.equal(hook.init.headers.authorization, "Bearer hooktoken123");
  assert.equal(hook.body.address, "agent:a2");
  assert.equal(hook.body.kind, "instruction");
  assert.deepEqual(hook.body.from, ["owner"], "the hook names no sender for an instruction");

  // a1 answers the row: it goes back to Claude, not to central.
  const inbox = await h.agent.inbox("a1");
  const row = inbox.messages[0];
  assert.equal(row.kind, "command");
  assert.equal(row.from_addr, null, "a command reads as Drew's");
  assert.equal(row.sender, null);
  const ans = await (await h.agent.send({ agent: "a1", body: "all green", reply_to: row.id })).json();
  assert.equal(ans.messages[0].channel, "claude");
  assert.equal(ans.messages[0].from_addr, "agent:a1");
  assert.equal(ans.messages[0].source, "fleet");
  const claudeGot = (await unread(h, TOKEN)).find((m) => m.body === "all green");
  assert.equal(claudeGot.from_addr, "agent:a1");

  // Claude answers a1 back by reply_to: lands in a1's mailbox.
  const back = await claude(h, { body: "thanks", reply_to: claudeGot.id });
  assert.equal(back.fleet[0].agent, "a1");
  assert.equal(back.fleet[0].from_addr, "channel:claude");

  // An agent instructs the group, itself excluded, and ChatGPT by name.
  const grp = await (await h.agent.send({ agent: "a1", body: "rebase", to: "group:lanes, chatgpt", kind: "command" })).json();
  assert.deepEqual(grp.fleet.map((m) => m.agent), ["a2"]);
  assert.equal(grp.messages[0].kind, "instruction");
  assert.equal((await h.agent.send({ agent: "a1", body: "x", to: "a1" })).status, 400);

  // A central row answered by reply_to still goes to central, as before.
  await h.post(`/api/${TOKEN}/fleet/send`, { body: "from Drew", to: "a2" });
  const central = (await h.agent.inbox("a2")).messages.find((m) => m.body === "from Drew");
  assert.equal(central.from_addr, null);
  const toCentral = await (await h.agent.send({ agent: "a2", body: "done", reply_to: central.id })).json();
  assert.equal(toCentral.message.direction, "outbound");

  // Drew gets messages, never instructions.
  assert.match((await claude(h, { body: "x", to: "drew", kind: "instruction" })).error, /not instructions/);
  assert.equal((await h.agent.send({ agent: "a1", body: "x", to: "drew", kind: "command" })).status, 400);
});

test("'all' with no agents registered still reaches the other connector", async () => {
  const h = await harness();
  const sent = await chatgpt(h, { body: "anyone?", to: "all" });
  assert.deepEqual(sent.messages.map((m) => m.channel), ["claude"]);
  assert.match((await chatgpt(h, { body: "x", to: "agents" })).error, /no agents/);
});

test("wake hooks are set and removed through the API (#14378: the page form is retired)", async () => {
  const h = await harness();
  assert.equal((await h.setHook({ address: "channel:claude", url: FIRE, token: ROUTINE_TOKEN })).status, 200);
  assert.equal((await h.store.wakeHooks()).length, 1);
  await chatgpt(h, { body: "peer row", to: "claude" });
  const { hooks } = await (await h.call(`/api/${TOKEN}/wake-hooks`)).json();
  assert.equal(hooks.length, 1);
  assert.equal(hooks[0].address, "channel:claude");
  assert.equal(hooks[0].kind, "claude-routine");
  assert.equal(hooks[0].host, "api.anthropic.com");
  assert.equal(hooks[0].has_token, true);
  const listed = JSON.stringify(hooks);
  assert.ok(!listed.includes(ROUTINE_TOKEN) && !listed.includes("trig_01"), "token and path stay hidden");
  assert.equal((await h.setHook({ address: "channel:claude", url: null })).status, 200);
  assert.equal((await h.store.wakeHooks()).length, 0);
  assert.equal((await h.setHook({ address: "channel:claude", url: "http://insecure.example" })).status, 400);
});

test("the wake text names ids and message senders, an instruction as Drew's, never bodies", () => {
  const t = wakeText("channel:claude", [
    { id: 4, kind: "instruction", from_addr: "agent:muse-3", body: "hidden" },
    { id: 5, kind: "message", from_addr: "channel:chatgpt", body: "hidden" },
  ]);
  assert.match(t, /ids 4, 5/);
  assert.match(t, /from Drew, ChatGPT/);
  assert.ok(!t.includes("muse-3"), "an instruction's sender stays hidden");
  assert.match(t, /including instructions/);
  assert.ok(!t.includes("hidden"));
});

test("addresses accept agent globs and refuse malformed ones", () => {
  assert.deepEqual(parseAddresses("agent:f*", "channel:claude").fleet, ["agent:f*"]);
  assert.throws(() => parseAddresses("agent:f o*", "channel:claude"), /unknown address/);
});

test("a connector sends to a glob with a limit, reports the match, then broadens", async () => {
  const { tool, agent } = await harness();
  await agent.hello({ agent: "fable" });
  await agent.hello({ agent: "falcon" });

  // Oracle (#14367) over the relay grammar: agent:f* with limit 1.
  const sent = await tool(TOKEN, "send_message", { body: "take the next task", to: "agent:f*", kind: "instruction", limit: 1 });
  assert.equal(sent.report.matched, 2);
  assert.equal(sent.report.delivered, 1);
  assert.deepEqual(sent.report.recipients, ["fable"]);
  assert.equal(sent.report.remaining, 1);
  assert.equal((await agent.inbox("fable")).messages.length, 1);
  assert.equal((await agent.inbox("falcon")).messages.length, 0);

  const b1 = await tool(TOKEN, "send_message", { broaden: sent.report.batch, limit: 2 });
  assert.equal(b1.report.delivered, 1);
  assert.deepEqual(b1.report.recipients, ["falcon"]);
  assert.equal(b1.report.remaining, 0);

  const b2 = await tool(TOKEN, "send_message", { broaden: sent.report.batch, limit: 2 });
  assert.equal(b2.report.delivered, 0);
  assert.equal((await agent.inbox("falcon")).messages.length, 1);
});
