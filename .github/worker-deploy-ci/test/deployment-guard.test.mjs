import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import test from "node:test";

const SCRIPT = new URL("../../../pkgs/github-ci/worker/scripts/deployment-guard.mjs", import.meta.url);
const SHA = "5f4dcc3b5aa765d61d8327deb882cf9921f7e3e0";
const RUN_ID = "123456789";
const RUN_ATTEMPT = "2";
const VERSION_ID = "01234567-89ab-cdef-0123-456789abcdef";
const BASELINE_VERSION_ID = "11234567-89ab-cdef-0123-456789abcdef";
const BASELINE_DEPLOYMENT_ID = "21234567-89ab-cdef-0123-456789abcdef";
const DEPLOYMENT_ID = "31234567-89ab-cdef-0123-456789abcdef";
const SECRET_NAMES = [
  "GITHUB_WEBHOOK_SECRET",
  "TURSO_PLATFORM_TOKEN",
  "TURSO_QUEUE_TOKEN",
  "EVENTS_BROKER_TOKEN",
  "EVENTS_BROKER_ACCESS_CLIENT_ID",
  "EVENTS_BROKER_ACCESS_CLIENT_SECRET",
];

function version(id, createdOn, overrides = {}) {
  return {
    id,
    created_on: createdOn,
    annotations: {
      "workers/tag": `git-${SHA.slice(0, 12)}`,
      "workers/message": `github-ci candidate ${SHA}`,
    },
    bindings: SECRET_NAMES.map((name) => ({ name, type: "secret_text" })),
    ...overrides,
  };
}

function baselineDeployment(overrides = {}) {
  return {
    id: BASELINE_DEPLOYMENT_ID,
    created_on: "2026-11-22T12:00:00.000Z",
    versions: [{ version_id: BASELINE_VERSION_ID, percentage: 100 }],
    ...overrides,
  };
}

function candidateReceipt() {
  return {
    schema: "github-ci-worker-upload-receipt/v1",
    upload: {
      repository: "developing-today/code",
      workflow_path: ".github/workflows/github-ci-worker-upload.yml",
      workflow_ref: "developing-today/code/.github/workflows/github-ci-worker-upload.yml@refs/heads/main",
      event: "workflow_dispatch",
      ref: "refs/heads/main",
      run_id: RUN_ID,
      run_attempt: Number(RUN_ATTEMPT),
      source_sha: SHA,
      artifact_name: `github-ci-candidate-${RUN_ID}-${RUN_ATTEMPT}`,
    },
    baseline: {
      deployment_id: BASELINE_DEPLOYMENT_ID,
      version_id: BASELINE_VERSION_ID,
      created_on: "2026-11-22T12:00:00.000Z",
    },
    candidate: {
      version_id: VERSION_ID,
      created_on: "2026-11-22T12:05:00.000Z",
      tag: `git-${SHA.slice(0, 12)}`,
      message: `github-ci candidate ${SHA}`,
      secret_bindings: SECRET_NAMES.map((name) => ({ name, type: "secret_text" })),
    },
  };
}

function writeJson(path, value) {
  writeFileSync(path, JSON.stringify(value));
}

function run(root, args, env = {}) {
  return spawnSync(process.execPath, [SCRIPT.pathname, ...args], {
    cwd: root,
    encoding: "utf8",
    env: { ...process.env, ...env },
  });
}

