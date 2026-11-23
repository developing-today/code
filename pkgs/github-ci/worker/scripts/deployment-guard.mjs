#!/usr/bin/env node
import { appendFileSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { parseArgs } from "node:util";

const WORKER_NAME = "github-ci";
const UPLOAD_WORKFLOW = ".github/workflows/github-ci-worker-upload.yml";
const PROMOTE_WORKFLOW = ".github/workflows/github-ci-worker-promote.yml";
const SECRET_BINDINGS = [
  "GITHUB_WEBHOOK_SECRET",
  "TURSO_PLATFORM_TOKEN",
  "TURSO_QUEUE_TOKEN",
  "EVENTS_BROKER_TOKEN",
  "EVENTS_BROKER_ACCESS_CLIENT_ID",
  "EVENTS_BROKER_ACCESS_CLIENT_SECRET",
];
const EXPECTED_TEXT_BINDINGS = {
  TURSO_PLATFORM_URL: "https://fleet-platform-dezren39.aws-us-east-1.turso.io",
  TURSO_QUEUE_URL: "https://fleet-queue-dezren39.aws-us-east-1.turso.io",
  EVENTS_BROKER_URL: "https://queue-broker.security.cab",
};

const VERSION_ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const COMMIT_SHA = /^[0-9a-f]{40}$/i;
const POSITIVE_INTEGER = /^[1-9][0-9]*$/;

function usage() {
  throw new Error(
    "usage: deployment-guard.mjs build <output-v1-dir> | select-branch-policy <branch-policies.json> | check-environment <environment.json> <branch-policy.json> <environment-name> <branch-policy-id> | select-baseline <deployments.json> | verify-baseline <deployments.json> <version.json> | verify-inheritance-source <deployments.json> <versions.json> | select-upload-result <create-response.json> | verify-version <version.json> <version-id> [commit-sha] | verify-deployable <deployable.json> <version-id> | write-upload-receipt <baseline-deployments.json> <baseline-version.json> <create-response.json> <candidate-version.json> <receipt.json> | verify-upload-receipt <receipt.json> <run-attempt.json> <run-id> <run-attempt> <version-id> <baseline-deployment-id> <baseline-version-id> | verify-baseline-current <deployments.json> <upload-receipt.json> | verify-created-deployment <created.json> <upload-receipt.json> | verify-promotion-readback <deployments.json> <upload-receipt.json> <created.json> | write-promotion-receipt <upload-receipt.json> <created.json> <deployments.json> <receipt.json>",
  );
}

function readJson(path) {
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch {
    throw new Error(`could not read valid JSON from ${path}`);
  }
}

function unwrap(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) return value;
  if (value.success === false) throw new Error("API response was unsuccessful");
  return Object.hasOwn(value, "result") ? value.result : value;
}

function rows(value, label) {
  const result = unwrap(value);
  if (Array.isArray(result)) return result;
  if (result && typeof result === "object") {
    if (Array.isArray(result.items)) return result.items;
    if (Array.isArray(result.versions)) return result.versions;
    if (Array.isArray(result.deployments)) return result.deployments;
  }
  throw new Error(`${label} response did not contain a list`);
}

function createdOn(record, label) {
  const value = record?.created_on ?? record?.metadata?.created_on;
  const parsed = typeof value === "string" ? Date.parse(value) : Number.NaN;
  if (!Number.isFinite(parsed)) throw new Error(`${label} is missing a valid created_on timestamp`);
  return { value, parsed };
}

function newest(items, label) {
  if (!items.length) throw new Error(`${label} response did not contain any records`);
  const sorted = items
    .map((item) => ({ item, ...createdOn(item, `${label} record`) }))
    .sort((left, right) => right.parsed - left.parsed);
  if (sorted.length > 1 && sorted[0].parsed === sorted[1].parsed && sorted[0].item.id !== sorted[1].item.id) {
    throw new Error(`${label} response has ambiguous newest records`);
  }
  return sorted[0].item;
}

function annotation(version, key) {
  return version?.annotations?.[key] ?? version?.metadata?.annotations?.[key] ?? "";
}

