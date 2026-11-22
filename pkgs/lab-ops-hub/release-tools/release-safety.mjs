import assert from "node:assert/strict";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const WORKER = "lab-ops-hub";
export const UPLOAD_WORKFLOW = ".github/workflows/lab-ops-hub-worker-release.yml";
export const RECEIPT_ARTIFACT = "lab-ops-hub-version-receipt";

const packageDir = dirname(dirname(fileURLToPath(import.meta.url)));
const uuidPattern = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const shaPattern = /^[0-9a-f]{40}$/i;

function readJson(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

function cfResult(response) {
  if (!response || typeof response !== "object" || Array.isArray(response)) {
    throw new Error("Cloudflare CLI output must be a JSON object");
  }
  if (response.success === false || (Array.isArray(response.errors) && response.errors.length > 0)) {
    throw new Error("Cloudflare CLI returned an unsuccessful API response");
  }
  return Object.hasOwn(response, "result") ? response.result : response;
}

export function readExpectedWorkerConfig(configText) {
  const config = JSON.parse(configText.replace(/^\s*\/\/.*$/gm, ""));
  const migrations = config.migrations;
  const containers = config.containers;
  assert.equal(config.name, WORKER, "unexpected Worker name in wrangler.jsonc");
  assert.ok(Array.isArray(migrations) && migrations.length > 0, "Worker migration history is required");
  assert.ok(Array.isArray(containers) && containers.length > 0, "Worker container metadata is required");
  assert.equal(migrations.at(-1).tag, "v2", "release gate is intentionally pinned to migration tag v2");
  return {
    worker: config.name,
    migrationTag: migrations.at(-1).tag,
    containers: containers.map(({ class_name, image }) => ({ class_name, image })),
  };
}

export function latestServingVersionId(deploymentsResponse) {
  const result = cfResult(deploymentsResponse);
  const deployments = Array.isArray(result) ? result : result?.deployments;
  if (!Array.isArray(deployments) || deployments.length === 0) {
    throw new Error("Cloudflare returned no current Worker deployment; refusing release");
  }
  const active = deployments[0];
  if (!Array.isArray(active?.versions) || active.versions.length !== 1) {
    throw new Error("current Worker deployment is not a single-version deployment; refusing release");
  }
  const [version] = active.versions;
  if (version?.percentage !== 100 || typeof version.version_id !== "string" || !uuidPattern.test(version.version_id)) {
    throw new Error("current Worker deployment is not exactly one version at 100%; refusing release");
  }
  return version.version_id;
}

export function assertCurrentBaseline(deploymentsResponse, expectedVersionId) {
  if (!uuidPattern.test(expectedVersionId ?? "")) {
    throw new Error("expected baseline version ID is not a UUID");
  }
  const servingVersionId = latestServingVersionId(deploymentsResponse);
  if (servingVersionId !== expectedVersionId) {
    throw new Error(`serving baseline changed: expected ${expectedVersionId}, found ${servingVersionId}`);
  }
  return servingVersionId;
}

export function assertWorkerVersion(response, {
  versionId,
  versionTag,
  expectedConfig,
} = {}) {
  const version = cfResult(response);
  if (!version || typeof version !== "object" || Array.isArray(version)) {
    throw new Error("Cloudflare returned no Worker version metadata");
  }
  if (!uuidPattern.test(versionId ?? "") || version.id !== versionId) {
    throw new Error("Worker Version response ID does not match the requested exact version ID");
  }

  const config = expectedConfig ?? readExpectedWorkerConfig(readFileSync(join(packageDir, "wrangler.jsonc"), "utf8"));
  if (version.migration_tag !== config.migrationTag) {
    throw new Error(
      `Worker migration metadata is ${JSON.stringify(version.migration_tag)}; expected active tag ${config.migrationTag}`,
    );
  }
  if (versionTag !== undefined && version.annotations?.["workers/tag"] !== versionTag) {
    throw new Error("Worker Version tag metadata does not match the trusted upload receipt");
  }

  if (!Array.isArray(version.containers) || version.containers.length !== config.containers.length) {
    throw new Error("Worker Version container metadata is missing or does not match the configured container count");
  }
  for (const expected of config.containers) {
    const match = version.containers.find((container) => container?.class_name === expected.class_name);
    if (!match) {
      throw new Error(`Worker Version metadata is missing container class ${expected.class_name}`);
    }
    if (typeof match.image !== "string") {
      throw new Error(
        "Cloudflare Workers Versions GET exposes the container class but not its image URI; " +
        "cannot verify the pinned image with the Worker-only token. Failing closed before release.",
      );
    }
    if (match.image !== expected.image) {
      throw new Error(`Worker Version container image ${match.image} does not match the pinned image ${expected.image}`);
    }
  }
  return version;
}

export function versionIdFromUpload(response) {
  const result = cfResult(response);
  if (!result || typeof result !== "object" || !uuidPattern.test(result.id ?? "")) {
    throw new Error("cf workers versions create did not return an exact Worker Version UUID");
  }
  return result.id;
}

export function makeUploadReceipt({ uploadResponse, versionResponse, baselineVersionId, context }) {
  const versionId = versionIdFromUpload(uploadResponse);
  const config = readExpectedWorkerConfig(readFileSync(join(packageDir, "wrangler.jsonc"), "utf8"));
  const sourceSha = context?.sourceSha;
  if (!shaPattern.test(sourceSha ?? "")) throw new Error("upload source SHA must be a full 40-character commit SHA");
  if (!uuidPattern.test(baselineVersionId ?? "")) throw new Error("baseline version ID must be a UUID");
  const expectedTag = `git-${sourceSha.slice(0, 12)}`;
  assertWorkerVersion(versionResponse, { versionId, versionTag: expectedTag, expectedConfig: config });

  const runId = String(context.runId ?? "");
  const runAttempt = Number(context.runAttempt);
  if (!/^[1-9][0-9]{0,19}$/.test(runId) || !Number.isSafeInteger(runAttempt) || runAttempt < 1) {
    throw new Error("upload run ID and attempt must identify a real GitHub Actions run");
  }
  if (
    context.repository !== "developing-today/code" ||
    context.ref !== "refs/heads/main" ||
    context.event !== "workflow_dispatch"
  ) {
    throw new Error("upload receipt may only be made by a trusted main-branch workflow_dispatch run");
  }
  return {
    schema_version: 1,
    worker: WORKER,
    version_id: versionId,
    version_tag: expectedTag,
    migration_tag: config.migrationTag,
    image: config.containers[0].image,
    baseline_version_id: baselineVersionId,
    source_sha: sourceSha,
    upload_run_id: runId,
    upload_run_attempt: runAttempt,
    repository: context.repository,
    ref: context.ref,
    event: context.event,
    workflow_path: UPLOAD_WORKFLOW,
  };
}

export function assertTrustedUploadReceipt(receipt, run) {
  const fields = [
    "schema_version", "worker", "version_id", "version_tag", "migration_tag", "image",
    "baseline_version_id", "source_sha", "upload_run_id", "upload_run_attempt",
    "repository", "ref", "event", "workflow_path",
  ];
  if (!receipt || typeof receipt !== "object" || fields.some((field) => !Object.hasOwn(receipt, field))) {
    throw new Error("upload receipt is incomplete or has an unsupported schema");
  }
  const config = readExpectedWorkerConfig(readFileSync(join(packageDir, "wrangler.jsonc"), "utf8"));
  const expectedTag = `git-${receipt.source_sha?.slice(0, 12)}`;
  if (
    receipt.schema_version !== 1 || receipt.worker !== WORKER ||
    !uuidPattern.test(receipt.version_id ?? "") || !uuidPattern.test(receipt.baseline_version_id ?? "") ||
    !shaPattern.test(receipt.source_sha ?? "") || receipt.version_tag !== expectedTag ||
    receipt.migration_tag !== config.migrationTag || receipt.image !== config.containers[0].image ||
    receipt.repository !== "developing-today/code" || receipt.ref !== "refs/heads/main" ||
    receipt.event !== "workflow_dispatch" || receipt.workflow_path !== UPLOAD_WORKFLOW
  ) {
    throw new Error("upload receipt does not match the configured Worker release target");
  }
  if (!run || typeof run !== "object") throw new Error("GitHub Actions upload run metadata is missing");
  const runRepository = run.repository?.full_name;
  if (
    String(run.id) !== receipt.upload_run_id || Number(run.run_attempt) !== receipt.upload_run_attempt ||
    run.status !== "completed" || run.conclusion !== "success" || run.event !== "workflow_dispatch" ||
    run.head_branch !== "main" || run.head_sha !== receipt.source_sha || run.path !== UPLOAD_WORKFLOW ||
    runRepository !== receipt.repository
  ) {
    throw new Error("receipt is not from the exact successful trusted main-branch upload run");
  }
  return receipt;
}

function requireArgs(args, count, usage) {
  if (args.length !== count) throw new Error(`usage: ${usage}`);
}

function writeDeployVersions(receiptPath, outputPath) {
  const receipt = readJson(receiptPath);
  if (!uuidPattern.test(receipt.version_id ?? "")) throw new Error("receipt version ID is not a UUID");
  writeFileSync(outputPath, `${JSON.stringify([{ version_id: receipt.version_id, percentage: 100 }], null, 2)}\n`);
}

function cli(args) {
  const [command, ...rest] = args;
  if (command === "baseline-id") {
    requireArgs(rest, 1, "release-safety.mjs baseline-id <deployments.json>");
    console.log(latestServingVersionId(readJson(rest[0])));
    return;
  }
  if (command === "verify-current") {
    requireArgs(rest, 2, "release-safety.mjs verify-current <deployments.json> <expected-version-id>");
    assertCurrentBaseline(readJson(rest[0]), rest[1]);
    console.log(`current serving version matches ${rest[1]} at 100%`);
    return;
  }
  if (command === "verify-version") {
    if (rest.length < 2 || rest.length > 3) {
      throw new Error("usage: release-safety.mjs verify-version <version.json> <version-id> [version-tag]");
    }
    assertWorkerVersion(readJson(rest[0]), { versionId: rest[1], versionTag: rest[2] });
    console.log(`Worker version ${rest[1]} migration and container metadata verified`);
    return;
  }
  if (command === "upload-id") {
    requireArgs(rest, 1, "release-safety.mjs upload-id <upload.json>");
    console.log(versionIdFromUpload(readJson(rest[0])));
    return;
  }
  if (command === "receipt") {
    requireArgs(rest, 4, "release-safety.mjs receipt <upload.json> <version.json> <baseline-version-id> <output.json>");
    const receipt = makeUploadReceipt({
      uploadResponse: readJson(rest[0]),
      versionResponse: readJson(rest[1]),
      baselineVersionId: rest[2],
      context: {
        sourceSha: process.env.GITHUB_SHA,
        runId: process.env.GITHUB_RUN_ID,
        runAttempt: process.env.GITHUB_RUN_ATTEMPT,
        repository: process.env.GITHUB_REPOSITORY,
        ref: process.env.GITHUB_REF,
        event: process.env.GITHUB_EVENT_NAME,
      },
    });
    writeFileSync(rest[3], `${JSON.stringify(receipt, null, 2)}\n`);
    console.log(`wrote receipt for ${receipt.worker} version ${receipt.version_id}`);
    return;
  }
  if (command === "trust-receipt") {
    requireArgs(rest, 2, "release-safety.mjs trust-receipt <receipt.json> <run.json>");
    const receipt = assertTrustedUploadReceipt(readJson(rest[0]), readJson(rest[1]));
    console.log(`source_sha=${receipt.source_sha}`);
    console.log(`baseline_version_id=${receipt.baseline_version_id}`);
    console.log(`version_id=${receipt.version_id}`);
    console.log(`version_tag=${receipt.version_tag}`);
    return;
  }
  if (command === "deployment-versions") {
    requireArgs(rest, 2, "release-safety.mjs deployment-versions <receipt.json> <output.json>");
    writeDeployVersions(rest[0], rest[1]);
    return;
  }
  if (command === "verify-deployed") {
    requireArgs(rest, 2, "release-safety.mjs verify-deployed <deployments.json> <version-id>");
    assertCurrentBaseline(readJson(rest[0]), rest[1]);
    console.log(`confirmed ${rest[1]} is the only version serving at 100%`);
    return;
  }
  throw new Error(
    "usage: release-safety.mjs <baseline-id|verify-current|verify-version|upload-id|receipt|trust-receipt|deployment-versions|verify-deployed> ...",
  );
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  try {
    cli(process.argv.slice(2));
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  }
}
