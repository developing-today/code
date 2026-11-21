// The ChatGPT connector's Site Worker (pkgs/dot-bridge/site) wired to
// this bridge's own router: ChatGPT's tool calls run the same code as
// Claude's and land in the same store, on the ChatGPT channel. The nix
// package builds this directory alone, so there the test is skipped.
import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { test } from "node:test";
import { MemoryStore } from "../src/messages.js";
import { route } from "../src/routes.js";

const OWNER = "o".repeat(40);
const GPT = "g".repeat(40);
const bridgeEnv = { ACCESS_TOKEN: OWNER, CHATGPT_TOKEN: GPT };

const sitePath = new URL("../../../dot-bridge/site/worker/index.js", import.meta.url);

test("ChatGPT's Site sends and reads through the bridge's own code", { skip: !existsSync(sitePath) }, async (t) => {
  const { default: site } = await import(sitePath.href);
  const store = new MemoryStore(() => "2026-11-19T00:00:00.000Z");
  const realFetch = globalThis.fetch;
  globalThis.fetch = (url, init) => route(new Request(url, init), bridgeEnv, () => store);
  t.after(() => {
    globalThis.fetch = realFetch;
  });
  const siteEnv = { MESSAGE_BRIDGE_TOKEN: GPT };
  let id = 0;
  const gpt = async (method, params) => {
    const res = await site.fetch(
      new Request("https://site.invalid/mcp", {
        method: "POST",
        headers: { "oai-authenticated-user-id": "drew" },
        body: JSON.stringify({ jsonrpc: "2.0", id: ++id, method, params }),
      }),
      siteEnv,
    );
    assert.equal(res.status, 200);
    return (await res.json()).result;
  };
  const owner = (path, init) => route(new Request("https://bridge.invalid" + path, init), bridgeEnv, () => store);

  assert.deepEqual(
    (await gpt("tools/list", {})).tools.map((x) => x.name),
    ["send_message", "read_messages", "ack_messages"],
  );
  // Drew sends one message to both from the page.
  await owner(`/api/${OWNER}/messages`, {
    method: "POST",
    body: JSON.stringify({ body: "hello both", to: ["claude", "chatgpt"], source: "page" }),
  });
  const read = (await gpt("tools/call", { name: "read_messages", arguments: { unread_only: true, mark_read: true } }))
    .structuredContent;
  assert.deepEqual(
    read.messages.map((m) => [m.body, m.channel]),
    [["hello both", "chatgpt"]],
  );
  const sent = (await gpt("tools/call", { name: "send_message", arguments: { body: "hi from chatgpt" } }))
    .structuredContent.message;
  assert.equal(sent.channel, "chatgpt");
  assert.equal(sent.direction, "outbound");

  const history = await (await owner(`/api/${OWNER}/messages?direction=all`)).json();
  assert.deepEqual(
    history.messages.map((m) => [m.channel, m.direction, m.read_at === null]),
    [
      ["claude", "inbound", true],
      ["chatgpt", "inbound", false],
      ["chatgpt", "outbound", true],
    ],
  );
});