function commitFrom(version) {
  const tag = annotation(version, "workers/tag");
  const message = annotation(version, "workers/message");
  const match = /^github-ci candidate ([0-9a-f]{40})$/i.exec(message);
  if (!match || tag !== `git-${match[1].slice(0, 12)}`) {
    throw new Error("version provenance does not match a github-ci CI candidate commit");
  }
  return match[1].toLowerCase();
}

function bindingsOf(version) {
  const bindings = version?.bindings ?? version?.resources?.bindings;
  if (Array.isArray(bindings)) return bindings;
  const env = version?.env ?? version?.resources?.env;
  if (env && typeof env === "object" && !Array.isArray(env)) {
    return Object.entries(env).map(([name, value]) => ({ name, ...value }));
  }
  throw new Error("version response did not include its binding metadata");
}

function assertSecrets(version) {
  const bindings = bindingsOf(version);
  const missing = SECRET_BINDINGS.filter((name) => {
    const matching = bindings.filter((binding) => binding.name === name);
    return matching.length !== 1 || matching[0].type !== "secret_text";
  });
  if (missing.length) {
    throw new Error(`version is missing required secret binding names or types: ${missing.join(", ")}`);
  }
  return [...SECRET_BINDINGS];
}

function reportOutput(values, summary) {
  const outputPath = process.env.GITHUB_OUTPUT;
  if (!outputPath) throw new Error("GITHUB_OUTPUT is not set");
  appendFileSync(outputPath, Object.entries(values).map(([key, value]) => `${key}=${value}\n`).join(""));
  if (summary && process.env.GITHUB_STEP_SUMMARY) {
    appendFileSync(process.env.GITHUB_STEP_SUMMARY, `${summary}\n`);
  }
}

function checkBuild(root) {
  const rootConfig = readJson(join(root, "config.json"));
  const worker = readJson(join(root, "workers/default/worker.config.json"));
  if (rootConfig.producer?.name !== "wrangler" || rootConfig.producer?.version !== "4.166.0") {
    throw new Error("Build Output must be produced by the pinned Wrangler 4.166.0 delegate");
  }
  if (worker.name !== WORKER_NAME || worker.compatibilityDate !== "2025-09-15" || worker.workersDev !== true) {
    throw new Error("Build Output worker identity, compatibility date, or workers.dev ingress differs from production");
  }
  if (Object.hasOwn(worker, "previewUrls")) {
    throw new Error("Build Output must preserve the existing preview URL policy without setting it");
  }
  for (const [name, value] of Object.entries(EXPECTED_TEXT_BINDINGS)) {
    const binding = worker.env?.[name];
    if (binding?.type !== "text" || binding.value !== value) {
      throw new Error(`Build Output non-secret binding mismatch: ${name}`);
    }
  }
  for (const name of SECRET_BINDINGS) {
    const binding = worker.env?.[name];
    if (binding?.type !== "secret" || Object.hasOwn(binding, "value")) {
      throw new Error(`Build Output must declare ${name} as a value-free secret binding`);
    }
  }
  console.log("Build Output v1 has the expected Worker, ingress, production vars, and value-free secret declarations.");
}

function selectBranchPolicy(path) {
  const response = unwrap(readJson(path));
  const policies = response?.branch_policies;
  if (!Array.isArray(policies) || policies.length !== 1 || response.total_count !== 1) {
    throw new Error("environment must have exactly one deployment branch policy");
  }
  const policy = policies[0];
  if (policy?.name !== "main" || !Number.isSafeInteger(policy.id) || policy.id < 1) {
    throw new Error("environment must have one identified main deployment branch policy");
  }
  reportOutput({ branch_policy_id: policy.id }, `Selected the only deployment policy ID for main (${policy.id}).`);
}

