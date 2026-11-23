const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
export const releaseRepository = "developing-today/code";
export const releaseWorker = "voice-line";
const uploadWorkflow = ".github/workflows/voice-line-worker-deploy.yml";
export const uploadEnvironment = "voice-line-worker-upload";
export const promotionEnvironment = "voice-line-worker-production";
const integerInput = /^[1-9]\d{0,19}$/;

/** Parse cf's JSON output even when it prefixes human-readable progress. */
export function parseCfJson(output) {
  const text = output.trim();
  try {
    return JSON.parse(text);
  } catch {
    for (const marker of ["{", "["]) {
      for (let index = text.indexOf(marker); index >= 0; index = text.indexOf(marker, index + 1)) {
        try {
          return JSON.parse(text.slice(index));
        } catch {
          // Continue until the first complete JSON value is found.
        }
      }
    }
    throw new Error(`could not parse cf JSON output`);
  }
}

function itemsFrom(parsed, label) {
  if (Array.isArray(parsed)) return parsed;
  if (parsed?.success === false) throw new Error(`Cloudflare rejected ${label} listing`);
  if (Array.isArray(parsed?.result)) return parsed.result;
  if (Array.isArray(parsed?.result?.items)) return parsed.result.items;
  if (Array.isArray(parsed?.items)) return parsed.items;
  if (Array.isArray(parsed?.result?.[label])) return parsed.result[label];
  if (Array.isArray(parsed?.[label])) return parsed[label];
  throw new Error(`unexpected ${label} output`);
}

export function parseVersionItems(output) {
  return itemsFrom(parseCfJson(output), "versions");
}

export function parseDeploymentItems(output) {
  return itemsFrom(parseCfJson(output), "deployments");
}

function versionTag(version) {
  return version.annotations?.["workers/tag"]
    ?? version.metadata?.annotations?.["workers/tag"]
    ?? version.metadata?.tag;
}

function createdAt(version) {
  const raw = version.metadata?.created_on ?? version.created_on ?? version.createdOn ?? 0;
  const time = typeof raw === "number" ? raw : Date.parse(raw);
  return Number.isFinite(time) ? time : 0;
}

export function versionForTag(versions, tag) {
  return versions.filter((version) => version.id && versionTag(version) === tag)
    .sort((left, right) => createdAt(right) - createdAt(left))[0] ?? null;
}

export function deployableVersionIds(payload) {
  if (payload?.success === false) throw new Error("Cloudflare rejected the deployable-version check");
  const result = payload?.result;
  const candidates = Array.isArray(result) ? result : (result?.items ?? []);
  if (!Array.isArray(candidates)) throw new Error("unexpected deployable-version response shape");
  return candidates.flatMap((version) => typeof version?.id === "string" ? [version.id] : []);
}

export function deploymentIsFullRollout(deployments, versionId) {
  const latest = deployments[0];
  if (!Array.isArray(latest?.versions) || latest.versions.length !== 1) return false;
  const [version] = latest.versions;
  return version?.version_id === versionId && version.percentage === 100;
}

export function currentFullRolloutVersionId(deployments) {
  const latest = deployments[0];
  if (!Array.isArray(latest?.versions) || latest.versions.length !== 1) return null;
  const [version] = latest.versions;
  return typeof version?.version_id === "string" && version.percentage === 100
    ? version.version_id
    : null;
}

export function versionMatchesProvenance(version, sha, runId, runAttempt) {
  const tag = versionTag(version);
  const message = version.annotations?.["workers/message"]
    ?? version.metadata?.annotations?.["workers/message"];
  return tag === `git-${sha}-r${runId}-a${runAttempt}` && message === `git ${sha}`;
}

export function uploadRunMatchesProvenance(run, sha, runId, runAttempt) {
  return run?.path === uploadWorkflow
    && run.repository?.full_name === releaseRepository
    && run.head_repository?.full_name === releaseRepository
    && run.event === "workflow_dispatch"
    && run.head_branch === "main"
    && run.head_sha?.toLowerCase() === sha.toLowerCase()
    && String(run.id) === runId
    && String(run.run_attempt) === runAttempt
    && run.status === "completed"
    && run.conclusion === "success";
}

