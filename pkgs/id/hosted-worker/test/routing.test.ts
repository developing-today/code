import { test } from "node:test";
import assert from "node:assert/strict";
import { FailClosed, parseBindingId, parseJurisdiction, userStub } from "../src/routing.ts";

const BINDING = "0123456789abcdef0123456789abcdef";

test("binding id must be a minted 32-hex id", () => {
  assert.equal(parseBindingId(BINDING), BINDING);
  assert.throws(() => parseBindingId(null), FailClosed);
  assert.throws(() => parseBindingId("google:123"), FailClosed);
  assert.throws(() => parseBindingId("user@example.com"), FailClosed);
  assert.throws(() => parseBindingId(BINDING.toUpperCase()), FailClosed);
});

test("jurisdiction must be present and supported", () => {
  assert.equal(parseJurisdiction("us"), "us");
  assert.equal(parseJurisdiction("eu"), "eu");
  assert.throws(() => parseJurisdiction(null), FailClosed);
  assert.throws(() => parseJurisdiction(""), FailClosed);
  assert.throws(() => parseJurisdiction("apac"), FailClosed);
});

test("userStub takes id and stub from the restricted namespace only", () => {
  const calls: string[] = [];
  const restricted = {
    jurisdiction(j: string) {
      calls.push("jurisdiction:" + j);
      return restricted;
    },
    idFromName(name: string) {
      calls.push("idFromName:" + name);
      return { name };
    },
    get(id: unknown) {
      calls.push("get");
      return { fetch: async () => new Response("ok") };
    },
  };
  userStub(restricted, BINDING, "eu");
  assert.deepEqual(calls, ["jurisdiction:eu", "idFromName:" + BINDING, "get"]);
});

test("userStub fails closed when restriction is unavailable", () => {
  const ns = {
    jurisdiction(): never {
      throw new Error("Jurisdiction restrictions are not implemented in workerd.");
    },
    idFromName(name: string) {
      return { name };
    },
    get(id: unknown) {
      return { fetch: async () => new Response("unrestricted") };
    },
  };
  assert.throws(
    () => userStub(ns, BINDING, "us"),
    (e: unknown) => e instanceof FailClosed && e.status === 503,
  );
});