function checkEnvironment(environmentPath, policyPath, expectedName, expectedPolicyId) {
  const environment = readJson(environmentPath);
  const policy = unwrap(readJson(policyPath));
  if (environment?.name !== expectedName) throw new Error(`required environment ${expectedName} is missing`);
  const rules = environment.protection_rules;
  const reviewerRules = Array.isArray(rules) ? rules.filter((rule) => rule.type === "required_reviewers") : [];
  const reviewerRule = reviewerRules[0];
  const hasReviewer = Array.isArray(reviewerRule?.reviewers) && reviewerRule.reviewers.some((entry) =>
    ["User", "Team"].includes(entry?.type) && Number.isSafeInteger(entry?.reviewer?.id) && entry.reviewer.id > 0,
  );
  if (reviewerRules.length !== 1 || !hasReviewer) throw new Error(`${expectedName} must have exactly one valid required-reviewer protection rule`);
  if (reviewerRule.prevent_self_review !== true) throw new Error(`${expectedName} must prevent self-review`);
  if (environment.can_admins_bypass !== false) throw new Error(`${expectedName} must disallow administrator bypass`);
  const branchPolicy = environment.deployment_branch_policy;
  if (branchPolicy?.protected_branches !== false || branchPolicy?.custom_branch_policies !== true) {
    throw new Error(`${expectedName} must restrict deployments with a custom branch policy`);
  }
  if (!POSITIVE_INTEGER.test(String(expectedPolicyId)) || String(policy.id) !== String(expectedPolicyId) || policy.name !== "main" || policy.type !== "branch") {
    throw new Error(`${expectedName} must have exactly the selected policy as type branch with name main`);
  }
  reportOutput(
    { environment_name: expectedName },
    `Confirmed ${expectedName} has required reviewers, self-review prevention, no administrator bypass, and exactly the selected branch:main policy.`,
  );
}

function deploymentRecord(path) {
  const deployment = newest(rows(readJson(path), "deployment"), "deployment");
  if (!VERSION_ID.test(deployment.id ?? "")) throw new Error("latest deployment has no Cloudflare Deployment UUID");
  const versions = deployment.versions;
  if (!Array.isArray(versions) || versions.length !== 1 || !VERSION_ID.test(versions[0]?.version_id ?? "") || Number(versions[0]?.percentage) !== 100) {
    throw new Error("latest deployment must serve exactly one version at 100% traffic");
  }
  const timestamp = createdOn(deployment, "latest deployment");
  return {
    deployment_id: deployment.id,
    version_id: versions[0].version_id,
    created_on: timestamp.value,
    created_at_ms: timestamp.parsed,
  };
}

function versionRecord(path) {
  const version = unwrap(readJson(path));
  if (!VERSION_ID.test(version?.id ?? "")) throw new Error("Worker Version response has no Cloudflare Version UUID");
  const timestamp = createdOn(version, "Worker Version");
  return { version, created_on: timestamp.value, created_at_ms: timestamp.parsed };
}

function verifyBaseline(deploymentsPath, versionPath) {
  const baseline = deploymentRecord(deploymentsPath);
  const version = versionRecord(versionPath);
  if (version.version.id !== baseline.version_id) throw new Error("baseline version read-back does not match the active deployment");
  assertSecrets(version.version);
  reportOutput(
    {
      baseline_deployment_id: baseline.deployment_id,
      baseline_version_id: baseline.version_id,
      baseline_created_on: baseline.created_on,
    },
    `Verified active baseline deployment ${baseline.deployment_id}, version ${baseline.version_id}, and all ${SECRET_BINDINGS.length} secret binding names.`,
  );
}

function verifyInheritanceSource(deploymentsPath, versionsPath) {
  const baseline = deploymentRecord(deploymentsPath);
  const versions = rows(readJson(versionsPath), "uploaded Worker Versions");
  if (versions.length !== 1) {
    throw new Error("inheritance-source check must contain only the first latest Worker Version from page 1");
  }
  const latestUploaded = versions[0];
  createdOn(latestUploaded, "latest uploaded Worker Version");
  if (!VERSION_ID.test(latestUploaded.id ?? "")) throw new Error("latest uploaded Worker Version has no Cloudflare Version UUID");
  if (latestUploaded.id !== baseline.version_id) {
    throw new Error("latest uploaded Worker Version is not the currently serving production baseline; strict secret inheritance would use another version");
  }
  console.log(`Latest uploaded Worker Version ${latestUploaded.id} matches serving baseline ${baseline.version_id}.`);
}

