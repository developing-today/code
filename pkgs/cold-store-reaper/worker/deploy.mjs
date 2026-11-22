// Cloudflare `cf` deployment for cold-store-rehydrator.
//
// Upload and promotion are deliberately separate operations:
//   deploy.mjs                 build + upload an immutable version only
//   deploy.mjs --deploy-version <id>   promote an already staged version
//
// The upload creates an immutable version only; it never routes traffic.
// DO Workers do not receive Version URLs, so no public preview is attempted.
// Promotion also checks the exact serving baseline and existing secret
// bindings (#3475).
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const WORKER = "cold-store-rehydrator";
const WORKER_URL = "https://cold-store-rehydrator.developing-today.workers.dev";
const REQUIRED_SECRETS = [
  "REHYDRATOR_API_KEY",
  "GOOGLE_CLIENT_ID",
  "GOOGLE_CLIENT_SECRET",
  "GOOGLE_REFRESH_TOKEN",
];

function usageError(message) {
  console.error(`${message}\nusage: deploy.mjs [--dry-run] | deploy.mjs --deploy-version <id> --expected-baseline <sha256>`);
  process.exit(2);
}

// Parse the complete CLI contract before any subprocess, network request,
// or filesystem write (#3352).
let dryRun = false;
let deployVersion = null;
let expectedBaseline = null;
const args = process.argv.slice(2);
for (let i = 0; i < args.length; i += 1) {
  const arg = args[i];
  if (arg === "--dry-run") {
    if (dryRun) usageError("duplicate argument: --dry-run");
    dryRun = true;
  } else if (arg === "--deploy-version") {
    if (deployVersion !== null) usageError("duplicate argument: --deploy-version");
    const value = args[i + 1];
    if (!value || value.startsWith("-")) usageError("--deploy-version needs a version id");
    deployVersion = value;
    i += 1;
  } else if (arg === "--expected-baseline") {
    if (expectedBaseline !== null) usageError("duplicate argument: --expected-baseline");
    const value = args[i + 1];
    if (!value || !/^[a-f0-9]{64}$/.test(value)) usageError("--expected-baseline needs a sha256 fingerprint");
    expectedBaseline = value;
    i += 1;
  } else {
    usageError(`unrecognized argument: ${arg}`);
  }
}
if (dryRun && deployVersion !== null) {
  usageError("--dry-run cannot be combined with --deploy-version");
}
if ((deployVersion === null) !== (expectedBaseline === null)) {
  usageError("--deploy-version and --expected-baseline must be supplied together");
}
if (process.env.CSR_CHECK_ARGS_ONLY === "1") process.exit(0);

if (deployVersion !== null) {
  if (process.env.GITHUB_ACTIONS !== "true" || process.env.COLD_STORE_REHYDRATOR_PRIVATE_STAGING_VERIFIED !== "true") {
    throw new Error("promotion is restricted to the trusted GitHub workflow after isolated private staging evidence");
  }
  if (!/^[0-9]+$/.test(process.env.COLD_STORE_REHYDRATOR_UPLOAD_RUN_ID ?? "")) {
    throw new Error("promotion requires the validated upload run id");
  }
  if (!/^[0-9a-f]{40}$/.test(process.env.COLD_STORE_REHYDRATOR_WORKER_SHA ?? "")) {
    throw new Error("promotion requires the validated uploaded Worker SHA");
  }
}

const accountId = requireEnv("CLOUDFLARE_ACCOUNT_ID");
const apiToken = resolveApiToken();
const env = { ...process.env, CLOUDFLARE_API_TOKEN: apiToken, CLOUDFLARE_ACCOUNT_ID: accountId };

const stage = mkdtempSync(join(tmpdir(), "cold-store-rehydrator-deploy-"));
mkdirSync(join(stage, "dist"));
mkdirSync(join(stage, "node_modules"));
copyFileSync(join(here, "cloudflare.config.ts"), join(stage, "cloudflare.config.ts"));
const bundle = process.env.COLD_STORE_REHYDRATOR_BUNDLE ?? join(here, "dist", "worker.js");
if (!existsSync(bundle)) {
  throw new Error(`Worker bundle not found at ${bundle}; build .#coldStoreRehydrator first`);
}
copyFileSync(bundle, join(stage, "dist", "worker.js"));
for (const [name, variable] of [["cf", "CSR_CF_MODULE"], ["wrangler", "CSR_WRANGLER_MODULE"]]) {
  const target = process.env[variable] ?? join(here, "node_modules", name);
  if (!existsSync(target)) throw new Error(`${variable} is not set and ${target} does not exist; run via nix run`);
  symlinkSync(target, join(stage, "node_modules", name));
}
// cf's build bridge requires Wrangler to be declared in the project
// manifest. Use the version in the Nix-provided module rather than
// resolving or installing a floating npm dependency during deployment.
const packageManifest = JSON.parse(readFileSync(join(here, "package.json"), "utf8"));
const wranglerManifest = JSON.parse(readFileSync(join(stage, "node_modules", "wrangler", "package.json"), "utf8"));
packageManifest.devDependencies = { ...packageManifest.devDependencies, wrangler: wranglerManifest.version };
writeFileSync(join(stage, "package.json"), `${JSON.stringify(packageManifest, null, 2)}\n`);

