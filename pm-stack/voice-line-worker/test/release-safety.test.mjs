import assert from "node:assert/strict";
import test from "node:test";
import {
  deploymentCreatedId, exactHandoff, exactWorkerId, parseDeploymentItems, requireExactVersion, uploadedVersionId,
  uploadRunMatchesProvenance, validateEnvironment, verifyLatestDeployment,
} from "../src/deploy-contract.mjs";

const worker = "voice-line";
const workerId = "a".repeat(32);
const repo = "developing-today/code";
const environment = "voice-line-worker-production";
const v1 = "123e4567-e89b-42d3-a456-426614174000";
const v2 = "123e4567-e89b-42d3-a456-426614174001";
const d1 = "123e4567-e89b-42d3-a456-426614174002";
const d2 = "123e4567-e89b-42d3-a456-426614174003";
const sha = "a".repeat(40), run = "12345", attempt = "2";
const version = (id) => ({ id, annotations: { "workers/tag": `git-${sha}-r${run}-a${attempt}`, "workers/message": `git ${sha}` } });
const deployed = (id, version_id = v1) => ({ id, versions: [{ version_id, percentage: 100 }] });

test("pinned cf receipt oracle identifies exactly the returned Worker version, not another matching tag", () => {
  const receipt = { type: "version-upload", version: 1, worker_name: worker, version_id: v1 };
  assert.equal(uploadedVersionId([receipt]), v1);
  assert.doesNotThrow(() => requireExactVersion([version(v2), version(v1)], v1, sha, run, attempt));
  for (const shape of [[], [receipt, receipt], [{ ...receipt, worker_name: "other" }], [{ ...receipt, version_id: null }]]) {
    assert.throws(() => uploadedVersionId(shape), /receipt/);
  }
  assert.throws(() => requireExactVersion([version(v2)], v1, sha, run, attempt), /exact version ID/);
  assert.throws(() => requireExactVersion([version(v1), version(v1)], v1, sha, run, attempt), /exact version ID/);
});

test("legacy deployment envelope oracle verifies the returned deployment, rejects stale history and split", () => {
  assert.equal(deploymentCreatedId({ success: true, result: deployed(d1) }, v1), d1);
  const entries = parseDeploymentItems(JSON.stringify({ success: true, result: { deployments: [deployed(d1), deployed(d2)] } }));
  assert.doesNotThrow(() => verifyLatestDeployment(entries, d1, v1));
  assert.throws(() => verifyLatestDeployment(entries, d2, v1), /latest active/);
  assert.throws(() => verifyLatestDeployment([deployed(d1), deployed(d1)], d1, v1), /latest active/);
  const split = { id: d1, versions: [{ version_id: v1, percentage: 50 }, { version_id: v2, percentage: 50 }] };
  assert.throws(() => deploymentCreatedId({ success: true, result: split }, v1), /100%/);
  assert.throws(() => deploymentCreatedId({ success: false, result: deployed(d1) }, v1), /100%/);
});

test("GitHub run and artifact must name the exact repo, file, account fingerprint, Worker, SHA, version and attempt", () => {
  const gh = {
    path: ".github/workflows/voice-line-worker-deploy.yml", repository: { full_name: repo }, head_repository: { full_name: repo },
    id: Number(run), run_attempt: Number(attempt), event: "workflow_dispatch", head_branch: "main",
    head_sha: sha, status: "completed", conclusion: "success",
  };
  assert.equal(uploadRunMatchesProvenance(gh, sha, run, attempt), true);
  assert.equal(uploadRunMatchesProvenance({ ...gh, head_repository: { full_name: "other/code" } }, sha, run, attempt), false);
  assert.equal(uploadRunMatchesProvenance({ ...gh, path: gh.path + "@main" }, sha, run, attempt), false);
  assert.equal(exactWorkerId({ success: true, result: { name: worker, id: workerId } }), workerId);
  assert.throws(() => exactWorkerId({ success: true, result: { name: "other-worker", id: workerId } }), /exact existing/);
  const handoff = { worker, workerIdHash: "c".repeat(64), accountHash: "b".repeat(64), versionId: v1, sourceSha: sha, uploadRunId: run, uploadRunAttempt: attempt };
  assert.doesNotThrow(() => exactHandoff(handoff, handoff));
  for (const key of Object.keys(handoff)) assert.throws(() => exactHandoff({ ...handoff, [key]: "wrong" }, handoff), /provenance/);
});

test("environment policy oracle fails closed before a job can attach missing or weaker protection", () => {
  const env = {
    name: environment, id: 17, can_admins_bypass: false,
    deployment_branch_policy: { protected_branches: false, custom_branch_policies: true },
    protection_rules: [{ type: "required_reviewers", prevent_self_review: true, reviewers: [{ type: "User", reviewer: { id: 1 } }] }],
  };
  const rules = { total_count: 1, branch_policies: [{ name: "main", type: "branch" }] };
  assert.doesNotThrow(() => validateEnvironment(environment, env, rules));
  assert.throws(() => validateEnvironment(environment, null, rules), /not protected/);
  assert.throws(() => validateEnvironment(environment, { ...env, can_admins_bypass: true }, rules), /not protected/);
  assert.throws(() => validateEnvironment(environment, { ...env, protection_rules: [] }, rules), /reviewer protection/);
  assert.throws(() => validateEnvironment(environment, env, { total_count: 2, branch_policies: rules.branch_policies }), /only main/);
  assert.throws(() => validateEnvironment(environment, env, { total_count: 1, branch_policies: [{ name: "main", type: "tag" }] }), /no tags/);
});