function selectUploadResult(path) {
  const response = unwrap(readJson(path));
  const versionId = response?.id;
  if (!VERSION_ID.test(versionId ?? "")) throw new Error("cf Workers Version create response is missing its exact Version UUID");
  reportOutput({ version_id: versionId }, `Captured exact Workers Version UUID ${versionId} from the cf create response.`);
}

function verifyVersion(path, requestedId, expectedSha) {
  if (!VERSION_ID.test(requestedId)) throw new Error("requested version ID is not a UUID");
  const { version } = versionRecord(path);
  if (version.id !== requestedId) throw new Error("Cloudflare returned a different Worker Version ID");
  const sha = commitFrom(version);
  if (expectedSha !== undefined && sha !== expectedSha.toLowerCase()) {
    throw new Error("version provenance does not match the requested source commit");
  }
  assertSecrets(version);
  reportOutput(
    { source_sha: sha },
    `Verified candidate provenance ${sha} and all ${SECRET_BINDINGS.length} retained secret binding names for version ${requestedId}.`,
  );
}

function verifyDeployable(path, requestedId) {
  const versions = rows(readJson(path), "deployable versions");
  if (!versions.some((version) => version.id === requestedId)) {
    throw new Error("requested Worker Version is not in Cloudflare's deployable set");
  }
  console.log("Requested version is deployable.");
}

function assertUploadRuntime() {
  const sourceSha = (process.env.GITHUB_SHA ?? "").toLowerCase();
  if (process.env.GITHUB_REPOSITORY !== "developing-today/code") throw new Error("upload receipt must come from the production repository");
  if (process.env.GITHUB_EVENT_NAME !== "workflow_dispatch" || process.env.GITHUB_REF !== "refs/heads/main") {
    throw new Error("upload receipt must come from a manual run on main");
  }
  if (!COMMIT_SHA.test(sourceSha)) throw new Error("upload receipt is missing the full source commit SHA");
  const runId = process.env.GITHUB_RUN_ID ?? "";
  const attempt = process.env.GITHUB_RUN_ATTEMPT ?? "";
  if (!POSITIVE_INTEGER.test(runId) || !POSITIVE_INTEGER.test(attempt)) throw new Error("upload receipt is missing the exact run ID or attempt");
  const workflowRef = process.env.GITHUB_WORKFLOW_REF ?? "";
  if (!workflowRef.endsWith(`${UPLOAD_WORKFLOW}@refs/heads/main`)) throw new Error("upload receipt came from an unexpected workflow ref");
  return { sourceSha, runId, attempt: Number(attempt), workflowRef };
}

