import assert from "node:assert/strict";
import { test } from "node:test";
import worker from "../src/index.js";

test("GET /queue forwards cursor and exposes KV pagination metadata", async () => {
  const listCalls = [];
  const values = new Map([
    ["ev:first", JSON.stringify({ id: "first", repo: "owner/first" })],
    ["ev:second", JSON.stringify({ id: "second", repo: "owner/second" })],
  ]);
  const kv = {
    async list(options) {
      listCalls.push(options);
      if (options.cursor === "page two/+") {
        return {
          keys: [{ name: "ev:second" }],
          cursor: null,
          list_complete: true,
        };
      }
      return {
        keys: [{ name: "ev:first" }],
        cursor: "page two/+",
        list_complete: false,
      };
    },
    async get(name) {
      return values.get(name) ?? null;
    },
  };
  const env = { GHB_EVENTS: kv, DRAIN_BEARER: "test-bearer" };
  const request = (cursor) => new Request(
    "https://worker.test/queue" + (cursor === undefined
      ? "" : "?cursor=" + encodeURIComponent(cursor)),
    { headers: { Authorization: "Bearer test-bearer" } },
  );

  const first = await worker.fetch(request(undefined), env);
  assert.equal(first.status, 200);
  assert.deepEqual(await first.json(), {
    events: [{ id: "first", repo: "owner/first" }],
    cursor: "page two/+",
    list_complete: false,
  });

  const second = await worker.fetch(request("page two/+"), env);
  assert.equal(second.status, 200);
  assert.deepEqual(await second.json(), {
    events: [{ id: "second", repo: "owner/second" }],
    cursor: null,
    list_complete: true,
  });
  assert.deepEqual(listCalls, [
    { prefix: "ev:", limit: 500 },
    { prefix: "ev:", limit: 500, cursor: "page two/+" },
  ]);
});

test("GET /queue still requires the bearer token", async () => {
  let listed = false;
  const response = await worker.fetch(
    new Request("https://worker.test/queue"),
    { GHB_EVENTS: { async list() { listed = true; return { keys: [] }; } }, DRAIN_BEARER: "secret" },
  );
  assert.equal(response.status, 401);
  assert.equal(listed, false);
});
