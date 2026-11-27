import { test } from "node:test";
import assert from "node:assert/strict";
import { FailClosed, blobsBindingFor, parseBindingId, parseJurisdiction, userStub } from "../src/routing.ts";
import {
  createHostedAccountRecord,
  HostedAccountDirectory,
  inventoryLegacyHostedAccount,
  updateHostedAccountRecord,
} from "../src/account-directory.ts";

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
      () =>
        createHostedAccountRecord({
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
    () =>
      createHostedAccountRecord({ subjectKey: "provider-subject-hash", bindingId: "google:sub", jurisdiction: "us" }),
    (error: unknown) => error instanceof FailClosed,
  );
});

test("profile update changing jurisdiction fails closed and leaves the record unchanged", () => {
  const record = createHostedAccountRecord({
    subjectKey: "provider-subject-hash",
    bindingId: BINDING,
    jurisdiction: "us",
  });
  assert.throws(
    () => updateHostedAccountRecord(record, { jurisdiction: "eu" }),
    (error: unknown) => error instanceof FailClosed,
  );
  assert.deepEqual(record, {
    subjectKey: "provider-subject-hash",
    bindingId: BINDING,
    jurisdiction: "us",
    jurisdictionPending: null,
    migrationId: null,
    phase: "stable",
    routingEpoch: 0,
  });
});

test("profile update with the same or no jurisdiction succeeds with routing metadata unchanged", () => {
  const record = createHostedAccountRecord({
    subjectKey: "provider-subject-hash",
    bindingId: BINDING,
    jurisdiction: "us",
  });
  assert.deepEqual(updateHostedAccountRecord(record, { jurisdiction: "us" }), record);
  assert.deepEqual(updateHostedAccountRecord(record, {}), record);
  assert.deepEqual(updateHostedAccountRecord(record, { jurisdiction: null }), record);
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

test("legacy inventory records a pre-existing account without assigning a jurisdiction", () => {
  const directory = new HostedAccountDirectory();
  const entry = directory.inventoryLegacy({
    subjectKey: "provider-subject-hash",
    bindingId: BINDING,
  });
  assert.deepEqual(entry, {
    subjectKey: "provider-subject-hash",
    bindingId: BINDING,
    jurisdiction: null,
    classification: "legacy-unrestricted",
  });
  assert.equal(directory.legacySize, 1);
  assert.equal(directory.size, 0);
  assert.deepEqual(directory.legacyInventory("provider-subject-hash"), entry);
  assert.equal(directory.legacyInventory("unknown-subject"), undefined);
});

test("legacy inventory refuses to relabel an account with an asserted jurisdiction", () => {
  for (const jurisdiction of ["us", "eu", "apac"]) {
    assert.throws(
      () => inventoryLegacyHostedAccount({ subjectKey: "provider-subject-hash", bindingId: BINDING, jurisdiction }),
      (error: unknown) => error instanceof FailClosed && error.code === "legacy_jurisdiction_unverifiable",
    );
  }
  const directory = new HostedAccountDirectory();
  assert.throws(
    () => directory.inventoryLegacy({ subjectKey: "provider-subject-hash", bindingId: BINDING, jurisdiction: "us" }),
    (error: unknown) => error instanceof FailClosed && error.code === "legacy_jurisdiction_unverifiable",
  );
  assert.equal(directory.legacySize, 0);
});

test("legacy inventory rejects invalid identity inputs", () => {
  assert.throws(
    () => inventoryLegacyHostedAccount({ subjectKey: "", bindingId: BINDING }),
    (error: unknown) => error instanceof FailClosed,
  );
  assert.throws(
    () => inventoryLegacyHostedAccount({ subjectKey: "provider-subject-hash", bindingId: "google:sub" }),
    (error: unknown) => error instanceof FailClosed,
  );
});

test("duplicate legacy inventory returns the preserved entry; conflicting binding fails closed", () => {
  const directory = new HostedAccountDirectory();
  const first = directory.inventoryLegacy({ subjectKey: "provider-subject-hash", bindingId: BINDING });
  const second = directory.inventoryLegacy({ subjectKey: "provider-subject-hash", bindingId: BINDING });
  assert.deepEqual(second, first);
  assert.equal(directory.legacySize, 1);
  assert.throws(
    () => directory.inventoryLegacy({ subjectKey: "provider-subject-hash", bindingId: BINDING_OTHER }),
    (error: unknown) => error instanceof FailClosed && error.code === "duplicate_subject_conflict",
  );
  assert.throws(
    () => directory.inventoryLegacy({ subjectKey: "provider-subject-hash", bindingId: BINDING, jurisdiction: "us" }),
    (error: unknown) => error instanceof FailClosed && error.code === "legacy_jurisdiction_unverifiable",
  );
  assert.equal(directory.legacySize, 1);
  assert.deepEqual(directory.legacyInventory("provider-subject-hash"), first);
});

test("signup for a legacy-inventoried subject fails closed instead of creating a second store", () => {
  const directory = new HostedAccountDirectory();
  const entry = directory.inventoryLegacy({ subjectKey: "provider-subject-hash", bindingId: BINDING });
  assert.throws(
    () => directory.signup({ subjectKey: "provider-subject-hash", bindingId: BINDING, jurisdiction: "us" }),
    (error: unknown) => error instanceof FailClosed && error.code === "legacy_subject_requires_migration",
  );
  assert.equal(directory.size, 0);
  assert.equal(directory.legacySize, 1);
  assert.deepEqual(directory.legacyInventory("provider-subject-hash"), entry);
});

test("inventorying an already-registered subject as legacy fails closed", () => {
  const directory = new HostedAccountDirectory();
  const record = directory.signup({ subjectKey: "provider-subject-hash", bindingId: BINDING, jurisdiction: "eu" });
  assert.throws(
    () => directory.inventoryLegacy({ subjectKey: "provider-subject-hash", bindingId: BINDING }),
    (error: unknown) => error instanceof FailClosed && error.code === "legacy_subject_already_registered",
  );
  assert.equal(directory.size, 1);
  assert.equal(directory.legacySize, 0);
  assert.deepEqual(
    directory.signup({ subjectKey: "provider-subject-hash", bindingId: BINDING, jurisdiction: "eu" }),
    record,
  );
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

function fakeBucket() {
  return {
    head: async () => null,
    get: async () => null,
    put: async () => {},
    delete: async () => {},
  };
}

test("blobsBindingFor picks the bucket strictly from the authoritative jurisdiction", () => {
  const us = fakeBucket();
  const eu = fakeBucket();
  const fedramp = fakeBucket();
  assert.equal(blobsBindingFor({ BLOBS_US: us }, "us"), us);
  assert.equal(blobsBindingFor({ BLOBS_US: us, BLOBS_EU: eu }, "eu"), eu);
  assert.equal(blobsBindingFor({ BLOBS_FEDRAMP: fedramp }, "fedramp"), fedramp);
});

test("blobsBindingFor fails closed on a missing binding with no fallback", () => {
  const us = fakeBucket();
  assert.throws(
    () => blobsBindingFor({ BLOBS_US: us }, "eu"),
    (e: unknown) => e instanceof FailClosed && (e as FailClosed).code === "blobs_unavailable",
  );
  assert.throws(
    () => blobsBindingFor({}, "us"),
    (e: unknown) => e instanceof FailClosed && (e as FailClosed).code === "blobs_unavailable",
  );
});