function writeUploadReceipt(baselineDeploymentsPath, baselineVersionPath, createResponsePath, candidatePath, outputPath) {
  const runtime = assertUploadRuntime();
  const baseline = deploymentRecord(baselineDeploymentsPath);
  const baselineVersion = versionRecord(baselineVersionPath);
  if (baselineVersion.version.id !== baseline.version_id) throw new Error("baseline version read-back does not match the active deployment");
  assertSecrets(baselineVersion.version);

  const createResponse = unwrap(readJson(createResponsePath));
  if (!VERSION_ID.test(createResponse?.id ?? "")) throw new Error("cf Workers Version create response is missing its exact Version UUID");
  const candidateRecord = versionRecord(candidatePath);
  const candidate = candidateRecord.version;
  if (candidate.id !== createResponse.id) throw new Error("candidate read-back does not match the exact Version UUID returned by cf create");
  if (candidate.id === baseline.version_id) throw new Error("uploaded candidate is the same version as the baseline");
  if (candidateRecord.created_at_ms <= baseline.created_at_ms) throw new Error("uploaded candidate is not newer than its production baseline");
  const candidateSha = commitFrom(candidate);
  if (candidateSha !== runtime.sourceSha) throw new Error("uploaded candidate provenance does not match this workflow commit");
  const candidateBindings = assertSecrets(candidate);

  const artifactName = `github-ci-candidate-${runtime.runId}-${runtime.attempt}`;
  const receipt = {
    schema: "github-ci-worker-upload-receipt/v1",
    upload: {
      repository: process.env.GITHUB_REPOSITORY,
      workflow_path: UPLOAD_WORKFLOW,
      workflow_ref: runtime.workflowRef,
      event: process.env.GITHUB_EVENT_NAME,
      ref: process.env.GITHUB_REF,
      run_id: runtime.runId,
      run_attempt: runtime.attempt,
      source_sha: runtime.sourceSha,
      artifact_name: artifactName,
    },
    baseline: {
      deployment_id: baseline.deployment_id,
      version_id: baseline.version_id,
      created_on: baseline.created_on,
    },
    candidate: {
      version_id: candidate.id,
      created_on: candidateRecord.created_on,
      tag: annotation(candidate, "workers/tag"),
      message: annotation(candidate, "workers/message"),
      secret_bindings: candidateBindings.map((name) => ({ name, type: "secret_text" })),
    },
  };
  writeFileSync(outputPath, `${JSON.stringify(receipt, null, 2)}\n`, { flag: "wx" });
  reportOutput(
    { version_id: candidate.id, artifact_name: artifactName, baseline_deployment_id: baseline.deployment_id, baseline_version_id: baseline.version_id },
    `Upload receipt ${artifactName} records attempt ${runtime.runId}/${runtime.attempt}, baseline deployment ${baseline.deployment_id}, baseline version ${baseline.version_id}, candidate version ${candidate.id}, and source ${runtime.sourceSha}.`,
  );
}

function validateReceiptShape(receipt, expected) {
  if (receipt?.schema !== "github-ci-worker-upload-receipt/v1") throw new Error("upload receipt schema is missing or unsupported");
  const upload = receipt.upload;
  const baseline = receipt.baseline;
  const candidate = receipt.candidate;
  if (!upload || !baseline || !candidate) throw new Error("upload receipt is missing upload, baseline, or candidate records");
  if (upload.repository !== "developing-today/code" || upload.workflow_path !== UPLOAD_WORKFLOW || upload.event !== "workflow_dispatch" || upload.ref !== "refs/heads/main") {
    throw new Error("upload receipt does not identify the approved upload workflow on main");
  }
  if (String(upload.run_id) !== expected.runId || Number(upload.run_attempt) !== expected.attempt) {
    throw new Error("upload receipt run ID or attempt does not match the selected upload run");
  }
  if (upload.artifact_name !== `github-ci-candidate-${expected.runId}-${expected.attempt}`) {
    throw new Error("upload receipt artifact name does not match the selected run attempt");
  }
  if (!COMMIT_SHA.test(upload.source_sha ?? "")) throw new Error("upload receipt is missing a full source commit SHA");
  if (!VERSION_ID.test(candidate.version_id ?? "") || candidate.version_id !== expected.candidateVersionId) {
    throw new Error("upload receipt candidate Version UUID does not match the requested version");
  }
  if (!VERSION_ID.test(baseline.deployment_id ?? "") || baseline.deployment_id !== expected.baselineDeploymentId) {
    throw new Error("upload receipt baseline Deployment UUID does not match the requested baseline");
  }
  if (!VERSION_ID.test(baseline.version_id ?? "") || baseline.version_id !== expected.baselineVersionId) {
    throw new Error("upload receipt baseline Version UUID does not match the requested baseline");
  }
  if (baseline.version_id === candidate.version_id) throw new Error("upload receipt candidate and baseline versions are identical");
  const baselineTime = Date.parse(baseline.created_on ?? "");
  const candidateTime = Date.parse(candidate.created_on ?? "");
  if (!Number.isFinite(baselineTime) || !Number.isFinite(candidateTime) || candidateTime <= baselineTime) {
    throw new Error("upload receipt is missing an ordered baseline/candidate timestamp");
  }
  const expectedTag = `git-${upload.source_sha.slice(0, 12)}`;
  const expectedMessage = `github-ci candidate ${upload.source_sha}`;
  if (candidate.tag !== expectedTag || candidate.message !== expectedMessage) throw new Error("upload receipt candidate provenance does not match its source commit");
  const bindingNames = Array.isArray(candidate.secret_bindings) ? candidate.secret_bindings.map((binding) => binding.name) : [];
  if (!Array.isArray(candidate.secret_bindings) || candidate.secret_bindings.length !== SECRET_BINDINGS.length || SECRET_BINDINGS.some((name) => bindingNames.filter((candidateName) => candidateName === name).length !== 1) || candidate.secret_bindings.some((binding) => binding.type !== "secret_text")) {
    throw new Error("upload receipt is missing one or more required retained secret binding names");
  }
  return { upload, baseline, candidate };
}