function runCf(cfArgs, { capture = false } = {}) {
  return execFileSync("cf", ["-q", ...cfArgs], {
    cwd: stage,
    env,
    encoding: "utf8",
    stdio: ["inherit", capture ? "pipe" : "inherit", "inherit"],
  });
}

function parseJsonOutput(output, label) {
  const text = String(output).trim();
  try {
    return JSON.parse(text);
  } catch {
    for (const opening of ["{", "["]) {
      for (let index = text.indexOf(opening); index >= 0; index = text.indexOf(opening, index + 1)) {
        try {
          return JSON.parse(text.slice(index));
        } catch {
          // Ignore progress text and try the next JSON boundary.
        }
      }
    }
    throw new Error(`unexpected ${label} output from cf: ${text.slice(0, 300)}`);
  }
}

function resultItems(value, label) {
  for (const candidate of [value, value?.result, value?.result?.items, value?.items, value?.[label]]) {
    if (Array.isArray(candidate)) return candidate;
  }
  throw new Error(`unexpected ${label} response from cf`);
}

async function getVersion(versionId) {
  const url = `https://api.cloudflare.com/client/v4/accounts/${accountId}/workers/scripts/${WORKER}/versions/${encodeURIComponent(versionId)}`;
  const response = await fetch(url, { headers: { authorization: `Bearer ${apiToken}` } });
  const body = await response.json();
  if (!response.ok || body.success !== true) {
    throw new Error(`could not inspect Worker version ${versionId} (HTTP ${response.status})`);
  }
  return body.result;
}

async function verifyVersionConfigured(versionId) {
  const version = await getVersion(versionId);
  const bindings = version?.resources?.bindings ?? version?.bindings ?? [];
  const secretNames = new Set(bindings.filter((binding) => binding?.type === "secret_text").map((binding) => binding.name));
  const missing = REQUIRED_SECRETS.filter((name) => !secretNames.has(name));
  if (missing.length) {
    throw new Error(
      `version ${versionId} lacks required secret bindings (${missing.join(", ")}); refusing to promote it. ` +
        "Upload and stage a configured version before promotion.",
    );
  }
}

function createDeployment(versionId) {
  const versions = JSON.stringify([{ version_id: versionId, percentage: 100 }]);
  runCf(["workers", "deployments", "create", "--worker", WORKER, "--strategy", "percentage", "--versions", versions]);
}

function versionList() {
  const response = parseJsonOutput(runCf(["workers", "versions", "list", "--worker", WORKER], { capture: true }), "versions list");
  return resultItems(response, "versions list");
}

function versionByTag(tag) {
  const versions = versionList()
    .filter((version) => (version.annotations?.["workers/tag"] ?? version.metadata?.annotations?.["workers/tag"]) === tag)
    .sort((left, right) => createdOn(right) - createdOn(left));
  return versions[0] ?? null;
}

function assertVersionSource(versionId, workerSha) {
  const version = versionList().find((candidate) => candidate.id === versionId);
  const tag = version?.annotations?.["workers/tag"] ?? version?.metadata?.annotations?.["workers/tag"];
  const expectedTag = `git-${workerSha.slice(0, 12)}`;
  if (tag !== expectedTag) {
    throw new Error(`version ${versionId} is not tagged for uploaded source ${workerSha}; refusing to promote`);
  }
}

function createdOn(version) {
  const raw = version?.metadata?.created_on ?? version?.created_on ?? version?.createdOn ?? 0;
  const parsed = typeof raw === "number" ? raw : Date.parse(raw);
  return Number.isFinite(parsed) ? parsed : 0;
}

function latestDeployment() {
  const response = parseJsonOutput(runCf(["workers", "deployments", "list", "--worker", WORKER], { capture: true }), "deployments list");
  // cf documents the first item as the latest deployment serving traffic.
  const deployments = resultItems(response, "deployments");
  if (!deployments[0]?.id || !Array.isArray(deployments[0]?.versions)) {
    throw new Error("could not determine current serving deployment; refusing to continue");
  }
  const total = deployments[0].versions.reduce((sum, version) => {
    const percentage = Number(version?.percentage);
    if (typeof version?.version_id !== "string" || !Number.isFinite(percentage) || percentage < 0 || percentage > 100) {
      throw new Error("current deployment has malformed version traffic; refusing to continue");
    }
    return sum + percentage;
  }, 0);
  if (deployments[0].versions.length === 0 || total !== 100) {
    throw new Error("current deployment does not describe 100% of traffic; refusing to continue");
  }
  return deployments[0];
}

