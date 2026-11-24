// Explicitly promotes one provenance-checked mail Worker Version. This script
// never changes routes, triggers, bindings, or script settings.
// Usage: promote.mjs --manifest <upload-provenance.json> --promote
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const cfBin = join(here, "node_modules", ".bin", "cf");
async function main() {
  const args = process.argv.slice(2);
  if (args.length !== 3 || args[0] !== "--manifest" || !args[1] || args[2] !== "--promote") {
    console.error("usage: promote.mjs --manifest <upload-provenance.json> --promote");
    process.exit(2);
  }

  const manifest = JSON.parse(readFileSync(args[1], "utf8"));
  const verifiedRun = JSON.parse(readFileSync(requiredEnv("ID_MAIL_VERIFIED_UPLOAD_RUN_FILE"), "utf8"));
  const config = JSON.parse(readFileSync(join(here, "config.json"), "utf8"));
  const token = requiredEnv("ID_MAIL_WORKER_CF_TOKEN_MAIL_WORKER_PRODUCTION");
  const expectedRunId = requiredEnv("ID_MAIL_UPLOAD_RUN_ID");
  const expectedAttempt = requiredEnv("ID_MAIL_UPLOAD_RUN_ATTEMPT");
  const repository = requiredEnv("GITHUB_REPOSITORY");
  validateManifest(manifest, {
    expectedRunId,
    expectedAttempt,
    repository,
    worker: config.worker,
    expectedSha: verifiedRun.sha,
  });
  if (verifiedRun.id !== Number(expectedRunId) || verifiedRun.attempt !== Number(expectedAttempt) ||
      verifiedRun.repository?.toLowerCase() !== repository.toLowerCase() ||
      verifiedRun.workflow !== ".github/workflows/id-mail-worker-publish.yml" ||
      verifiedRun.event !== "workflow_dispatch" || verifiedRun.ref !== "refs/heads/main" ||
      verifiedRun.conclusion !== "success") {
    throw new Error("selected upload attempt record does not match the provenance artifact");
  }

  const baseline = activeDeployment(cfJson([
    "workers", "deployments", "list", "--worker", config.worker, "--per-page", "1",
  ], token));
  if (baseline.id !== manifest.baselineDeploymentId || baseline.versions?.[0]?.version_id !== manifest.baselineVersionId) {
    throw new Error("active deployment changed since upload; no promotion was made");
  }

  const [baselineVersion, candidateVersion] = await Promise.all([
    cfJson(["workers", "versions", "get", manifest.baselineVersionId, "--worker-id", config.worker], token),
    cfJson(["workers", "versions", "get", manifest.versionId, "--worker-id", config.worker], token),
  ]);
  if (candidateVersion.id !== manifest.versionId) throw new Error("Workers API returned a different candidate version");
  if (candidateVersion.annotations?.["workers/tag"] !== manifest.tag) {
    throw new Error("uploaded version tag does not match the upload provenance");
  }
  assertVersionConfigurationUnchanged(baselineVersion, candidateVersion);
  await assertRouteHealth(config.routes);

  const versions = JSON.stringify([{ version_id: manifest.versionId, percentage: 100 }]);
  cfJson(["workers", "deployments", "create", "--worker", config.worker, "--strategy", "percentage", "--versions", versions], token);

  const promoted = activeDeployment(cfJson([
    "workers", "deployments", "list", "--worker", config.worker, "--per-page", "1",
  ], token));
  if (promoted.id === manifest.baselineDeploymentId || promoted.versions?.length !== 1 ||
      promoted.versions[0]?.version_id !== manifest.versionId || promoted.versions[0]?.percentage !== 100) {
    throw new Error("Workers API did not report the requested version as the sole 100% deployment");
  }
  const [baselineAfter, candidateAfter] = await Promise.all([
    cfJson(["workers", "versions", "get", manifest.baselineVersionId, "--worker-id", config.worker], token),
    cfJson(["workers", "versions", "get", manifest.versionId, "--worker-id", config.worker], token),
  ]);
  assertVersionConfigurationUnchanged(baselineAfter, candidateAfter);
  await assertRouteHealth(config.routes);
  console.log(`promoted mail Worker version ${manifest.versionId}; existing routes returned expected unauthenticated health responses`);
}

function requiredEnv(name) {
  const value = process.env[name];
  if (!value) throw new Error(`${name} is not set`);
  return value;
}

