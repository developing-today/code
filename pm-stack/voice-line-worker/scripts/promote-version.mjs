// Promote one exact, previously uploaded version after checking provenance and
// the expected currently serving version. This script is only called by the
// approval-gated main-branch workflow.
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import {
  currentFullRolloutVersionId,
  deployableVersionIds,
  deploymentCreatedId,
  exactHandoff,
  exactWorkerId,
  parseCfJson,
  releaseRepository,
  releaseWorker,
  requireExactVersion,
  parseDeploymentItems,
  parseVersionItems,
  uploadRunMatchesProvenance,
  validatePromotionInputs,
  verifyLatestDeployment,
} from "../src/deploy-contract.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const workerDir = resolve(here, "..");
const workerName = releaseWorker;
if (process.env.GITHUB_REPOSITORY !== releaseRepository || process.env.GITHUB_REF !== "refs/heads/main"
  || process.env.GITHUB_EVENT_NAME !== "workflow_dispatch") throw new Error("promotion requires the exact manual main workflow");
const accountId = requiredEnv("CLOUDFLARE_ACCOUNT_ID");
if (!/^[a-f0-9]{32}$/.test(accountId)) throw new Error("CLOUDFLARE_ACCOUNT_ID is invalid");
const apiToken = requiredEnv("CLOUDFLARE_API_TOKEN");
const githubToken = requiredEnv("GH_TOKEN");
const versionId = requiredEnv("CANDIDATE_VERSION_ID");
const sourceSha = requiredEnv("CANDIDATE_SOURCE_SHA").toLowerCase();
const uploadRunId = requiredEnv("CANDIDATE_UPLOAD_RUN_ID");
const uploadRunAttempt = requiredEnv("CANDIDATE_UPLOAD_RUN_ATTEMPT");
const expectedCurrentVersionId = requiredEnv("EXPECTED_CURRENT_VERSION_ID");
validatePromotionInputs({ versionId, sourceSha, uploadRunId, uploadRunAttempt, expectedCurrentVersionId });

const handoff = JSON.parse(await readFile(requiredEnv("VOICE_LINE_UPLOAD_RECEIPT"), "utf8"));

const uploadRunResponse = await fetch(
  `https://api.github.com/repos/${releaseRepository}/actions/runs/${encodeURIComponent(uploadRunId)}`,
  {
    headers: {
      authorization: `Bearer ${githubToken}`,
      accept: "application/vnd.github+json",
      "x-github-api-version": "2022-11-28",
    },
    redirect: "error",
    signal: AbortSignal.timeout(15_000),
  },
);
if (!uploadRunResponse.ok) throw new Error(`upload-run provenance check failed with HTTP ${uploadRunResponse.status}`);
const uploadRun = await uploadRunResponse.json();
if (!uploadRunMatchesProvenance(uploadRun, sourceSha, uploadRunId, uploadRunAttempt)) {
  throw new Error("the referenced upload run is not a successful main-branch upload of this exact source SHA and attempt");
}

// Only a commit reachable from the checked-out main branch can be promoted.
execFileSync("git", ["cat-file", "-e", `${sourceSha}^{commit}`], { cwd: workerDir, stdio: "ignore" });
execFileSync("git", ["merge-base", "--is-ancestor", sourceSha, "origin/main"], { cwd: workerDir, stdio: "ignore" });

const workerId = exactWorkerId(parseCfJson(runCf(["-q", "workers", "get", "--worker", workerName])));
exactHandoff(handoff, {
  worker: workerName, workerIdHash: createHash("sha256").update(workerId).digest("hex"), accountHash: createHash("sha256").update(accountId).digest("hex"),
  versionId, sourceSha, uploadRunId, uploadRunAttempt,
});
const versions = parseVersionItems(runCf(["-q", "workers", "versions", "list", "--worker", workerId, "--per-page", "100"]));
requireExactVersion(versions, versionId, sourceSha, uploadRunId, uploadRunAttempt);

const deployableResponse = await fetch(
  `https://api.cloudflare.com/client/v4/accounts/${encodeURIComponent(accountId)}/workers/scripts/${encodeURIComponent(workerName)}/versions?deployable=true`,
  { headers: { authorization: `Bearer ${apiToken}` }, redirect: "error", signal: AbortSignal.timeout(15_000) },
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

const newDeployment = parseCfJson(runCf([
  "-q", "workers", "deployments", "create",
  "--worker", workerName,
  "--strategy", "percentage",
  "--versions", JSON.stringify([{ version_id: versionId, percentage: 100 }]),
]));
const deploymentId = deploymentCreatedId(newDeployment, versionId);

const after = parseDeploymentItems(runCf(["-q", "workers", "deployments", "list", "--worker", workerName]));
verifyLatestDeployment(after, deploymentId, versionId);
console.log(`Promoted version ${versionId} from ${sourceSha}; exact deployment ${deploymentId} serves 100%.`);

function assertExpectedCurrentDeployment() {
  const deployments = parseDeploymentItems(runCf(["-q", "workers", "deployments", "list", "--worker", workerName]));
  const current = currentFullRolloutVersionId(deployments);
  if (current !== expectedCurrentVersionId) {
    throw new Error(`currently serving version is ${current ?? "not a single-version 100% rollout"}, expected ${expectedCurrentVersionId}; refusing promotion`);
  }
}

function runCf(args) {
  const binary = resolve(workerDir, "node_modules/cf/bin/cf");
  const { GH_TOKEN: _githubToken, ...env } = process.env;
  try {
    return execFileSync(process.execPath, [binary, ...args], {
      cwd: workerDir, env, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"],
    });
  } catch {
    throw new Error("cf promotion or verification failed; inspect provider state before retrying");
  }
}

function requiredEnv(name) {
  const value = process.env[name];
  if (!value) throw new Error(`${name} is required`);
  return value;
}