function servingBaseline() {
  const deployment = latestDeployment();
  const versions = deployment.versions
    .map(({ version_id, percentage }) => ({ version_id, percentage: Number(percentage) }))
    .sort((left, right) => left.version_id.localeCompare(right.version_id));
  return { deployment_id: deployment.id, versions };
}

function baselineFingerprint(baseline = servingBaseline()) {
  return createHash("sha256").update(JSON.stringify(baseline)).digest("hex");
}

function assertServingBaseline(expected) {
  const actual = baselineFingerprint();
  if (actual !== expected) {
    throw new Error(`serving baseline changed (expected ${expected}, found ${actual}); refusing to promote`);
  }
}

function verifyDeployment(versionId) {
  const active = (latestDeployment().versions ?? []).filter((version) => Number(version.percentage) > 0);
  if (active.length !== 1 || active[0].version_id !== versionId || Number(active[0].percentage) !== 100) {
    const serving = active.length ? active.map((version) => `${version.version_id}@${version.percentage}%`).join(", ") : "no active version";
    throw new Error(`post-promotion check failed: expected ${versionId} at 100%; latest deployment serves ${serving}`);
  }
}

function writeWorkflowOutputs(values) {
  if (!process.env.GITHUB_OUTPUT) return;
  const lines = Object.entries(values).map(([key, value]) => `${key}=${value}`);
  writeFileSync(process.env.GITHUB_OUTPUT, `${lines.join("\n")}\n`, { flag: "a" });
}

if (deployVersion !== null) {
  assertServingBaseline(expectedBaseline);
  assertVersionSource(deployVersion, process.env.COLD_STORE_REHYDRATOR_WORKER_SHA);
  await verifyVersionConfigured(deployVersion);
  // Recheck after the read-only version checks, immediately before traffic changes.
  assertServingBaseline(expectedBaseline);
  createDeployment(deployVersion);
  verifyDeployment(deployVersion);
  console.log(`live: ${WORKER_URL} at version ${deployVersion}`);
  process.exit(0);
}

const sha = execFileSync("git", ["rev-parse", "HEAD"], { cwd: here, encoding: "utf8" }).trim();
const tag = `git-${sha.slice(0, 12)}`;
const message = `stage ${sha} via GitHub Actions / cf Workers Versions API`;

// `cf build` writes Build Output from the same typed settings used for
// upload. The following versions-create request has no traffic side effect.
runCf(["build"]);
const baseline = servingBaseline();
const baselineSha256 = baselineFingerprint(baseline);
const uploadArgs = [
  "workers", "versions", "create",
  "--prebuilt",
  "--worker", WORKER,
  "--tag", tag,
  "--message", message,
];
if (dryRun) {
  runCf([...uploadArgs, "--dry-run"]);
  console.log("dry run: Worker bundle built; nothing uploaded and no production traffic changed");
  process.exit(0);
}

runCf(uploadArgs);
assertServingBaseline(baselineSha256);
const uploaded = versionByTag(tag);
if (!uploaded?.id) throw new Error(`uploaded version with tag ${tag} was not found`);
await verifyVersionConfigured(uploaded.id);
console.log(`uploaded version ${uploaded.id} (${tag})`);
console.log(`serving baseline: ${baselineSha256}`);
console.log("production traffic unchanged; this Durable Object Worker has no Version URL; do not promote without independent private staging evidence");
writeWorkflowOutputs({ version_id: uploaded.id, worker_sha: sha, baseline_sha256: baselineSha256 });

function requireEnv(name) {
  if (!process.env[name]) throw new Error(`${name} is not set`);
  return process.env[name];
}

function resolveApiToken() {
  if (process.env.GITHUB_ACTIONS === "true") {
    return requireEnv("COLD_STORE_REHYDRATOR_CF_API_TOKEN");
  }
  if (process.env.CLOUDFLARE_API_TOKEN) return process.env.CLOUDFLARE_API_TOKEN;
  const secretName = process.env.CSR_CF_TOKEN_SECRET ?? "cloudflare.api-token-day-to-day";
  const secrets = JSON.parse(execFileSync("bws", ["secret", "list", "-o", "json"], { encoding: "utf8" }));
  const token = secrets.find((secret) => secret.key === secretName)?.value;
  if (!token) throw new Error(`no Bitwarden secret named ${secretName}`);
  return token;
}