function cfJson(argv, token) {
  const stdout = execFileSync(cfBin, argv, {
    cwd: here,
    encoding: "utf8",
    env: { ...process.env, CLOUDFLARE_API_TOKEN: token },
  });
  try {
    return unwrap(JSON.parse(stdout));
  } catch (error) {
    if (error instanceof SyntaxError) throw new Error("cf returned non-JSON output; stopping safely");
    throw error;
  }
}

function unwrap(parsed) {
  return parsed?.result ?? parsed;
}

function activeDeployment(result) {
  const rows = Array.isArray(result) ? result : result?.deployments;
  if (!Array.isArray(rows) || rows.length === 0) throw new Error("no active Worker deployment was returned");
  return rows[0];
}

export function validateManifest(manifest, expected) {
  if (manifest?.schema !== 1 || manifest.worker !== expected.worker ||
      manifest.repository !== expected.repository || manifest.workflow !== ".github/workflows/id-mail-worker-publish.yml" ||
      manifest.event !== "workflow_dispatch" || manifest.ref !== "refs/heads/main" ||
      manifest.runId !== expected.expectedRunId || manifest.runAttempt !== expected.expectedAttempt ||
      !/^[0-9a-f]{40}$/.test(manifest.sha ?? "") || manifest.sha !== expected.expectedSha ||
      !manifest.versionId || !manifest.baselineDeploymentId ||
      !manifest.baselineVersionId || !/^mail-[0-9a-f]{12}-run-\d+-attempt-\d+$/.test(manifest.tag ?? "")) {
    throw new Error("upload provenance does not match the explicitly selected successful main upload run");
  }
}

export function assertVersionConfigurationUnchanged(before, after) {
  const beforeBindings = normalizedBindings(before);
  const afterBindings = normalizedBindings(after);
  const callers = afterBindings.find((binding) => binding.name === "CALLERS");
  if (callers?.type !== "secret_text") throw new Error("candidate does not inherit the existing CALLERS secret binding");
  for (const [name, type] of [["MAIL_CONFIG", "plain_text"], ["EMAIL", "send_email"], ["LIMITER", "durable_object"]]) {
    if (!afterBindings.some((binding) => binding.name === name && binding.type === type)) {
      throw new Error(`candidate is missing the existing ${name} binding`);
    }
  }
  const beforeFingerprint = fingerprint({ bindings: beforeBindings, settings: versionSettings(before) });
  const afterFingerprint = fingerprint({ bindings: afterBindings, settings: versionSettings(after) });
  if (beforeFingerprint !== afterFingerprint) {
    throw new Error("candidate changes existing bindings or versioned script settings; no promotion was made");
  }
}

function normalizedBindings(version) {
  const bindings = version?.bindings ?? version?.metadata?.bindings;
  if (!Array.isArray(bindings)) throw new Error("Workers API response did not include version bindings");
  return bindings.map((binding) => {
    const clean = { ...binding };
    if (binding.type === "secret_text") {
      // Cloudflare returns secret binding descriptors without a value. Never
      // inspect or print any secret-like field from an API response.
      for (const key of ["text", "value", "secret", "secret_text"]) delete clean[key];
    }
    return clean;
  }).sort((a, b) => a.name.localeCompare(b.name));
}

function versionSettings(version) {
  const metadata = version?.metadata ?? {};
  const fields = [
    "compatibility_date", "compatibility_flags", "usage_model", "limits", "placement",
    "logpush", "observability", "tail_consumers", "streaming_tail_consumers", "cache_options", "assets",
    "migrations", "migration_tag", "exports",
  ];
  const settings = {};
  for (const field of fields) {
    const value = Object.hasOwn(version, field) ? version[field] : metadata[field];
    if (value !== undefined) settings[field] = value;
  }
  return settings;
}

function fingerprint(value) {
  return createHash("sha256").update(stableJson(value)).digest("hex");
}

function stableJson(value) {
  if (Array.isArray(value)) return `[${value.map(stableJson).join(",")}]`;
  if (value && typeof value === "object") {
    return `{${Object.keys(value).sort().map((key) => `${JSON.stringify(key)}:${stableJson(value[key])}`).join(",")}}`;
  }
  return JSON.stringify(value);
}

export async function assertRouteHealth(routes, fetchImpl = fetch) {
  for (const route of routes) {
    const response = await fetchImpl(`https://${route}/`, {
      method: "POST",
      redirect: "manual",
      signal: AbortSignal.timeout(15_000),
    });
    if (response.status !== 401) throw new Error(`safe unauthenticated health probe failed for an existing route (HTTP ${response.status})`);
    const body = await response.json().catch(() => null);
    if (body?.error !== "unauthorized") throw new Error("health probe returned an unexpected response body");
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) await main();
