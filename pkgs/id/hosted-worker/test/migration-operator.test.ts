// #3404 slice 5: the operator route is secret-gated, strict about its
// command shape, and dispatches only into the existing resumable coordinator.
import { test } from "node:test";
import assert from "node:assert/strict";
import worker from "../src/worker.ts";
import { handleMigrationOperatorRequest } from "../src/migration-operator.ts";
import type { MigrationPorts } from "../src/migration-coordinator.ts";

const TOKEN = "test-operator-token";
const BINDING = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

function fixture(phase: string = "stable", jurisdiction: "us" | "eu" = "us") {
  const record = {
    subjectKey: "subject-hash",
    bindingId: BINDING,
    jurisdiction,
    jurisdictionPending: null as "us" | "eu" | null,
    migrationId: null as string | null,
    phase,
    routingEpoch: 0,
  };
  const calls: Array<{ method: string; path: string; owner?: string }> = [];
  const directory = {
    async findByBindingId(bindingId: string) {
      return bindingId === BINDING ? record : null;
    },
    async get(subjectKey: string) {
      return subjectKey === record.subjectKey ? record : null;
    },
    async acquireLease(_subjectKey: string, owner: string) {
      calls.push({ method: "lease", path: "acquire", owner });
      return { owner, token: 1, expiresAtMs: Date.now() + 60_000 };
    },
    async releaseLease() {
      calls.push({ method: "lease", path: "release" });
    },
    async control(_record: unknown, _jurisdiction: unknown, method: string, path: string) {
      calls.push({ method, path });
      return Response.json({});
    },
  };
  const ports = {
    directory,
    control: directory.control,
    async copyBlob() {},
    now: () => Date.now(),
  } as unknown as MigrationPorts;
  return { ports, calls, record };
}

function request(action: string, body: unknown, token: string = TOKEN): Request {
  return new Request(`https://hosted.internal/v1/internal/migrations/${action}`, {
    method: "POST",
    headers: { authorization: `Bearer ${token}`, "content-type": "application/json" },
    body: JSON.stringify(body),
  });
}

test("operator route refuses when disabled and does not inspect client binding headers", async () => {
  const response = await worker.fetch(
    new Request("https://hosted.internal/v1/internal/migrations/start", {
      method: "POST",
      headers: {
        "x-binding-id": BINDING,
        "x-jurisdiction": "us",
        "x-hosted-migration-principal": "1",
      },
    }),
    {
      USERS: {
        idFromName: () => {
          throw new Error("must not route to a user object");
        },
      },
    } as never,
  );
  assert.equal(response.status, 503);
  assert.equal(((await response.json()) as { error: string }).error, "migration_operator_disabled");
  const whitespaceSecret = await worker.fetch(
    new Request("https://hosted.internal/v1/internal/migrations/start", { method: "POST" }),
    {
      USERS: {
        idFromName: () => {
          throw new Error("must not route to a user object");
        },
      },
      MIGRATION_OPERATOR_TOKEN: "   ",
    } as never,
  );
  assert.equal(whitespaceSecret.status, 503);
});

test("operator route requires the dedicated bearer credential", async () => {
  const { ports, calls } = fixture();
  const response = await handleMigrationOperatorRequest(
    request("start", {}, "wrong"),
    { MIGRATION_OPERATOR_TOKEN: TOKEN },
    ports,
  );
  assert.equal(response.status, 401);
  assert.equal(((await response.json()) as { error: string }).error, "operator_auth_required");
  assert.equal(calls.length, 0);
});

test("operator route rejects unknown or missing command fields", async () => {
  const { ports, calls } = fixture();
  const response = await handleMigrationOperatorRequest(
    request("start", { binding_id: BINDING, migration_id: "m1", target_jurisdiction: "eu", routing_epoch: 9 }),
    { MIGRATION_OPERATOR_TOKEN: TOKEN },
    ports,
  );
  assert.equal(response.status, 400);
  assert.equal(((await response.json()) as { error: string }).error, "invalid_body");
  assert.equal(calls.length, 0);
});

test("operator command reports malformed binding ids as request errors", async () => {
  const { ports, calls } = fixture();
  const response = await handleMigrationOperatorRequest(
    request("start", { binding_id: "not-a-binding", migration_id: "m1", target_jurisdiction: "eu" }),
    { MIGRATION_OPERATOR_TOKEN: TOKEN },
    ports,
  );
  assert.equal(response.status, 400);
  assert.equal(((await response.json()) as { error: string }).error, "invalid_binding_id");
  assert.equal(calls.length, 0);
});

test("start dispatches through the coordinator and returns no subject key", async () => {
  const { ports, calls } = fixture("stable", "eu");
  const response = await handleMigrationOperatorRequest(
    request("start", { binding_id: BINDING, migration_id: "m1", target_jurisdiction: "eu" }),
    { MIGRATION_OPERATOR_TOKEN: TOKEN },
    ports,
  );
  assert.equal(response.status, 200);
  const body = (await response.json()) as { result: string; account: Record<string, unknown> };
  assert.equal(body.result, "complete");
  assert.equal(body.account.jurisdiction, "eu");
  assert.equal("subjectKey" in body.account, false);
  assert.equal(calls[0]?.path, "acquire");
  assert.match(calls[0]?.owner ?? "", /^operator:/);
  assert.equal(calls.at(-1)?.path, "release");
});

test("resume refuses a stable account at a different target", async () => {
  const { ports, calls } = fixture("stable", "us");
  const response = await handleMigrationOperatorRequest(
    request("resume", { binding_id: BINDING, migration_id: "m1", target_jurisdiction: "eu" }),
    { MIGRATION_OPERATOR_TOKEN: TOKEN },
    ports,
  );
  assert.equal(response.status, 409);
  assert.equal(((await response.json()) as { error: string }).error, "migration_not_in_progress");
  assert.equal(calls.length, 0);
});

test("abort after switch preserves forward-only recovery", async () => {
  const { ports } = fixture("switched", "eu");
  const response = await handleMigrationOperatorRequest(
    request("abort", { binding_id: BINDING, migration_id: "m1" }),
    { MIGRATION_OPERATOR_TOKEN: TOKEN },
    ports,
  );
  assert.equal(response.status, 409);
  assert.equal(((await response.json()) as { error: string }).error, "directory_transition_refused");
});