function verifyUploadRun(receiptPath, attemptPath, runIdArg, attemptArg, candidateIdArg, baselineDeploymentIdArg, baselineVersionIdArg) {
  if (!POSITIVE_INTEGER.test(runIdArg) || !POSITIVE_INTEGER.test(attemptArg)) throw new Error("selected upload run ID and attempt must be positive integers");
  if (!VERSION_ID.test(candidateIdArg) || !VERSION_ID.test(baselineDeploymentIdArg) || !VERSION_ID.test(baselineVersionIdArg)) {
    throw new Error("selected candidate and baseline receipt IDs must be Cloudflare UUIDs");
  }
  const expected = {
    runId: runIdArg,
    attempt: Number(attemptArg),
    candidateVersionId: candidateIdArg,
    baselineDeploymentId: baselineDeploymentIdArg,
    baselineVersionId: baselineVersionIdArg,
  };
  const { upload, baseline, candidate } = validateReceiptShape(readJson(receiptPath), expected);
  const run = readJson(attemptPath);
  const workflowPath = (run.path ?? "").split("@")[0];
  const runId = String(run.run_id ?? run.id ?? "");
  if (runId !== expected.runId || Number(run.run_attempt) !== expected.attempt) throw new Error("GitHub returned a different workflow run attempt");
  if (workflowPath !== UPLOAD_WORKFLOW) throw new Error("selected run attempt is not the github-ci Worker upload workflow");
  if (run.event !== "workflow_dispatch" || run.head_branch !== "main" || run.status !== "completed" || run.conclusion !== "success") {
    throw new Error("selected upload run attempt is not a successful manual run on main");
  }
  if (run.head_sha?.toLowerCase() !== upload.source_sha) throw new Error("upload receipt source commit does not match the selected run attempt");
  if (run.repository?.full_name !== upload.repository) throw new Error("selected upload run belongs to a different repository");
  reportOutput(
    {
      source_sha: upload.source_sha,
      candidate_version_id: candidate.version_id,
      baseline_deployment_id: baseline.deployment_id,
      baseline_version_id: baseline.version_id,
    },
    `Validated upload receipt from run attempt ${expected.runId}/${expected.attempt} with baseline deployment ${baseline.deployment_id} and candidate version ${candidate.version_id}.`,
  );
}

function verifyBaselineCurrent(deploymentsPath, receiptPath) {
  const candidateReceipt = readJson(receiptPath);
  const receipt = validateReceiptShape(candidateReceipt, {
    runId: String(candidateReceipt.upload?.run_id ?? ""),
    attempt: Number(candidateReceipt.upload?.run_attempt ?? 0),
    candidateVersionId: candidateReceipt.candidate?.version_id ?? "",
    baselineDeploymentId: candidateReceipt.baseline?.deployment_id ?? "",
    baselineVersionId: candidateReceipt.baseline?.version_id ?? "",
  });
  const current = deploymentRecord(deploymentsPath);
  if (current.deployment_id !== receipt.baseline.deployment_id || current.version_id !== receipt.baseline.version_id || current.created_on !== receipt.baseline.created_on) {
    throw new Error("production baseline changed after the upload receipt was created");
  }
  console.log(`Current production baseline matches receipt ${current.deployment_id}/${current.version_id}.`);
}

