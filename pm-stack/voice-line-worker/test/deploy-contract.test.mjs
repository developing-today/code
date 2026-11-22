import assert from "node:assert/strict";
import test from "node:test";

import config from "../cloudflare.config.ts";
import {
  deployableVersionIds,
  deploymentIsFullRollout,
  currentFullRolloutVersionId,
  parseCfJson,
  parseDeploymentItems,
  parseVersionItems,
  versionForTag,
  versionMatchesProvenance,
  validatePromotionInputs,
} from "../src/deploy-contract.mjs";

test("cf config preserves the deployed Worker bindings and SQLite Durable Object", () => {
  assert.equal(config.worker.name, "voice-line");
  assert.equal(config.worker.compatibilityDate, "2025-09-15");
  assert.equal(config.worker.workersDev, true);
  assert.equal(config.worker.env.TURSO_URL.value, "https://voice-line-ledger-dezren39.aws-us-east-1.turso.io");
  assert.equal(config.worker.env.TURSO_TOKEN.type, "secret");
  assert.equal(config.worker.env.STATUS_INGEST_TOKEN.type, "secret");
  assert.deepEqual(config.worker.env.VOICE_SESSIONS, {
    type: "durable-object",
    worker: "voice-line",
    exportName: "VoiceSession",
  });
  assert.deepEqual(config.worker.env.AI, { type: "ai" });
  assert.deepEqual(config.worker.exports.VoiceSession, {
    type: "durable-object",
    storage: "sqlite",
  });
});

test("parses cf JSON after progress output", () => {
  assert.deepEqual(parseCfJson('Creating version…\n{"result":{"id":"v1"}}'), { result: { id: "v1" } });
});

test("rejects malformed cf output", () => {
  assert.throws(() => parseCfJson("not JSON"), /could not parse cf JSON output/);
});

test("finds the newest version matching the commit tag", () => {
  const versions = [
    { id: "old", annotations: { "workers/tag": "git-deadbeef1234" }, created_on: "2026-01-01T00:00:00Z" },
    { id: "new", metadata: { annotations: { "workers/tag": "git-deadbeef1234" }, created_on: "2026-02-01T00:00:00Z" } },
    { id: "other", annotations: { "workers/tag": "git-aabbccdd1122" } },
  ];
  assert.equal(versionForTag(versions, "git-deadbeef1234").id, "new");
  assert.equal(versionForTag(versions, "missing"), null);
});

test("requires exact version tag and message provenance for promotion", () => {
  const sha = "deadbeef1234" + "a".repeat(28);
  assert.equal(versionMatchesProvenance({
    id: "candidate",
    annotations: { "workers/tag": `git-${sha}-r12345-a2`, "workers/message": `git ${sha}` },
  }, sha, "12345", "2"), true);
  assert.equal(versionMatchesProvenance({
    id: "spoofed",
    annotations: { "workers/tag": `git-${sha}-r12345-a1`, "workers/message": `git ${sha}` },
  }, sha, "12345", "2"), false);
});

test("accepts API deployable versions and refuses absent versions", () => {
  assert.deepEqual(deployableVersionIds({ result: { items: [{ id: "v1" }] } }), ["v1"]);
  assert.deepEqual(deployableVersionIds({ result: [] }), []);
  assert.throws(() => deployableVersionIds({ success: false }), /rejected the deployable-version check/);
});

test("requires a sole 100 percent rollout of the requested version", () => {
  assert.equal(deploymentIsFullRollout([{ versions: [{ version_id: "v1", percentage: 100 }] }], "v1"), true);
  assert.equal(deploymentIsFullRollout([{ versions: [{ version_id: "v1", percentage: 99 }] }], "v1"), false);
  assert.equal(deploymentIsFullRollout([{ versions: [{ version_id: "v1", percentage: 50 }, { version_id: "v2", percentage: 50 }] }], "v1"), false);
  assert.equal(deploymentIsFullRollout([], "v1"), false);
});

test("identifies only an unambiguous current 100 percent deployment", () => {
  assert.equal(currentFullRolloutVersionId([{ versions: [{ version_id: "v1", percentage: 100 }] }]), "v1");
  assert.equal(currentFullRolloutVersionId([{ versions: [{ version_id: "v1", percentage: 99 }] }]), null);
  assert.equal(currentFullRolloutVersionId([{ versions: [{ version_id: "v1", percentage: 50 }, { version_id: "v2", percentage: 50 }] }]), null);
  assert.equal(currentFullRolloutVersionId([]), null);
});

test("promotion inputs require exact SHA and UUID values", () => {
  const values = {
    versionId: "123e4567-e89b-42d3-a456-426614174000",
    sourceSha: "a".repeat(40),
    uploadRunId: "12345",
    uploadRunAttempt: "2",
    expectedCurrentVersionId: "123e4567-e89b-42d3-a456-426614174001",
  };
  assert.doesNotThrow(() => validatePromotionInputs(values));
  assert.throws(() => validatePromotionInputs({ ...values, versionId: "latest" }), /Version UUID/);
  assert.throws(() => validatePromotionInputs({ ...values, sourceSha: "deadbeef" }), /full 40-character/);
  assert.throws(() => validatePromotionInputs({ ...values, uploadRunId: "-1" }), /positive GitHub Actions run ID/);
  assert.throws(() => validatePromotionInputs({ ...values, expectedCurrentVersionId: "latest" }), /expected_current_version_id/);
});

test("parses version and deployment list output shapes", () => {
  assert.deepEqual(parseVersionItems('{"result":{"items":[{"id":"v1"}]}}'), [{ id: "v1" }]);
  assert.deepEqual(parseDeploymentItems('{"result":[{"id":"d1"}]}'), [{ id: "d1" }]);
  assert.throws(() => parseVersionItems("{}"), /unexpected versions output/);
});
