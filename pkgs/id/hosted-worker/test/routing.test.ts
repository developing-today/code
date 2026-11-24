import { test } from "node:test";
import assert from "node:assert/strict";
import { FailClosed, parseBindingId, parseJurisdiction, userStub } from "../src/routing.ts";
import { createHostedAccountRecord, HostedAccountDirectory } from "../src/account-directory.ts";

const BINDING = "0123456789abcdef0123456789abcdef";
const BINDING_OTHER = "fedcba9876543210fedcba9876543210";

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

test("hosted account metadata requires an explicit supported jurisdiction", () => {
  const record = createHostedAccountRecord({
    subjectKey: "provider-subject-hash",
    bindingId: BINDING,
    jurisdiction: "eu",
  });
  assert.deepEqual(record, {
    subjectKey: "provider-subject-hash",
    bindingId: BINDING,
    jurisdiction: "eu",
    jurisdictionPending: null,
    migrationId: null,
    phase: "stable",
    routingEpoch: 0,
  });
});

test("hosted account creation fails closed before metadata exists for absent or invalid jurisdiction", () => {
  for (const jurisdiction of [null, "", "apac"]) {
    assert.throws(
      () => createHostedAccountRecord({
        subjectKey: "provider-subject-hash",
        bindingId: BINDING,
        jurisdiction,
      }),
      (error: unknown) => error instanceof FailClosed,
    );
  }
});

test("hosted account metadata rejects invalid identity inputs", () => {
  assert.throws(
    () => createHostedAccountRecord({ subjectKey: "", bindingId: BINDING, jurisdiction: "us" }),
    (error: unknown) => error instanceof FailClosed,
  );
  assert.throws(
    () => createHostedAccountRecord({ subjectKey: "provider-subject-hash", bindingId: "google:sub", jurisdiction: "us" }),
    (error: unknown) => error instanceof FailClosed,
  );
});

test("duplicate signup reads the preserved record instead of overwriting it", () => {
  const directory = new HostedAccountDirectory();
  const first = directory.signup({
    subjectKey: "provider-subject-hash",
    bindingId: BINDING,
    jurisdiction: "eu",
  });
  const second = directory.signup({
    subjectKey: "provider-subject-hash",
    bindingId: BINDING,
    jurisdiction: "eu",
  });
  assert.deepEqual(second, first);
  assert.equal(directory.size, 1);
});

test("duplicate signup with a conflicting binding id fails closed", () => {
  const directory = new HostedAccountDirectory();
  directory.signup({
    subjectKey: "provider-subject-hash",
    bindingId: BINDING,
    jurisdiction: "eu",
  });
  assert.throws(
    () =>
      directory.signup({
        subjectKey: "provider-subject-hash",
        bindingId: BINDING_OTHER,
        jurisdiction: "eu",
      }),
    (error: unknown) => error instanceof FailClosed && error.code === "duplicate_subject_conflict",
  );
  assert.equal(directory.size, 1);
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
