import test from "node:test";
import assert from "node:assert/strict";

import {
  assertActiveVersion,
  assertMigrationContract,
  assertSameActiveTraffic,
  deploymentFromCreateResponse,
  versionIdFromResponse,
} from "./release-contract.mjs";

const versionA = "023e105f-2a42-4f8b-a1c1-73f6a2a30c0f";
const versionB = "123e105f-2a42-4f8b-a1c1-73f6a2a30c0f";

test("uses the exact version id from the cf upload response", () => {
  const raw = JSON.stringify({ success: true, result: { id: versionA } });
  assert.equal(versionIdFromResponse(raw), versionA);
  assert.throws(() => versionIdFromResponse(`${raw}\nnoise`), /exactly one JSON response/);
  assert.throws(() => versionIdFromResponse(JSON.stringify({ id: versionA })), /result.id/);
});

test("preserves the existing Handoff SQLite v1 migration contract", () => {
  const config = JSON.stringify({
    name: "muse-agent-handoff",
    durable_objects: { bindings: [{ name: "HANDOFFS", class_name: "Handoff" }] },
    migrations: [{ tag: "v1", new_sqlite_classes: ["Handoff"] }],
  });
  assert.equal(assertMigrationContract(config).migrations[0].tag, "v1");
  assert.throws(() => assertMigrationContract(config.replace('"v1"', '"v2"')), /v1 migration/);
});

test("requires the create response to place the exact version at 100%", () => {
  const raw = JSON.stringify({
    success: true,
    result: { id: "323e105f-2a42-4f8b-a1c1-73f6a2a30c0f", versions: [{ version_id: versionA, percentage: 100 }] },
  });
  assert.equal(deploymentFromCreateResponse(raw).versions[0].percentage, 100);
  assert.throws(() => {
    const mixed = JSON.stringify({
      result: { id: "323e105f-2a42-4f8b-a1c1-73f6a2a30c0f", versions: [{ version_id: versionA, percentage: 99 }] },
    });
    const deployment = deploymentFromCreateResponse(mixed);
    assert.equal(deployment.versions.find((item) => item.version_id === versionA).percentage, 100);
  }, /100/);
});

test("requires the active deployment list to serve exactly the expected version", () => {
  const listing = (versionId) => JSON.stringify({
    success: true,
    result: [{ id: "423e105f-2a42-4f8b-a1c1-73f6a2a30c0f", versions: [{ version_id: versionId, percentage: 100 }] }],
  });
  assert.equal(assertActiveVersion(listing(versionA), versionA).versions[0].version_id, versionA);
  assert.throws(() => assertActiveVersion(listing(versionB), versionA), /does not serve exactly/);
});

test("refuses promotion if serving deployment id or traffic changed after upload", () => {
  const listing = (deploymentId, versionId = versionA) => JSON.stringify({
    success: true,
    result: [{ id: deploymentId, versions: [{ version_id: versionId, percentage: 100 }] }],
  });
  assert.doesNotThrow(() => assertSameActiveTraffic(
    listing("523e105f-2a42-4f8b-a1c1-73f6a2a30c0f"),
    listing("523e105f-2a42-4f8b-a1c1-73f6a2a30c0f"),
  ));
  assert.throws(() => assertSameActiveTraffic(
    listing("523e105f-2a42-4f8b-a1c1-73f6a2a30c0f"),
    listing("623e105f-2a42-4f8b-a1c1-73f6a2a30c0f", versionB),
  ), /changed since/);
});
