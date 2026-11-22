// Promote one exact, previously uploaded version after checking provenance and
// the expected currently serving version. This script is only called by the
// approval-gated main-branch workflow.
import { execFileSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import {
  currentFullRolloutVersionId,
  deployableVersionIds,
  deploymentIsFullRollout,
  parseDeploymentItems,
  parseVersionItems,
  uploadRunMatchesProvenance,
  validatePromotionInputs,
  versionMatchesProvenance,
} from "../src/deploy-contract.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const workerDir = resolve(here, "..");
const workerName = "voice-line";
const accountId = requiredEnv("CLOUDFLARE_ACCOUNT_ID");
const apiToken = requiredEnv("CLOUDFLARE_API_TOKEN");
const githubToken = requiredEnv("GH_TOKEN");
const versionId = requiredEnv("CANDIDATE_VERSION_ID");
const sourceSha = requiredEnv("CANDIDATE_SOURCE_SHA").toLowerCase();
const uploadRunId = requiredEnv("CANDIDATE_UPLOAD_RUN_ID");
const uploadRunAttempt = requiredEnv("CANDIDATE_UPLOAD_RUN_ATTEMPT");
const expectedCurrentVersionId = requiredEnv("EXPECTED_CURRENT_VERSION_ID");
validatePromotionInputs({ versionId, sourceSha, uploadRunId, uploadRunAttempt, expectedCurrentVersionId });

const repository = requiredEnv("GITHUB_REPOSITORY");
const uploadRunResponse = await fetch(
  `https://api.github.com/repos/${repository}/actions/runs/${encodeURIComponent(uploadRunId)}`,
  {
    headers: {
      authorization: `Bearer ${githubToken}`,
      accept: "application/vnd.github+json",
      "x-github-api-version": "2022-11-28",
    },
    signal: AbortSignal.timeout(15_000),
  },
);
if (!uploadRunResponse.ok) throw new Error(`upload-run provenance check failed with HTTP ${uploadRunResponse.status}`);
const uploadRun = await uploadRunResponse.json();
if (!uploadRunMatchesProvenance(uploadRun, sourceSha, uploadRunAttempt)) {
  throw new Error("the referenced upload run is not a successful main-branch upload of this exact source SHA and attempt");
}

// Only a commit reachable from the checked-out main branch can be promoted.
execFileSync("git", ["cat-file", "-e", `${sourceSha}^{commit}`], { cwd: workerDir, stdio: "ignore" });
execFileSync("git", ["merge-base", "--is-ancestor", sourceSha, "origin/main"], { cwd: workerDir, stdio: "ignore" });

const versions = parseVersionItems(runCf(["-q", "workers", "versions", "list", "--worker", workerName, "--per-page", "100"]));
const candidate = versions.find((version) => version.id === versionId);
if (!candidate || !versionMatchesProvenance(candidate, sourceSha, uploadRunId, uploadRunAttempt)) {
  throw new Error(`version ${versionId} is missing or does not match exact source SHA and upload-run provenance`);
}

const deployableResponse = await fetch(
  `https://api.cloudflare.com/client/v4/accounts/${encodeURIComponent(accountId)}/workers/scripts/${encodeURIComponent(workerName)}/versions?deployable=true`,
  { headers: { authorization: `Bearer ${apiToken}` }, signal: AbortSignal.timeout(15_000) },
);
if (!deployableResponse.ok) throw new Error(`deployable-version check failed with HTTP ${deployableResponse.status}`);
const deployablePayload = await deployableResponse.json();
if (!deployableVersionIds(deployablePayload).includes(versionId)) {
  throw new Error(`version ${versionId} is not in the deployable set`);
}

assertExpectedCurrentDeployment();
// Repeat the read immediately before the mutating API command to fail closed
// if another deployment changed the serving version during the checks above.
assertExpectedCurrentDeployment();

runCf([
  "-q", "workers", "deployments", "create",
  "--worker", workerName,
  "--strategy", "percentage",
  "--versions", JSON.stringify([{ version_id: versionId, percentage: 100 }]),
]);

const after = parseDeploymentItems(runCf(["-q", "workers", "deployments", "list", "--worker", workerName]));
if (!deploymentIsFullRollout(after, versionId)) {
  throw new Error(`deployment read-back did not show version ${versionId} serving 100% of traffic`);
}
console.log(`Promoted version ${versionId} from ${sourceSha}; read-back confirms 100% traffic.`);

function assertExpectedCurrentDeployment() {
  const deployments = parseDeploymentItems(runCf(["-q", "workers", "deployments", "list", "--worker", workerName]));
  const current = currentFullRolloutVersionId(deployments);
  if (current !== expectedCurrentVersionId) {
    throw new Error(`currently serving version is ${current ?? "not a single-version 100% rollout"}, expected ${expectedCurrentVersionId}; refusing promotion`);
  }
}

function runCf(args) {
  const binary = resolve(workerDir, "node_modules/cf/bin/cf");
  return execFileSync(process.execPath, [binary, ...args], {
    cwd: workerDir,
    env: process.env,
    encoding: "utf8",
    stdio: ["inherit", "pipe", "inherit"],
  });
}

function requiredEnv(name) {
  const value = process.env[name];
  if (!value) throw new Error(`${name} is required`);
  return value;
}