function temp(t) {
  const root = mkdtempSync(join(tmpdir(), "github-ci-guard-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  return root;
}

function uploadEnv(root) {
  return {
    GITHUB_REPOSITORY: "developing-today/code",
    GITHUB_EVENT_NAME: "workflow_dispatch",
    GITHUB_REF: "refs/heads/main",
    GITHUB_SHA: SHA,
    GITHUB_RUN_ID: RUN_ID,
    GITHUB_RUN_ATTEMPT: RUN_ATTEMPT,
    GITHUB_WORKFLOW_REF: "developing-today/code/.github/workflows/github-ci-worker-upload.yml@refs/heads/main",
    GITHUB_OUTPUT: join(root, "output"),
    GITHUB_STEP_SUMMARY: join(root, "summary"),
  };
}

test("environment check fails closed without required reviewers and exact main policy", (t) => {
  const root = temp(t);
  const environmentPath = join(root, "environment.json");
  const policiesPath = join(root, "policies.json");
  writeJson(environmentPath, {
    name: "github-ci-production",
    protection_rules: [{ type: "required_reviewers", reviewers: [{ type: "User", reviewer: { login: "reviewer" } }] }],
    deployment_branch_policy: { protected_branches: false, custom_branch_policies: true },
  });
  writeJson(policiesPath, { branch_policies: [{ id: 1, name: "main" }] });
  assert.equal(run(root, ["check-environment", environmentPath, policiesPath, "github-ci-production"], {
    GITHUB_OUTPUT: join(root, "output"),
  }).status, 0);

  writeJson(environmentPath, { name: "github-ci-production", protection_rules: [], deployment_branch_policy: { custom_branch_policies: true } });
  assert.notEqual(run(root, ["check-environment", environmentPath, policiesPath, "github-ci-production"], {
    GITHUB_OUTPUT: join(root, "output"),
  }).status, 0);
  writeJson(environmentPath, {
    name: "github-ci-production",
    protection_rules: [{ type: "required_reviewers", reviewers: [{ type: "User", reviewer: { login: "reviewer" } }] }],
    deployment_branch_policy: { protected_branches: false, custom_branch_policies: true },
  });
  writeJson(policiesPath, { branch_policies: [{ id: 1, name: "main" }, { id: 2, name: "release/*" }] });
  assert.notEqual(run(root, ["check-environment", environmentPath, policiesPath, "github-ci-production"], {
    GITHUB_OUTPUT: join(root, "output"),
  }).status, 0);
});

test("baseline and uploaded candidate receipt preserve exact IDs, attempts, provenance, and binding names", (t) => {
  const root = temp(t);
  const baselinePath = join(root, "baseline-deployments.json");
  const baselineVersionPath = join(root, "baseline-version.json");
  const candidatePath = join(root, "candidate-version.json");
  const receiptPath = join(root, "upload-receipt.json");
  const runPath = join(root, "run-attempt.json");
  writeJson(baselinePath, { result: { deployments: [baselineDeployment()] } });
  writeJson(baselineVersionPath, { result: version(BASELINE_VERSION_ID, "2026-11-22T11:59:00.000Z", { annotations: {} }) });
  writeJson(candidatePath, { result: version(VERSION_ID, "2026-11-22T12:05:00.000Z") });

  const env = uploadEnv(root);
  const written = run(root, ["write-upload-receipt", baselinePath, baselineVersionPath, candidatePath, receiptPath], env);
  assert.equal(written.status, 0, written.stderr);
  const receipt = JSON.parse(readFileSync(receiptPath, "utf8"));
  assert.equal(receipt.upload.run_id, RUN_ID);
  assert.equal(receipt.upload.run_attempt, Number(RUN_ATTEMPT));
  assert.equal(receipt.baseline.deployment_id, BASELINE_DEPLOYMENT_ID);
  assert.equal(receipt.baseline.version_id, BASELINE_VERSION_ID);
  assert.deepEqual(receipt.candidate.secret_bindings.map((binding) => binding.name), SECRET_NAMES);
  assert.doesNotMatch(readFileSync(receiptPath, "utf8"), /secret_text.*value|"value"/);

  writeJson(runPath, {
    id: Number(RUN_ID),
    run_attempt: Number(RUN_ATTEMPT),
    path: ".github/workflows/github-ci-worker-upload.yml@main",
    event: "workflow_dispatch",
    head_branch: "main",
    head_sha: SHA,
    status: "completed",
    conclusion: "success",
    repository: { full_name: "developing-today/code" },
  });
  const verified = run(root, ["verify-upload-receipt", receiptPath, runPath, RUN_ID, RUN_ATTEMPT, VERSION_ID, BASELINE_DEPLOYMENT_ID, BASELINE_VERSION_ID], env);
  assert.equal(verified.status, 0, verified.stderr);
  assert.match(readFileSync(env.GITHUB_OUTPUT, "utf8"), new RegExp(`source_sha=${SHA}`));

  assert.notEqual(run(root, ["verify-upload-receipt", receiptPath, runPath, RUN_ID, "1", VERSION_ID, BASELINE_DEPLOYMENT_ID, BASELINE_VERSION_ID], env).status, 0);
  assert.notEqual(run(root, ["verify-upload-receipt", receiptPath, runPath, RUN_ID, RUN_ATTEMPT, VERSION_ID, DEPLOYMENT_ID, BASELINE_VERSION_ID], env).status, 0);
  assert.notEqual(run(root, ["verify-upload-receipt", join(root, "missing.json"), runPath, RUN_ID, RUN_ATTEMPT, VERSION_ID, BASELINE_DEPLOYMENT_ID, BASELINE_VERSION_ID], env).status, 0);
});

test("version guards reject missing bindings and verify the existing active baseline", (t) => {
  const root = temp(t);
  const baselinePath = join(root, "baseline-deployments.json");
  const versionPath = join(root, "version.json");
  const outputPath = join(root, "output");
  writeJson(baselinePath, { result: [baselineDeployment()] });
  writeJson(versionPath, { result: version(BASELINE_VERSION_ID, "2026-11-22T11:59:00.000Z", { annotations: {} }) });
  const good = run(root, ["verify-baseline", baselinePath, versionPath], { GITHUB_OUTPUT: outputPath });
  assert.equal(good.status, 0, good.stderr);
  assert.match(readFileSync(outputPath, "utf8"), new RegExp(`baseline_deployment_id=${BASELINE_DEPLOYMENT_ID}`));

  writeJson(versionPath, { result: version(BASELINE_VERSION_ID, "2026-11-22T11:59:00.000Z", { bindings: [] }) });
  assert.notEqual(run(root, ["verify-baseline", baselinePath, versionPath], { GITHUB_OUTPUT: outputPath }).status, 0);

  const receiptPath = join(root, "receipt.json");
  writeJson(receiptPath, candidateReceipt());
  const changed = { deployments: [baselineDeployment({ id: DEPLOYMENT_ID, created_on: "2026-11-23T12:00:00.000Z" })] };
  writeJson(baselinePath, changed);
  assert.notEqual(run(root, ["verify-baseline-current", baselinePath, receiptPath]).status, 0);
});

test("version guard accepts Cloudflare env-map reads without secret values", (t) => {
  const root = temp(t);
  const versionPath = join(root, "version.json");
  writeJson(versionPath, {
    result: {
      ...version(VERSION_ID, "2026-11-22T12:05:00.000Z"),
      bindings: undefined,
      env: Object.fromEntries(SECRET_NAMES.map((name) => [name, { type: "secret_text" }])),
    },
  });
  const result = run(root, ["verify-version", versionPath, VERSION_ID, SHA], {
    GITHUB_OUTPUT: join(root, "output"),
    GITHUB_STEP_SUMMARY: join(root, "summary"),
  });
  assert.equal(result.status, 0, result.stderr);
  assert.doesNotMatch(readFileSync(join(root, "summary"), "utf8"), /secret_text/);
});

test("promotion needs the exact create receipt and latest 100 percent deployment read-back", (t) => {
  const root = temp(t);
  const receiptPath = join(root, "upload-receipt.json");
  const createdPath = join(root, "created.json");
  const deploymentsPath = join(root, "deployments.json");
  const outputPath = join(root, "output");
  const summaryPath = join(root, "summary");
  writeJson(receiptPath, candidateReceipt());
  const created = {
    id: DEPLOYMENT_ID,
    created_on: "2026-11-23T12:00:00.000Z",
    versions: [{ version_id: VERSION_ID, percentage: 100 }],
  };
  writeJson(createdPath, { result: created });
  writeJson(deploymentsPath, { result: { deployments: [created, baselineDeployment()] } });

  const createdResult = run(root, ["verify-created-deployment", createdPath, receiptPath], {
    GITHUB_OUTPUT: outputPath,
    GITHUB_STEP_SUMMARY: summaryPath,
  });
  assert.equal(createdResult.status, 0, createdResult.stderr);
  assert.match(readFileSync(outputPath, "utf8"), new RegExp(`deployment_id=${DEPLOYMENT_ID}`));
  const readback = run(root, ["verify-promotion-readback", deploymentsPath, receiptPath, createdPath], {
    GITHUB_OUTPUT: outputPath,
    GITHUB_STEP_SUMMARY: summaryPath,
  });
  assert.equal(readback.status, 0, readback.stderr);

  const wrongId = { ...created, id: "41234567-89ab-cdef-0123-456789abcdef" };
  writeJson(createdPath, { result: wrongId });
  assert.notEqual(run(root, ["verify-promotion-readback", deploymentsPath, receiptPath, createdPath], { GITHUB_OUTPUT: outputPath }).status, 0);
  writeJson(createdPath, { result: created });
  writeJson(deploymentsPath, { result: { deployments: [{ ...created, versions: [{ version_id: BASELINE_VERSION_ID, percentage: 100 }] }, baselineDeployment()] } });
  assert.notEqual(run(root, ["verify-promotion-readback", deploymentsPath, receiptPath, createdPath], { GITHUB_OUTPUT: outputPath }).status, 0);

  writeJson(deploymentsPath, { result: { deployments: [created, baselineDeployment()] } });
  const promotionReceiptPath = join(root, "promotion-receipt.json");
  const promotionOutput = {
    GITHUB_REPOSITORY: "developing-today/code",
    GITHUB_EVENT_NAME: "workflow_dispatch",
    GITHUB_REF: "refs/heads/main",
    GITHUB_WORKFLOW_REF: "developing-today/code/.github/workflows/github-ci-worker-promote.yml@refs/heads/main",
    GITHUB_RUN_ID: "987654321",
    GITHUB_RUN_ATTEMPT: "1",
    GITHUB_OUTPUT: outputPath,
    GITHUB_STEP_SUMMARY: summaryPath,
  };
  const receiptResult = run(root, ["write-promotion-receipt", receiptPath, createdPath, deploymentsPath, promotionReceiptPath], promotionOutput);
  assert.equal(receiptResult.status, 0, receiptResult.stderr);
  const promotionReceipt = JSON.parse(readFileSync(promotionReceiptPath, "utf8"));
  assert.equal(promotionReceipt.upload.run_attempt, Number(RUN_ATTEMPT));
  assert.equal(promotionReceipt.baseline.deployment_id, BASELINE_DEPLOYMENT_ID);
  assert.equal(promotionReceipt.deployment.id, DEPLOYMENT_ID);
  assert.equal(promotionReceipt.deployment.percentage, 100);
});

test("CLI rejects unknown arguments before reading inputs", () => {
  const result = run(process.cwd(), ["verify-version", "--dry-rnu", "/does/not/exist", "bad"]);
  assert.equal(result.status, 2);
  assert.match(result.stderr, /Unknown option/);
});
