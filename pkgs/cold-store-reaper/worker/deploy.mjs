// Cloudflare `cf` deployment for cold-store-rehydrator.
//
// Upload and promotion are deliberately separate operations:
//   deploy.mjs                 build + upload an immutable version only
//   deploy.mjs --deploy-version <id>   promote an already staged version
//
// The upload gets a unique preview alias and never changes production
// traffic. Promotion is allowed only for versions carrying the existing
// secret bindings, preserving the #3475 rollback guard.
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
  console.error(`${message}\nusage: deploy.mjs [--dry-run] | deploy.mjs --deploy-version <id>`);
  process.exit(2);
}

// Parse the complete CLI contract before any subprocess, network request,
// or filesystem write (#3352).
let dryRun = false;
let deployVersion = null;
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
  } else {
    usageError(`unrecognized argument: ${arg}`);
  }
}
if (dryRun && deployVersion !== null) {
  usageError("--dry-run cannot be combined with --deploy-version");
}
if (process.env.CSR_CHECK_ARGS_ONLY === "1") process.exit(0);

const accountId = requireEnv("CLOUDFLARE_ACCOUNT_ID");
const apiToken = requireEnv("CLOUDFLARE_API_TOKEN");
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

function createdOn(version) {
  const raw = version?.metadata?.created_on ?? version?.created_on ?? version?.createdOn ?? 0;
  const parsed = typeof raw === "number" ? raw : Date.parse(raw);
  return Number.isFinite(parsed) ? parsed : 0;
}

function verifyDeployment(versionId) {
  const response = parseJsonOutput(runCf(["workers", "deployments", "list", "--worker", WORKER], { capture: true }), "deployments list");
  const deployments = resultItems(response, "deployments").sort((left, right) => createdOn(right) - createdOn(left));
  const active = (deployments[0]?.versions ?? []).filter((version) => Number(version.percentage) > 0);
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
  await verifyVersionConfigured(deployVersion);
  createDeployment(deployVersion);
  verifyDeployment(deployVersion);
  console.log(`live: ${WORKER_URL} at version ${deployVersion}`);
  process.exit(0);
}

const sha = execFileSync("git", ["rev-parse", "HEAD"], { cwd: here, encoding: "utf8" }).trim();
const tag = `git-${sha.slice(0, 12)}`;
const previewAlias = `ci-${sha.slice(0, 8)}`;
const previewUrl = `https://${previewAlias}-${WORKER}.developing-today.workers.dev`;
const message = `stage ${sha} via GitHub Actions / cf Workers Versions API`;

// `cf build` writes Build Output from the same typed settings used for
// upload. The following versions-create request has no traffic side effect.
runCf(["build"]);
const uploadArgs = [
  "workers", "versions", "create",
  "--prebuilt",
  "--worker", WORKER,
  "--tag", tag,
  "--message", message,
  "--preview-alias", previewAlias,
];
if (dryRun) {
  runCf([...uploadArgs, "--dry-run"]);
  console.log("dry run: Worker bundle built; nothing uploaded and no production traffic changed");
  process.exit(0);
}

runCf(uploadArgs);
const uploaded = versionByTag(tag);
if (!uploaded?.id) throw new Error(`uploaded version with tag ${tag} was not found`);
await verifyVersionConfigured(uploaded.id);
console.log(`uploaded version ${uploaded.id} (${tag})`);
console.log(`preview: ${previewUrl}`);
console.log("production traffic unchanged; promote only after staged parity verification");
writeWorkflowOutputs({ version_id: uploaded.id, worker_sha: sha, preview_url: previewUrl });

function requireEnv(name) {
  if (!process.env[name]) throw new Error(`${name} is not set`);
  return process.env[name];
}
