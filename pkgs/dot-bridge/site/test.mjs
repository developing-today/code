import assert from "node:assert/strict";
import worker from "./worker/index.js";

// A stand-in for the message bridge: records what reaches it and answers
// like its /mcp. The full round trip against the bridge's own code is in
// pkgs/message-bridge/worker/test/chatgpt-site.test.mjs.
const TOKEN = "c".repeat(40);
const seen = [];
globalThis.fetch = async (url, init) => {
  seen.push({ url: String(url), init });
  if (!String(url).endsWith(`/mcp/${TOKEN}`)) return new Response("Not found\n", { status: 404 });
  const msg = JSON.parse(init.body);
  if (!("id" in msg)) return new Response(null, { status: 202 });
  return Response.json({ jsonrpc: "2.0", id: msg.id, result: { echoed: msg.method } });
};

const rows = new Map();
const bucket = { get: async (k) => (rows.has(k) ? { text: async () => rows.get(k) } : null) };
const post = (body, { user = "owner-a", env = { BUCKET: bucket }, method = "POST", path = "/mcp" } = {}) =>
  worker.fetch(
    new Request("https://fixture.invalid" + path, {
      method,
      headers: { ...(user ? { "oai-authenticated-user-id": user } : {}), authorization: "Bearer caller-secret" },
      body: method === "POST" ? (typeof body === "string" ? body : JSON.stringify(body)) : undefined,
    }),
    env,
  );
const rpc = (method, id = 1) => ({ jsonrpc: "2.0", id, method, params: {} });

// Sites sign-in stays the gate.
assert.equal((await post(rpc("tools/list"), { user: "" })).status, 401);
assert.equal((await post(rpc("tools/list"), { user: "x".repeat(301) })).status, 401);
assert.equal((await post(null, { method: "GET" })).status, 405);
assert.equal((await post(null, { method: "GET", path: "/other" })).status, 404);
assert.equal(
  (await post(rpc("tools/list"), { env: { BUCKET: bucket, OWNER_USER_ID: "owner-b", MESSAGE_BRIDGE_TOKEN: TOKEN } })).status,
  403,
);
assert.equal((await post("x".repeat(64 * 1024 + 1))).status, 413);
assert.equal((await post("{nope")).status, 400);
assert.equal(seen.length, 0, "nothing reaches the bridge before the checks pass");

// Without a token it says so instead of calling out.
let r = await (await post(rpc("tools/list", 7))).json();
assert.equal(r.id, 7);
assert.match(r.error.message, /isn't configured/);
assert.equal(seen.length, 0);

// Token from R2 when there's no Site secret; forwarded body only.
rows.set("config/message-bridge-token", TOKEN + "\n");
r = await (await post(rpc("tools/list", 8))).json();
assert.deepEqual(r, { jsonrpc: "2.0", id: 8, result: { echoed: "tools/list" } });
assert.equal(seen[0].url, `https://bridge.security.cab/mcp/${TOKEN}`);
assert.equal(new Headers(seen[0].init.headers).get("authorization"), null, "caller credentials are not forwarded");
assert.equal(new Headers(seen[0].init.headers).get("oai-authenticated-user-id"), null);

// A Site secret wins; notifications pass through as 202.
assert.equal((await post({ jsonrpc: "2.0", method: "notifications/initialized" }, { env: { MESSAGE_BRIDGE_TOKEN: TOKEN } })).status, 202);

// A token the bridge refuses never shows up in the answer.
r = await (await post(rpc("tools/list", 9), { env: { MESSAGE_BRIDGE_TOKEN: "w".repeat(40) } })).json();
assert.match(r.error.message, /didn't accept/);
assert.doesNotMatch(JSON.stringify(r), /w{40}|c{40}/);

// A short or odd token counts as none.
r = await (await post(rpc("tools/list", 10), { env: { MESSAGE_BRIDGE_TOKEN: "short" } })).json();
assert.match(r.error.message, /isn't configured/);

console.log("ChatGPT Site pass-through checks passed");
