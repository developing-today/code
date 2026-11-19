import assert from "node:assert/strict";
import { test } from "node:test";
import { matchTokenPath, safeEqual } from "../src/auth.js";
import { MemoryStore } from "../src/messages.js";
import { renderPage } from "../src/page.js";
import { route } from "../src/routes.js";

const TOKEN = "t".repeat(20) + "0123456789abcdefXYZ_-";
const env = { ACCESS_TOKEN: TOKEN };
const BASE = "https://bridge.example";

function harness() {
  const store = new MemoryStore(() => "2026-11-19T00:00:00.000Z");
  const call = (path, init) => route(new Request(BASE + path, init), env, () => store);
  let rpcId = 0;
  const rpc = async (method, params) => {
    const res = await call(`/mcp/${TOKEN}`, {
      method: "POST",
      headers: { "content-type": "application/json", accept: "application/json, text/event-stream" },
      body: JSON.stringify({ jsonrpc: "2.0", id: ++rpcId, method, params }),
    });
    assert.equal(res.status, 200);
    assert.match(res.headers.get("content-type"), /application\/json/);
    return res.json();
  };
  const tool = async (name, args) => (await rpc("tools/call", { name, arguments: args })).result;
  return { store, call, rpc, tool };
}

test("safeEqual and token paths", () => {
  assert.ok(safeEqual("abc", "abc"));
  assert.ok(!safeEqual("abc", "abd"));
  assert.ok(!safeEqual("abc", "abcd"));
  assert.deepEqual(matchTokenPath(`/api/${TOKEN}/messages`, env), { area: "api", rest: ["messages"] });
  assert.equal(matchTokenPath(`/api/wrong/messages`, env), null);
  assert.equal(matchTokenPath(`/mcp/${TOKEN}`, { ACCESS_TOKEN: "short" }), null, "short token disables access");
  assert.equal(matchTokenPath(`/mcp/${TOKEN}`, {}), null, "no token disables access");
});

test("wrong or missing token is a plain 404 everywhere", async () => {
  const { call } = harness();
  for (const path of ["/", "/mcp", "/mcp/nope", "/p/nope", "/api/nope/messages", `/x/${TOKEN}`]) {
    assert.equal((await call(path, { method: "POST", body: "{}" })).status, 404, path);
    assert.equal((await call(path)).status, 404, path);
  }
  const health = await call("/healthz");
  assert.deepEqual(await health.json(), { ok: true, configured: true });
});

test("MCP initialize, tools/list, notifications, unknown method", async () => {
  const { rpc, call } = harness();
  const init = await rpc("initialize", {
    protocolVersion: "2025-06-18",
    capabilities: {},
    clientInfo: { name: "test", version: "1" },
  });
  assert.equal(init.result.protocolVersion, "2025-06-18");
  assert.equal(init.result.serverInfo.name, "message-bridge");
  assert.ok(init.result.capabilities.tools);
  const odd = await rpc("initialize", { protocolVersion: "1999-01-01" });
  assert.equal(odd.result.protocolVersion, "2025-11-25");

  const notif = await call(`/mcp/${TOKEN}`, {
    method: "POST",
    body: JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" }),
  });
  assert.equal(notif.status, 202);

  const tools = await rpc("tools/list", {});
  assert.deepEqual(
    tools.result.tools.map((t) => t.name),
    ["send_message", "read_messages", "ack_messages"],
  );
  assert.equal((await rpc("nope/nope", {})).error.code, -32601);
  assert.equal((await rpc("ping")).result && true, true);

  const get = await call(`/mcp/${TOKEN}`);
  assert.equal(get.status, 405);
  const bad = await call(`/mcp/${TOKEN}`, { method: "POST", body: "{not json" });
  assert.equal(bad.status, 400);
  assert.equal((await bad.json()).error.code, -32700);
});

