import test from "node:test";
import assert from "node:assert/strict";
import {
  assertConfiguredVersion,
  assertDeployment,
  uploadedVersionId,
  KV_NAMESPACE_ID,
} from "../release-version.mjs";

const versionId = "12345678-1234-1234-1234-123456789abc";
const configuredBindings = [
  { name: "GHB_EVENTS", type: "kv_namespace", namespace_id: KV_NAMESPACE_ID },
  { name: "DRAIN_BEARER", type: "secret_text" },
  { name: "GITHUB_WEBHOOK_SECRET", type: "secret_text" },
];

test("reads the full version id from a cf upload response", () => {
  assert.equal(uploadedVersionId({ result: { id: versionId } }), versionId);
  assert.equal(uploadedVersionId({ id: versionId }), versionId);
});

test("promotion requires both secrets and the existing queue namespace", () => {
  assertConfiguredVersion({ result: { id: versionId, bindings: configuredBindings } }, versionId);
  assertConfiguredVersion({ id: versionId, resources: { bindings: configuredBindings } }, versionId);
  assert.throws(
    () => assertConfiguredVersion({ result: { id: versionId, bindings: configuredBindings.slice(1) } }, versionId),
    /GHB_EVENTS KV namespace/,
  );
  assert.throws(
    () => assertConfiguredVersion({ result: { id: versionId, bindings: configuredBindings.slice(0, 2) } }, versionId),
    /GITHUB_WEBHOOK_SECRET/,
  );
  assert.throws(
    () => assertConfiguredVersion({ result: { id: versionId, bindings: configuredBindings } }, "12345678"),
    /full Worker version UUID/,
  );
});

test("promotion verification requires the exact latest version at 100%", () => {
  assertDeployment({ result: [
    { created_on: "2026-11-20T10:00:00Z", versions: [{ version_id: "87654321-1234-1234-1234-123456789abc", percentage: 100 }] },
    { created_on: "2026-11-21T10:00:00Z", versions: [{ version_id: versionId, percentage: 100 }] },
  ] }, versionId);
  assertDeployment({ result: { items: [
    { created_on: "2026-11-21T10:00:00Z", versions: [{ version_id: versionId, percentage: 100 }] },
  ] } }, versionId);
  assert.throws(
    () => assertDeployment({ result: [{ created_on: "2026-11-21T10:00:00Z", versions: [{ version_id: "87654321-1234-1234-1234-123456789abc", percentage: 100 }] }] }, versionId),
    /latest Worker deployment is not exactly/,
  );
});