function createdDeployment(path, receipt) {
  const response = unwrap(readJson(path));
  const deployment = response?.deployment ?? response;
  if (!VERSION_ID.test(deployment?.id ?? "")) throw new Error("deployment create response is missing its exact Deployment UUID");
  const versions = deployment.versions;
  if (!Array.isArray(versions) || versions.length !== 1 || versions[0]?.version_id !== receipt.candidate.version_id || Number(versions[0]?.percentage) !== 100) {
    throw new Error("deployment create response does not contain the receipted candidate at 100% traffic");
  }
  if (deployment.id === receipt.baseline.deployment_id) throw new Error("deployment create response reused the baseline Deployment UUID");
  const timestamp = createdOn(deployment, "created deployment");
  if (timestamp.parsed <= Date.parse(receipt.baseline.created_on)) throw new Error("created deployment is not newer than its receipted baseline");
  return { id: deployment.id, created_on: timestamp.value };
}

function promotionReadback(deploymentsPath, receiptPath, createdPath) {
  const candidateReceipt = readJson(receiptPath);
  const receipt = validateReceiptShape(candidateReceipt, {
    runId: String(candidateReceipt.upload?.run_id ?? ""),
    attempt: Number(candidateReceipt.upload?.run_attempt ?? 0),
    candidateVersionId: candidateReceipt.candidate?.version_id ?? "",
    baselineDeploymentId: candidateReceipt.baseline?.deployment_id ?? "",
    baselineVersionId: candidateReceipt.baseline?.version_id ?? "",
  });
  const created = createdDeployment(createdPath, receipt);
  const actual = deploymentRecord(deploymentsPath);
  if (actual.deployment_id !== created.id) throw new Error("latest deployment read-back does not match the exact deployment create receipt");
  if (actual.created_on !== created.created_on) throw new Error("latest deployment timestamp does not match the exact deployment create receipt");
  if (actual.version_id !== receipt.candidate.version_id || actual.created_at_ms <= Date.parse(receipt.baseline.created_on)) {
    throw new Error("latest deployment read-back does not promote the receipted candidate over its baseline");
  }
  return { receipt, created, actual };
}

function verifyCreatedDeployment(createdPath, receiptPath) {
  const candidateReceipt = readJson(receiptPath);
  const receipt = validateReceiptShape(candidateReceipt, {
    runId: String(candidateReceipt.upload?.run_id ?? ""),
    attempt: Number(candidateReceipt.upload?.run_attempt ?? 0),
    candidateVersionId: candidateReceipt.candidate?.version_id ?? "",
    baselineDeploymentId: candidateReceipt.baseline?.deployment_id ?? "",
    baselineVersionId: candidateReceipt.baseline?.version_id ?? "",
  });
  const created = createdDeployment(createdPath, receipt);
  reportOutput(
    { deployment_id: created.id },
    `Cloudflare created deployment ${created.id} for receipted candidate ${receipt.candidate.version_id}.`,
  );
}

function verifyPromotionReadback(deploymentsPath, receiptPath, createdPath) {
  const { receipt, created } = promotionReadback(deploymentsPath, receiptPath, createdPath);
  reportOutput(
    { deployment_id: created.id },
    `Read-back confirms new deployment ${created.id} promotes candidate ${receipt.candidate.version_id} at 100% over baseline ${receipt.baseline.deployment_id}.`,
  );
}