export function validatePromotionInputs({ versionId, sourceSha, uploadRunId, uploadRunAttempt, expectedCurrentVersionId }) {
  if (!uuid.test(versionId)) throw new Error("version_id must be a Cloudflare Version UUID");
  if (!/^[a-f0-9]{40}$/i.test(sourceSha)) throw new Error("source_sha must be a full 40-character commit SHA");
  if (!integerInput.test(uploadRunId)) throw new Error("upload_run_id must be a positive GitHub Actions run ID");
  if (!integerInput.test(uploadRunAttempt)) throw new Error("upload_run_attempt must be a positive run attempt");
  if (!uuid.test(expectedCurrentVersionId)) throw new Error("expected_current_version_id must be a Cloudflare Version UUID");
}

export function validateEnvironment(environment, structure, policies) {
  if (!(environment === uploadEnvironment || environment === promotionEnvironment)
    || structure?.name !== environment || !Number.isSafeInteger(structure.id) || structure.id <= 0
    || structure.can_admins_bypass !== false || structure.deployment_branch_policy?.protected_branches !== false
    || structure.deployment_branch_policy.custom_branch_policies !== true) {
    throw new Error("Voice Line environment is missing or not protected");
  }
  const reviewer = structure.protection_rules?.filter((rule) => rule?.type === "required_reviewers");
  if (reviewer?.length !== 1 || reviewer[0].prevent_self_review !== true || !Array.isArray(reviewer[0].reviewers)
    || !reviewer[0].reviewers.some((record) => ["User", "Team"].includes(record.type) && Number.isSafeInteger(record.reviewer?.id)
      && record.reviewer.id > 0)) throw new Error("Voice Line environment lacks independent reviewer protection");
  const rules = policies?.branch_policies;
  if (policies?.total_count !== 1 || !Array.isArray(rules) || rules.length !== 1
    || rules[0]?.type !== "branch" || rules[0]?.name !== "main") {
    throw new Error("Voice Line environment must allow only main and no tags");
  }
}

export function exactWorkerId(result) {
  if (result?.success !== true || result.result?.name !== releaseWorker || !/^[a-f0-9]{32}$/.test(result.result.id)) {
    throw new Error("Cloudflare did not verify the exact existing Voice Line Worker");
  }
  return result.result.id;
}

// The cf version uploader's structured output file is distinct from stdout.
export function uploadedVersionId(receipts) {
  if (!Array.isArray(receipts) || receipts.length !== 1) throw new Error("expected exactly one cf version-upload receipt");
  const row = receipts[0];
  if (row?.type !== "version-upload" || row.version !== 1 || row.worker_name !== releaseWorker
    || !uuid.test(row.version_id)) throw new Error("cf version-upload receipt does not match the exact Worker");
  return row.version_id;
}

export function requireExactVersion(versions, versionId, sha, runId, runAttempt) {
  const rows = versions.filter((row) => row?.id === versionId);
  if (rows.length !== 1 || !versionMatchesProvenance(rows[0], sha, runId, runAttempt)) {
    throw new Error("exact version ID in receipt does not match Worker version provenance");
  }
}

export function deploymentCreatedId(receipt, versionId) {
  if (receipt?.success !== true || !uuid.test(receipt.result?.id)
    || !deploymentIsFullRollout([receipt.result], versionId)) {
    throw new Error("cf deployment receipt is not the exact 100% candidate");
  }
  return receipt.result.id;
}

export function verifyLatestDeployment(deployments, deploymentId, versionId) {
  if (deployments[0]?.id !== deploymentId || deployments.filter((row) => row?.id === deploymentId).length !== 1
    || !deploymentIsFullRollout(deployments, versionId)) {
    throw new Error("latest active deployment is not the returned deployment at 100% candidate traffic");
  }
}

export function exactHandoff(handoff, expected) {
  const keys = ["worker", "workerIdHash", "accountHash", "versionId", "sourceSha", "uploadRunId", "uploadRunAttempt"];
  if (!handoff || Object.keys(handoff).length !== keys.length
    || keys.some((key) => handoff[key] !== expected[key])) throw new Error("upload receipt does not match the requested provenance");
}
