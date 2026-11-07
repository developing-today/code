import assert from "node:assert/strict";
import { test } from "node:test";
import worker from "../src/index.js";

test("GET /queue forwards cursor and exposes KV pagination metadata", async () => {
  const listCalls = [];
  const getCalls = [];
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
    async get(nameOrNames) {
      if (Array.isArray(nameOrNames)) {
        getCalls.push(nameOrNames);
        return new Map(nameOrNames.map((n) => [n, values.get(n) ?? null]));
      }
      return values.get(nameOrNames) ?? null;
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
  // Page values come from bulk reads (#5496): every get() carried an
  // array of key names, never a single sequential lookup.
  assert.deepEqual(getCalls, [["ev:first"], ["ev:second"]]);
});

test("GET /queue reads a full page in bulk chunks of at most 100 keys", async () => {
  const total = 250;
  const names = Array.from({ length: total }, (_, i) => `ev:e${i}`);
  const chunkSizes = [];
  const kv = {
    async list() {
      return {
        keys: names.map((name) => ({ name })),
        cursor: null,
        list_complete: true,
      };
    },
    async get(nameOrNames) {
      assert.ok(Array.isArray(nameOrNames), "page reads must be bulk");
      chunkSizes.push(nameOrNames.length);
      return new Map(
        nameOrNames.map((n) => [n, JSON.stringify({ id: n.slice(3) })]),
      );
    },
  };
  const env = { GHB_EVENTS: kv, DRAIN_BEARER: "test-bearer" };
  const response = await worker.fetch(
    new Request("https://worker.test/queue", {
      headers: { Authorization: "Bearer " + "test-bearer" },
    }),
    env,
  );
  assert.equal(response.status, 200);
  const body = await response.json();
  assert.equal(body.events.length, total);
  assert.equal(body.events[0].id, "e0");
  assert.equal(body.events[total - 1].id, `e${total - 1}`);
  assert.deepEqual(chunkSizes, [100, 100, 50]);
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

test("POST /queue/ack deletes every id and reports the count", async () => {
  const deleted = [];
  const kv = {
    async delete(name) {
      deleted.push(name);
    },
  };
  const env = { GHB_EVENTS: kv, DRAIN_BEARER: "test-bearer" };
  const response = await worker.fetch(
    new Request("https://worker.test/queue/ack", {
      method: "POST",
      headers: {
        Authorization: "Bearer " + "test-bearer",
        "Content-Type": "application/json",
      },
      body: JSON.stringify({ ids: ["a", "b", "c"] }),
    }),
    env,
  );
  assert.equal(response.status, 200);
  assert.deepEqual(await response.json(), { acked: 3 });
  assert.deepEqual([...deleted].sort(), ["ev:a", "ev:b", "ev:c"]);
});

test("POST /queue/ack requires auth", async () => {
  let deleted = false;
  const response = await worker.fetch(
    new Request("https://worker.test/queue/ack", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ ids: ["a"] }),
    }),
    {
      GHB_EVENTS: {
        async delete() {
          deleted = true;
        },
      },
      DRAIN_BEARER: "secret",
    },
  );
  assert.equal(response.status, 401);
  assert.equal(deleted, false);
});