function writePromotionReceipt(receiptPath, createdPath, deploymentsPath, outputPath) {
  const { receipt, created } = promotionReadback(deploymentsPath, receiptPath, createdPath);
  const runId = process.env.GITHUB_RUN_ID ?? "";
  const attempt = process.env.GITHUB_RUN_ATTEMPT ?? "";
  const workflowRef = process.env.GITHUB_WORKFLOW_REF ?? "";
  if (process.env.GITHUB_REPOSITORY !== receipt.upload.repository || process.env.GITHUB_EVENT_NAME !== "workflow_dispatch" || process.env.GITHUB_REF !== "refs/heads/main") {
    throw new Error("promotion receipt must come from a manual run on main in the upload repository");
  }
  if (!POSITIVE_INTEGER.test(runId) || !POSITIVE_INTEGER.test(attempt) || !workflowRef.endsWith(`${PROMOTE_WORKFLOW}@refs/heads/main`)) {
    throw new Error("promotion receipt is missing its exact workflow run identity");
  }
  const promotionReceipt = {
    schema: "github-ci-worker-promotion-receipt/v1",
    upload: receipt.upload,
    baseline: receipt.baseline,
    candidate: receipt.candidate,
    promotion: {
      repository: process.env.GITHUB_REPOSITORY,
      workflow_path: PROMOTE_WORKFLOW,
      workflow_ref: workflowRef,
      event: process.env.GITHUB_EVENT_NAME,
      ref: process.env.GITHUB_REF,
      run_id: runId,
      run_attempt: Number(attempt),
      source_sha: receipt.upload.source_sha,
    },
    deployment: {
      id: created.id,
      created_on: created.created_on,
      version_id: receipt.candidate.version_id,
      percentage: 100,
    },
  };
  writeFileSync(outputPath, `${JSON.stringify(promotionReceipt, null, 2)}\n`, { flag: "wx" });
  const artifactName = `github-ci-promotion-${runId}-${attempt}`;
  reportOutput(
    { promotion_artifact_name: artifactName },
    `Promotion receipt ${artifactName} records upload attempt ${receipt.upload.run_id}/${receipt.upload.run_attempt}, baseline deployment ${receipt.baseline.deployment_id}, and deployment ${created.id} at 100%.`,
  );
}

function main() {
  const parsed = parseArgs({ args: process.argv.slice(2), options: {}, allowPositionals: true, strict: true });
  const [command, ...args] = parsed.positionals;
  if (command === "build" && args.length === 1) return checkBuild(args[0]);
  if (command === "select-branch-policy" && args.length === 1) return selectBranchPolicy(args[0]);
  if (command === "check-environment" && args.length === 4) return checkEnvironment(args[0], args[1], args[2], args[3]);
  if (command === "select-upload-result" && args.length === 1) return selectUploadResult(args[0]);
  if (command === "select-baseline" && args.length === 1) {
    const baseline = deploymentRecord(args[0]);
    reportOutput(
      { baseline_deployment_id: baseline.deployment_id, baseline_version_id: baseline.version_id, baseline_created_on: baseline.created_on },
      `Selected active baseline deployment ${baseline.deployment_id} and version ${baseline.version_id}.`,
    );
    return;
  }
  if (command === "verify-baseline" && args.length === 2) return verifyBaseline(args[0], args[1]);
  if (command === "verify-inheritance-source" && args.length === 2) return verifyInheritanceSource(args[0], args[1]);
  if (command === "verify-version" && (args.length === 2 || args.length === 3)) return verifyVersion(args[0], args[1], args[2]);
  if (command === "verify-deployable" && args.length === 2) return verifyDeployable(args[0], args[1]);
  if (command === "write-upload-receipt" && args.length === 5) return writeUploadReceipt(args[0], args[1], args[2], args[3], args[4]);
  if (command === "verify-upload-receipt" && args.length === 7) return verifyUploadRun(...args);
  if (command === "verify-baseline-current" && args.length === 2) return verifyBaselineCurrent(args[0], args[1]);
  if (command === "verify-created-deployment" && args.length === 2) return verifyCreatedDeployment(args[0], args[1]);
  if (command === "verify-promotion-readback" && args.length === 3) return verifyPromotionReadback(args[0], args[1], args[2]);
  if (command === "write-promotion-receipt" && args.length === 4) return writePromotionReceipt(args[0], args[1], args[2], args[3]);
  usage();
}

try {
  main();
} catch (error) {
  console.error(`deployment guard: ${error instanceof Error ? error.message : String(error)}`);
  process.exitCode = 2;
}