test("send, receive, mark read and ack through MCP; page and API share the store", async () => {
  const { tool, call, store } = harness();
  const sent = await tool("send_message", { body: "hello from claude", sender: "claude" });
  assert.equal(sent.isError, undefined);
  assert.equal(sent.structuredContent.message.direction, "outbound");
  assert.equal(sent.structuredContent.message.source, "mcp");

  // Page form post -> inbound, 303 back to the page.
  const form = await call(`/p/${TOKEN}`, {
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({ body: "hi claude <b>", sender: "Drew" }).toString(),
  });
  assert.equal(form.status, 303);
  assert.equal(form.headers.get("location"), `/p/${TOKEN}`);
  assert.equal(form.headers.get("referrer-policy"), "no-referrer");

  // JSON API post -> inbound via api.
  const api = await call(`/api/${TOKEN}/messages`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ body: "from a script" }),
  });
  assert.equal(api.status, 201);
  assert.equal((await api.json()).message.source, "api");

  const unread = await tool("read_messages", { unread_only: true });
  assert.deepEqual(
    unread.structuredContent.messages.map((m) => [m.body, m.source]),
    [
      ["hi claude <b>", "page"],
      ["from a script", "api"],
    ],
  );
  assert.equal(unread.structuredContent.stats.unread_inbound, 2);
  assert.equal(unread.structuredContent.next_since_id, 3);

  const marked = await tool("read_messages", { unread_only: true, limit: 1, mark_read: true });
  assert.equal(marked.structuredContent.acked, 1);
  assert.equal(marked.structuredContent.stats.unread_inbound, 1);

  const acked = await tool("ack_messages", { up_to_id: 3 });
  assert.equal(acked.structuredContent.acked, 1);
  assert.equal(acked.structuredContent.stats.unread_inbound, 0);

  const outbound = await tool("read_messages", { direction: "outbound" });
  assert.equal(outbound.structuredContent.messages.length, 1);
  const after = await tool("read_messages", { direction: "all", since_id: 1 });
  assert.deepEqual(
    after.structuredContent.messages.map((m) => m.id),
    [2, 3],
  );

  // History JSON, newest first.
  const hist = await (await call(`/api/${TOKEN}/messages?order=desc`)).json();
  assert.deepEqual(
    hist.messages.map((m) => m.id),
    [3, 2, 1],
  );
  assert.equal(hist.stats.total, 3);

  // Page renders every message, escaped, with the private headers.
  const page = await call(`/p/${TOKEN}`);
  assert.equal(page.status, 200);
  assert.match(page.headers.get("content-security-policy"), /frame-ancestors 'none'/);
  assert.equal(page.headers.get("cache-control"), "no-store");
  const html = await page.text();
  assert.ok(html.includes("hi claude &lt;b&gt;"));
  assert.ok(!html.includes("hi claude <b>"));
  assert.ok(html.includes("hello from claude"));
  assert.ok(html.includes("from a script"));
  assert.equal(store.rows.length, 3);
});

test("bad input is a tool error or a 400, never stored", async () => {
  const { tool, call, store } = harness();
  const empty = await tool("send_message", { body: "   " });
  assert.equal(empty.isError, true);
  const extra = await tool("send_message", { body: "x", to: "someone" });
  assert.equal(extra.isError, true);
  const big = await tool("send_message", { body: "x".repeat(16001) });
  assert.equal(big.isError, true);
  const badDir = await tool("read_messages", { direction: "sideways" });
  assert.equal(badDir.isError, true);
  const noSel = await tool("ack_messages", {});
  assert.equal(noSel.isError, true);
  const r = await call(`/api/${TOKEN}/messages`, { method: "POST", body: "nope" });
  assert.equal(r.status, 400);
  const r2 = await call(`/api/${TOKEN}/messages?limit=0`);
  assert.equal(r2.status, 400);
  assert.equal(store.rows.length, 0);
});

test("page script cannot be broken out of by the api path", () => {
  const html = renderPage({
    messages: [],
    stats: { total: 0, inbound: 0, outbound: 0, unread_inbound: 0 },
    basePath: "/p/x",
    apiPath: "/api/</script><script>alert(1)",
  });
  assert.ok(!html.includes("</script><script>alert(1)"));
});
