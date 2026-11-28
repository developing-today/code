#!/usr/bin/env node
// Offline-safe provenance and traffic checks for the handoff Worker release.
// This tool never calls Cloudflare; it validates files produced by cf and CI.
import { createHash } from "node:crypto";
import { readFile, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const WORKER = "muse-agent-handoff";
const STAGING_WORKER = "muse-agent-handoff-staging";
const VERSION_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const SHA_RE = /^[0-9a-f]{40}$/i;

export function parseCfResponse(raw, label = "cf response") {
  let parsed;
  try {
    parsed = JSON.parse(raw);
  } catch (error) {
    throw new Error(`${label} is not exactly one JSON response: ${error.message}`);
  }
  if (parsed?.success === false) throw new Error(`${label} reports success=false`);
  return parsed;
}

export function versionIdFromResponse(raw, label = "version create response") {
  const parsed = parseCfResponse(raw, label);
  const id = parsed.result?.id;
  if (typeof id !== "string" || !VERSION_RE.test(id)) {
    throw new Error(`${label} does not contain a valid result.id UUID`);
  }
  return id;
}

export function deploymentList(raw, label = "deployment list") {
  const parsed = parseCfResponse(raw, label);
  const result = parsed.result ?? parsed;
  const items = Array.isArray(result) ? result
    : Array.isArray(result?.items) ? result.items
    : Array.isArray(result?.deployments) ? result.deployments
    : Array.isArray(parsed.deployments) ? parsed.deployments
    : null;
  if (!items) throw new Error(`${label} has no deployment list`);
  return items;
}

export function deploymentFromCreateResponse(raw, label = "deployment create response") {
  const parsed = parseCfResponse(raw, label);
  const deployment = parsed.result?.deployment ?? parsed.result ?? parsed.deployment ?? parsed;
  if (!deployment || typeof deployment.id !== "string" || !Array.isArray(deployment.versions)) {
    throw new Error(`${label} has no deployment result with versions`);
  }
  return deployment;
}

function trafficSnapshot(deployment, label) {
  if (!deployment || typeof deployment.id !== "string" || !Array.isArray(deployment.versions)) {
    throw new Error(`${label} is missing a deployment id or versions`);
  }
  const versions = deployment.versions.map((entry) => {
    if (!entry || typeof entry.version_id !== "string" || !VERSION_RE.test(entry.version_id) ||
        typeof entry.percentage !== "number" || !Number.isFinite(entry.percentage) ||
        entry.percentage < 0 || entry.percentage > 100) {
      throw new Error(`${label} contains an invalid version traffic entry`);
    }
    return { version_id: entry.version_id, percentage: entry.percentage };
  }).sort((a, b) => a.version_id.localeCompare(b.version_id));
  const total = versions.reduce((sum, item) => sum + item.percentage, 0);
  if (versions.length === 0 || Math.abs(total - 100) > 1e-9) {
    throw new Error(`${label} traffic percentages do not sum to 100`);
  }
  return { id: deployment.id, versions };
}

export function activeSnapshot(raw, label = "deployment list") {
  const [active] = deploymentList(raw, label);
  if (!active) throw new Error(`${label} is empty; refusing to infer serving traffic`);
  return trafficSnapshot(active, `${label} active deployment`);
}

export function assertWorkerConfig(config, expectedWorker) {
  if (config.name !== expectedWorker || config.compatibilityDate !== "2026-11-21" ||
      config.workersDev !== true || config.previewUrls !== false ||
      config.observability?.enabled !== false ||
      config.env?.HANDOFFS?.type !== "durable-object" ||
      config.env?.HANDOFFS?.worker !== expectedWorker ||
      config.env?.HANDOFFS?.exportName !== "Handoff" ||
      config.env?.ADMIN_TOKEN?.type !== "secret" ||
      config.env?.HANDOFF_ENCRYPTION_KEY?.type !== "secret" ||
      config.exports?.Handoff?.type !== "durable-object" ||
      config.exports?.Handoff?.storage !== "sqlite") {
    throw new Error(`built Worker config does not preserve expected settings and bindings for ${expectedWorker}`);
  }
  return {
    worker: config.name,
    compatibilityDate: config.compatibilityDate,
    workersDev: config.workersDev,
    previewUrls: config.previewUrls,
    observability: config.observability.enabled,
    durableObjectBinding: config.env.HANDOFFS,
    exports: config.exports,
    secretBindingNames: ["ADMIN_TOKEN", "HANDOFF_ENCRYPTION_KEY"],
  };
}

export function assertMigrationContract(configText) {
  let config;
  try {
    config = JSON.parse(configText);
  } catch (error) {
    throw new Error(`wrangler.jsonc is invalid JSON: ${error.message}`);
  }
  if (config.name !== WORKER || config.durable_objects?.bindings?.length !== 1 ||
      config.durable_objects.bindings[0]?.name !== "HANDOFFS" ||
      config.durable_objects.bindings[0]?.class_name !== "Handoff" ||
      JSON.stringify(config.migrations) !== JSON.stringify([{ tag: "v1", new_sqlite_classes: ["Handoff"] }])) {
    throw new Error("wrangler.jsonc must preserve the existing Handoff SQLite v1 migration and binding");
  }
  return { durableObjectBinding: config.durable_objects.bindings[0], migrations: config.migrations };
}

export function assertActiveVersion(raw, expectedVersionId, label = "deployment list") {
  if (!VERSION_RE.test(expectedVersionId)) throw new Error("expected version id is invalid");
  const snapshot = activeSnapshot(raw, label);
  if (snapshot.versions.length !== 1 || snapshot.versions[0].version_id !== expectedVersionId ||
      snapshot.versions[0].percentage !== 100) {
    throw new Error(`${label} does not serve exactly the expected version at 100%`);
  }
  return snapshot;
}

export function assertSameActiveTraffic(beforeRaw, afterRaw) {
  const before = activeSnapshot(beforeRaw, "production baseline");
  const after = activeSnapshot(afterRaw, "current production deployment");
  if (JSON.stringify(before) !== JSON.stringify(after)) {
    throw new Error("production active deployment changed since the staged upload; refusing promotion");
  }
  return before;
}

function requireSha(value, label) {
  if (typeof value !== "string" || !SHA_RE.test(value)) throw new Error(`${label} must be a full git SHA`);
  return value.toLowerCase();
}

function digest(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

async function readRaw(dir, name) {
  return readFile(join(dir, name));
}

async function readJson(dir, name) {
  const raw = await readRaw(dir, name);
  try {
    return JSON.parse(raw.toString("utf8"));
  } catch (error) {
    throw new Error(`${name} is invalid JSON: ${error.message}`);
  }
}

export async function verifyReleaseBundle(dir, expectedRunId, expectedSourceSha) {
  const root = resolve(dir);
  if (!/^\d+$/.test(String(expectedRunId))) throw new Error("upload run id must be numeric");
  const sourceSha = requireSha(expectedSourceSha, "expected source SHA");
  const manifest = await readJson(root, "release.json");
  if (manifest.schema !== 1 || manifest.worker !== WORKER || manifest.stagingWorker !== STAGING_WORKER ||
      String(manifest.uploadRunId) !== String(expectedRunId) ||
      requireSha(manifest.sourceSha, "manifest source SHA") !== sourceSha) {
    throw new Error("release manifest does not match the trusted upload run and source");
  }

  const [sourceArchive, productionResponse, baselineRaw, stageProofRaw, stageVersionRaw, stageDeploymentRaw, stageActiveRaw, wranglerConfigRaw] = await Promise.all([
    readRaw(root, "worker-source.tar.gz"),
    readRaw(root, "production-version-response.json"),
    readRaw(root, "production-baseline.json"),
    readRaw(root, "stage-proof.json"),
    readRaw(root, "stage-version-response.json"),
    readRaw(root, "stage-deployment-response.json"),
    readRaw(root, "stage-active-deployments.json"),
    readRaw(root, "wrangler.jsonc"),
  ]);
  const stageConfigRaw = await readRaw(root, "stage-worker-config.json");
  const productionConfigRaw = await readRaw(root, "production-worker-config.json");
  const checks = {
    sourceArchiveSha256: digest(sourceArchive),
    productionResponseSha256: digest(productionResponse),
    productionBaselineSha256: digest(baselineRaw),
    stageProofSha256: digest(stageProofRaw),
    stageVersionResponseSha256: digest(stageVersionRaw),
    stageDeploymentResponseSha256: digest(stageDeploymentRaw),
    stageActiveDeploymentsSha256: digest(stageActiveRaw),
    stageConfigSha256: digest(stageConfigRaw),
    productionConfigSha256: digest(productionConfigRaw),
    wranglerConfigSha256: digest(wranglerConfigRaw),
  };
  for (const [field, value] of Object.entries(checks)) {
    if (manifest[field] !== value) throw new Error(`${field} does not match the release manifest`);
  }

  const productionVersionId = versionIdFromResponse(productionResponse.toString("utf8"), "production version response");
  if (manifest.productionVersionId !== productionVersionId) {
    throw new Error("manifest production version id differs from the exact cf response");
  }
  const stageVersionId = versionIdFromResponse(stageVersionRaw.toString("utf8"), "stage version response");
  const stageProof = JSON.parse(stageProofRaw.toString("utf8"));
  if (stageProof.schema !== 1 || stageProof.worker !== STAGING_WORKER ||
      requireSha(stageProof.sourceSha, "stage proof source SHA") !== sourceSha ||
      stageProof.versionId !== stageVersionId ||
      stageProof.smoke?.rootStatus !== 200 ||
      stageProof.smoke?.unauthorizedCreateStatus !== 401 ||
      stageProof.smoke?.createStatus !== 201 ||
      stageProof.smoke?.claimStatus !== 200 ||
      stageProof.smoke?.replayStatus !== 410 ||
      stageProof.smoke?.roundTripMatches !== true) {
    throw new Error("stage proof does not verify the same source and expected smoke responses");
  }
  const stageConfig = assertWorkerConfig(JSON.parse(stageConfigRaw.toString("utf8")), STAGING_WORKER);
  assertWorkerConfig(JSON.parse(productionConfigRaw.toString("utf8")), WORKER);
  assertMigrationContract(wranglerConfigRaw.toString("utf8"));
  if (stageProof.config?.worker !== stageConfig.worker ||
      stageProof.config?.durableObjectBinding?.exportName !== "Handoff" ||
      stageProof.config?.exports?.Handoff?.storage !== "sqlite") {
    throw new Error("stage proof config summary does not match the validated staged build config");
  }
  const stageDeployment = deploymentFromCreateResponse(stageDeploymentRaw.toString("utf8"), "stage deployment response");
  const deployedStageVersion = stageDeployment.versions.find((item) => item.version_id === stageVersionId);
  if (!deployedStageVersion || deployedStageVersion.percentage !== 100) {
    throw new Error("stage deployment response did not place the uploaded stage version at 100%");
  }
  assertActiveVersion(stageActiveRaw.toString("utf8"), stageVersionId, "stage active deployment list");
  const baselineSnapshot = activeSnapshot(baselineRaw.toString("utf8"), "production baseline");
  if (JSON.stringify(baselineSnapshot) !== JSON.stringify(manifest.productionBaseline)) {
    throw new Error("production baseline snapshot does not match its exact captured response");
  }
  return { productionVersionId, sourceSha, productionBaseline: baselineSnapshot };
}

async function stageProof(args) {
  const [sourceShaArg, versionResponsePath, deploymentResponsePath, activeListPath,
    configPath, smokeProofPath, outputPath] = args;
  if (args.length !== 7) usage();
  const sourceSha = requireSha(sourceShaArg, "source SHA");
  const versionId = versionIdFromResponse(await readFile(versionResponsePath, "utf8"));
  const deployment = deploymentFromCreateResponse(await readFile(deploymentResponsePath, "utf8"));
  const active = assertActiveVersion(await readFile(activeListPath, "utf8"), versionId, "stage deployment list");
  const config = assertWorkerConfig(JSON.parse(await readFile(configPath, "utf8")), STAGING_WORKER);
  const smoke = JSON.parse(await readFile(smokeProofPath, "utf8"));
  const exactVersion = deployment.versions.find((item) => item.version_id === versionId);
  if (smoke.schema !== 1 || smoke.worker !== STAGING_WORKER || smoke.sourceSha !== sourceSha ||
      smoke.versionId !== versionId) {
    throw new Error("stage smoke proof does not match the uploaded staging version and source");
  }
  if (!exactVersion || exactVersion.percentage !== 100) throw new Error("stage create response is not 100% on uploaded version");
  if (smoke.rootStatus !== 200 || smoke.unauthorizedCreateStatus !== 401 ||
      smoke.createStatus !== 201 || smoke.claimStatus !== 200 ||
      smoke.replayStatus !== 410 || smoke.roundTripMatches !== true) {
    throw new Error("stage smoke checks did not verify create, claim, and replay behavior");
  }
  await writeFile(outputPath, `${JSON.stringify({
    schema: 1,
    worker: STAGING_WORKER,
    sourceSha,
    versionId,
    deploymentId: deployment.id,
    activeDeployment: active,
    config,
    smoke,
  }, null, 2)}\n`, { mode: 0o600 });
  process.stdout.write(`${versionId}\n`);
}

async function writeReleaseManifest(args) {
  const [dir, runId, sourceShaArg] = args;
  if (args.length !== 3 || !/^\d+$/.test(runId)) usage();
  const root = resolve(dir);
  const sourceSha = requireSha(sourceShaArg, "source SHA");
  const [sourceArchive, productionResponse, baselineRaw, stageProofRaw, stageVersionRaw, stageDeploymentRaw, stageActiveRaw, wranglerConfigRaw] = await Promise.all([
    readRaw(root, "worker-source.tar.gz"),
    readRaw(root, "production-version-response.json"),
    readRaw(root, "production-baseline.json"),
    readRaw(root, "stage-proof.json"),
    readRaw(root, "stage-version-response.json"),
    readRaw(root, "stage-deployment-response.json"),
    readRaw(root, "stage-active-deployments.json"),
    readRaw(root, "wrangler.jsonc"),
  ]);
  const stageConfigRaw = await readRaw(root, "stage-worker-config.json");
  const productionConfigRaw = await readRaw(root, "production-worker-config.json");
  const stageProof = JSON.parse(stageProofRaw.toString("utf8"));
  const stageVersionId = versionIdFromResponse(stageVersionRaw.toString("utf8"), "stage version response");
  const stageDeployment = deploymentFromCreateResponse(stageDeploymentRaw.toString("utf8"), "stage deployment response");
  const exactStageVersion = stageDeployment.versions.find((item) => item.version_id === stageVersionId);
  if (stageProof.schema !== 1 || stageProof.worker !== STAGING_WORKER || stageProof.sourceSha !== sourceSha ||
      stageProof.versionId !== stageVersionId ||
      stageProof.smoke?.rootStatus !== 200 || stageProof.smoke?.unauthorizedCreateStatus !== 401 ||
      stageProof.smoke?.createStatus !== 201 || stageProof.smoke?.claimStatus !== 200 ||
      stageProof.smoke?.replayStatus !== 410 || stageProof.smoke?.roundTripMatches !== true ||
      !exactStageVersion || exactStageVersion.percentage !== 100) {
    throw new Error("stage proof does not match the source commit");
  }
  const stageConfig = assertWorkerConfig(JSON.parse(stageConfigRaw.toString("utf8")), STAGING_WORKER);
  assertWorkerConfig(JSON.parse(productionConfigRaw.toString("utf8")), WORKER);
  assertMigrationContract(wranglerConfigRaw.toString("utf8"));
  assertActiveVersion(stageActiveRaw.toString("utf8"), stageVersionId, "stage active deployment list");
  stageProof.config = {
    ...stageConfig,
  };
  const productionVersionId = versionIdFromResponse(productionResponse.toString("utf8"), "production version response");
  const productionBaseline = activeSnapshot(baselineRaw.toString("utf8"), "production baseline");
  const manifest = {
    schema: 1,
    worker: WORKER,
    stagingWorker: STAGING_WORKER,
    uploadRunId: String(runId),
    sourceSha,
    productionVersionId,
    productionBaseline,
    sourceArchiveSha256: digest(sourceArchive),
    productionResponseSha256: digest(productionResponse),
    productionBaselineSha256: digest(baselineRaw),
    stageProofSha256: digest(stageProofRaw),
    stageVersionResponseSha256: digest(stageVersionRaw),
    stageDeploymentResponseSha256: digest(stageDeploymentRaw),
    stageActiveDeploymentsSha256: digest(stageActiveRaw),
    stageConfigSha256: digest(stageConfigRaw),
    productionConfigSha256: digest(productionConfigRaw),
    wranglerConfigSha256: digest(wranglerConfigRaw),
  };
  await writeFile(join(root, "release.json"), `${JSON.stringify(manifest, null, 2)}\n`, { mode: 0o600 });
  process.stdout.write(`${productionVersionId}\n`);
}

function usage() {
  console.error(
    "usage: release-contract.mjs <version-id|assert-created|assert-active|assert-same-active|snapshot|assert-config|stage-proof|write-manifest|verify-release> ...",
  );
  process.exit(2);
}

async function main() {
  const [command, ...args] = process.argv.slice(2);
  if (command === "version-id") {
    if (args.length !== 1) usage();
    process.stdout.write(`${versionIdFromResponse(await readFile(args[0], "utf8"))}\n`);
  } else if (command === "assert-created") {
    if (args.length !== 2) usage();
    const versionId = args[1];
    const deployment = deploymentFromCreateResponse(await readFile(args[0], "utf8"));
    const item = deployment.versions.find((entry) => entry.version_id === versionId);
    if (!item || item.percentage !== 100) throw new Error("create response does not deploy the supplied version at 100%");
  } else if (command === "assert-active") {
    if (args.length !== 2) usage();
    assertActiveVersion(await readFile(args[0], "utf8"), args[1]);
  } else if (command === "assert-same-active") {
    if (args.length !== 2) usage();
    assertSameActiveTraffic(await readFile(args[0], "utf8"), await readFile(args[1], "utf8"));
  } else if (command === "snapshot") {
    if (args.length !== 1) usage();
    process.stdout.write(`${JSON.stringify(activeSnapshot(await readFile(args[0], "utf8")))}\n`);
  } else if (command === "assert-config") {
    if (args.length !== 2) usage();
    const config = JSON.parse(await readFile(args[0], "utf8"));
    process.stdout.write(`${JSON.stringify(assertWorkerConfig(config, args[1]))}\n`);
  } else if (command === "stage-proof") {
    await stageProof(args);
  } else if (command === "write-manifest") {
    await writeReleaseManifest(args);
  } else if (command === "verify-release") {
    if (args.length !== 3) usage();
    const verified = await verifyReleaseBundle(args[0], args[1], args[2]);
    process.stdout.write(`${verified.productionVersionId}\n`);
  } else {
    usage();
  }
}

if (fileURLToPath(import.meta.url) === resolve(process.argv[1] ?? "")) {
  main().catch((error) => {
    console.error(`release-contract: ${error.message}`);
    process.exitCode = 1;
  });
}
